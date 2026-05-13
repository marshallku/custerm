//! `nesttyd` binary entry.
//!
//! v0: prepare + bind well-known socket, run accept loop, ping responder.
//! No supervisor, no trigger engine, no plugin host yet — those fold in
//! over the next commits per `docs/harness-integration.md` § Migration path.

use std::path::PathBuf;
use std::process::ExitCode;

use nestty_core::paths;
use nestty_daemon::socket::{self, SocketPrep};

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let socket_path: PathBuf = paths::socket_path();
    log::info!("nesttyd starting; socket={}", socket_path.display());

    match socket::prepare_socket_path(&socket_path) {
        SocketPrep::Fresh => log::debug!("socket path fresh"),
        SocketPrep::StaleCleared => log::info!("removed stale socket file"),
        SocketPrep::InUse => {
            log::error!(
                "socket {} already bound by another nesttyd; refusing to start",
                socket_path.display()
            );
            return ExitCode::from(2);
        }
        SocketPrep::Error(msg) => {
            log::error!("socket prep failed: {msg}");
            return ExitCode::from(1);
        }
        SocketPrep::NotSocket => {
            log::error!(
                "path {} exists but is not a Unix socket; refusing to unlink (set NESTTY_SOCKET to a fresh path)",
                socket_path.display()
            );
            return ExitCode::from(3);
        }
    }

    let listener = match socket::bind_listener(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("bind({}): {e}", socket_path.display());
            return ExitCode::from(1);
        }
    };

    log::info!("nesttyd listening on {}", socket_path.display());
    socket::run_accept_loop(listener);

    // run_accept_loop only returns on fatal listener error. Best-effort
    // cleanup; if we exited on SIGTERM/SIGINT/SIGKILL the socket file is
    // still on disk and `prepare_socket_path`'s stale detection clears
    // it on the next start.
    //
    // No async-signal handler is installed: doing socket cleanup from a
    // signal context requires async-signal-safe primitives (which std::fs
    // and std::sync::Mutex are not). Cooperative shutdown via a self-pipe
    // wake-up is a follow-up; the stale-detection path makes this
    // acceptable in the interim.
    socket::cleanup_socket(&socket_path);
    log::info!("nesttyd shut down");
    ExitCode::SUCCESS
}
