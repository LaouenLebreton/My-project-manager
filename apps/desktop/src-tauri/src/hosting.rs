//! The app hosting a team itself (iteration 3): the team server runs inside the app, so
//! creating a team needs no server address, account form or password. People on the same
//! network join with an invite. A team can be connected to a real server later.
//!
//! In settings and project files this built-in server is called `local`; its actual address
//! (the port can differ between launches) is looked up when a request is made.

use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager};

use crate::ipc::IpcError;

/// The server name stored for teams this app hosts.
pub const LOCAL: &str = "local";
const PREFERRED_PORT: u16 = 7878;

fn port_cell() -> &'static Mutex<Option<u16>> {
    static PORT: OnceLock<Mutex<Option<u16>>> = OnceLock::new();
    PORT.get_or_init(Default::default)
}

/// Separate settings per test profile (`SUPER_PM_PROFILE`), so two copies of the app on one
/// Mac can act as two people.
pub fn profiled(dir: PathBuf) -> PathBuf {
    match std::env::var("SUPER_PM_PROFILE") {
        Ok(p) if !p.trim().is_empty() => dir.join("Profiles").join(p.trim()),
        _ => dir,
    }
}

/// The address to send requests to for a stored server name.
pub fn base(server: &str) -> String {
    if server == LOCAL {
        let port = port_cell().lock().expect("lock").unwrap_or(PREFERRED_PORT);
        format!("http://127.0.0.1:{port}")
    } else {
        server.to_owned()
    }
}

/// How to name a server to the user.
pub fn display(server: &str) -> String {
    if server == LOCAL { "this Mac".to_owned() } else { server.to_owned() }
}

/// This Mac's address on the local network, for invites. No packet is sent: connecting a UDP
/// socket only asks the system which interface it would use.
fn lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

/// The address other people on the network use to reach the team hosted here.
pub fn invite_address() -> String {
    let port = port_cell().lock().expect("lock").unwrap_or(PREFERRED_PORT);
    format!("http://{}:{port}", lan_ip().unwrap_or_else(|| "127.0.0.1".into()))
}

/// This Mac has hosted teams (even if they moved: it keeps telling people where they went).
pub fn hosts_teams(app: &AppHandle) -> bool {
    app.path().app_data_dir().is_ok_and(|d| profiled(d).join("TeamServer").join("server.sqlite").exists())
}

/// The running server, kept so it can also be offered on the local network.
fn state_cell() -> &'static Mutex<Option<pm_server::AppState>> {
    static STATE: OnceLock<Mutex<Option<pm_server::AppState>>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// The local-network address the server is also offered on, once it is.
fn shared_on() -> &'static Mutex<Option<String>> {
    static SHARED: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SHARED.get_or_init(Default::default)
}

fn server_dir(app: &AppHandle) -> Option<PathBuf> {
    Some(profiled(app.path().app_data_dir().ok()?).join("TeamServer"))
}

/// Present once this Mac's team has been opened to the local network (people were invited).
fn share_file(app: &AppHandle) -> Option<PathBuf> {
    server_dir(app).map(|d| d.join("share-on-network"))
}

/// Starts the built-in team server if it is not running yet. It listens on this Mac only
/// (127.0.0.1); it is offered on the local network only once people are invited
/// (`share_on_network`), and then on every launch.
pub async fn ensure(app: &AppHandle) -> Result<(), IpcError> {
    if port_cell().lock().expect("lock").is_some() {
        return Ok(());
    }
    let fail = |what: &str, e: &dyn std::fmt::Display| IpcError { message: format!("the team could not be started on this Mac ({what}: {e})") };
    let dir = server_dir(app).ok_or_else(|| fail("data folder", &"unknown"))?;
    std::fs::create_dir_all(&dir).map_err(|e| fail("data folder", &e))?;
    let db_file = dir.join("server.sqlite");
    // Teams hosted before this rule existed already had their people on the network: keep it so.
    if db_file.exists() && !dir.join("share-decided").exists() {
        let _ = std::fs::write(dir.join("share-on-network"), b"");
    }
    let _ = std::fs::write(dir.join("share-decided"), b"");
    let storage = pm_server::storage::SqliteStorage::open(&db_file).map_err(|e| fail("database", &e))?;
    // The usual port if it is free, otherwise any free one.
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PREFERRED_PORT)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| fail("network", &e))?,
    };
    let port = listener.local_addr().map_err(|e| fail("network", &e))?.port();
    let state = pm_server::AppState::new(storage);
    {
        let mut cell = port_cell().lock().expect("lock");
        if cell.is_some() {
            return Ok(()); // another call started it meanwhile
        }
        *cell = Some(port);
        *state_cell().lock().expect("lock") = Some(state.clone());
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = pm_server::serve(listener, state).await {
            eprintln!("the built-in team server stopped: {e}");
            *port_cell().lock().expect("lock") = None;
            *state_cell().lock().expect("lock") = None;
        }
    });
    if share_file(app).is_some_and(|f| f.exists()) {
        share_on_network(app).await;
    }
    Ok(())
}

/// Offers the team hosted here to people on the same network (they join with an invite). Done
/// when people are invited; remembered for later launches.
pub async fn share_on_network(app: &AppHandle) {
    if let Some(f) = share_file(app) {
        let _ = std::fs::write(f, b"");
    }
    let (Some(port), Some(state)) = (*port_cell().lock().expect("lock"), state_cell().lock().expect("lock").clone()) else { return };
    let Some(ip) = lan_ip() else { return };
    if shared_on().lock().expect("lock").as_deref() == Some(ip.as_str()) {
        return;
    }
    match tokio::net::TcpListener::bind((ip.as_str(), port)).await {
        Ok(listener) => {
            *shared_on().lock().expect("lock") = Some(ip);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = pm_server::serve_also(listener, state).await {
                    eprintln!("the team stopped being offered on the network: {e}");
                    *shared_on().lock().expect("lock") = None;
                }
            });
        }
        Err(e) => eprintln!("the team could not be offered on the network ({ip}:{port}): {e}"),
    }
}
