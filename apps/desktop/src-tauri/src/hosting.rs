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

/// Starts the built-in team server if it is not running yet.
pub async fn ensure(app: &AppHandle) -> Result<(), IpcError> {
    if port_cell().lock().expect("lock").is_some() {
        return Ok(());
    }
    let fail = |what: &str, e: &dyn std::fmt::Display| IpcError { message: format!("the team could not be started on this Mac ({what}: {e})") };
    let dir = profiled(app.path().app_data_dir().map_err(|e| fail("data folder", &e))?).join("TeamServer");
    std::fs::create_dir_all(&dir).map_err(|e| fail("data folder", &e))?;
    let storage = pm_server::storage::SqliteStorage::open(&dir.join("server.sqlite")).map_err(|e| fail("database", &e))?;
    // The usual port if it is free, otherwise any free one.
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", PREFERRED_PORT)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("0.0.0.0", 0)).await.map_err(|e| fail("network", &e))?,
    };
    let port = listener.local_addr().map_err(|e| fail("network", &e))?.port();
    {
        let mut cell = port_cell().lock().expect("lock");
        if cell.is_some() {
            return Ok(()); // another call started it meanwhile
        }
        *cell = Some(port);
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = pm_server::serve(listener, pm_server::AppState::new(storage)).await {
            eprintln!("the built-in team server stopped: {e}");
            *port_cell().lock().expect("lock") = None;
        }
    });
    Ok(())
}
