//! `super-pm-server`: the team server.
//!
//! ```text
//! super-pm-server [--data DIR] [--listen ADDRESS:PORT] [--public-url URL] [--open-signup]
//! ```
//! Defaults: data in `./super-pm-data`, listening on `127.0.0.1:7878`. The same settings can come
//! from `SUPER_PM_DATA`, `SUPER_PM_LISTEN`, `SUPER_PM_PUBLIC_URL` and `SUPER_PM_OPEN_SIGNUP=1`.
//! `--public-url` is the address people use (it goes into invites); `--open-signup` lets anyone
//! create an account, as an online service would. Put it behind a reverse proxy with HTTPS (Caddy,
//! nginx) before people connect over a network.

use std::path::PathBuf;

use pm_server::storage::SqliteStorage;
use pm_server::{router, AppState, Config};

fn setting(args: &[String], flag: &str, env: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var(env).ok())
        .unwrap_or_else(|| default.to_owned())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("super-pm-server [--data DIR] [--listen ADDRESS:PORT] [--public-url URL] [--open-signup]");
        return;
    }
    let data = PathBuf::from(setting(&args, "--data", "SUPER_PM_DATA", "super-pm-data"));
    let listen = setting(&args, "--listen", "SUPER_PM_LISTEN", "127.0.0.1:7878");
    let public_url = Some(setting(&args, "--public-url", "SUPER_PM_PUBLIC_URL", "")).filter(|u| !u.is_empty()).map(|u| u.trim_end_matches('/').to_owned());
    let open_signup = args.iter().any(|a| a == "--open-signup") || std::env::var("SUPER_PM_OPEN_SIGNUP").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if let Err(e) = std::fs::create_dir_all(&data) {
        eprintln!("Cannot create the data folder {}: {e}", data.display());
        std::process::exit(1);
    }
    let storage = match SqliteStorage::open(&data.join("server.sqlite")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot open the database in {}: {e}", data.display());
            std::process::exit(1);
        }
    };
    let state = AppState::with_config(storage, Config { public_url, open_signup });
    // Weekly summaries and other timed rules.
    pm_server::rules::start(state.clone());
    let app = router(state);
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Cannot listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    println!("Super PM server listening on http://{listen} (data in {})", data.display());
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        println!("Stopping.");
    };
    if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(shutdown).await {
        eprintln!("The server stopped: {e}");
        std::process::exit(1);
    }
}
