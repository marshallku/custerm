//! `nesttyd` binary entry.
//!
//! Hosts the daemon-side `ActionRegistry` (`system.ping`, `system.log`,
//! `daemon.info`) and — when `NESTTYD_HOST_PLUGINS=1` — a
//! `ServiceSupervisor` that activates discovered plugins. The flag is
//! transitional: nestty-linux's GUI window still hosts its own supervisor,
//! so unconditional plugin hosting would double-spawn. Removed when the
//! GUI becomes a socket client (migration step 4–5).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use nestty_core::action_registry::{ActionRegistry, internal_error};
use nestty_core::paths;
use nestty_daemon::service_supervisor::ServiceSupervisor;
use nestty_daemon::socket::{
    self, DaemonState, LEGACY_DISPATCH_METHODS, SocketPrep, new_event_bus,
};
use nestty_daemon::trigger_sink::TRIGGER_ONLY_RESERVED_METHODS;
use serde_json::json;

const ENV_HOST_PLUGINS: &str = "NESTTYD_HOST_PLUGINS";

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

    let event_bus = new_event_bus();
    let actions = Arc::new(ActionRegistry::with_completion_bus(event_bus.clone()));
    register_builtins(&actions);

    // Bind before activating plugins so a bind failure can't orphan
    // eagerly-spawned children.
    let listener = match socket::bind_listener(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("bind({}): {e}", socket_path.display());
            return ExitCode::from(1);
        }
    };

    let supervisor_guard: Option<Arc<ServiceSupervisor>> = if env_flag_enabled(ENV_HOST_PLUGINS) {
        Some(activate_supervisor(&actions, &event_bus))
    } else {
        log::info!(
            "plugin host disabled (set {ENV_HOST_PLUGINS}=1 to activate plugins from this daemon)"
        );
        None
    };

    let state = DaemonState::new(actions);

    log::info!("nesttyd listening on {}", socket_path.display());
    socket::run_accept_loop(listener, state);

    // Arc::drop does not call shutdown_all; we must invoke it explicitly
    // for cooperative plugin shutdown before unlinking the socket.
    if let Some(sup) = supervisor_guard.as_ref() {
        log::info!("shutting down supervised plugins");
        sup.shutdown_all();
    }

    socket::cleanup_socket(&socket_path);
    log::info!("nesttyd shut down");
    ExitCode::SUCCESS
}

fn register_builtins(actions: &Arc<ActionRegistry>) {
    actions.register_silent("system.ping", |_| Ok(json!({ "status": "ok" })));
    actions.register("system.log", |params| {
        let msg = params
            .get("message")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| params.to_string());
        eprintln!("[system.log] {msg}");
        Ok(json!({}))
    });
    actions.register_silent("daemon.info", |_| {
        serde_json::to_value(serde_json::json!({
            "daemon": "nesttyd",
            "version": env!("CARGO_PKG_VERSION"),
            "host_plugins": env_flag_enabled(ENV_HOST_PLUGINS),
        }))
        .map_err(|e| internal_error(format!("daemon.info serialization failed: {e}")))
    });
}

/// Accepts `1`, `true`, `yes` (case-insensitive). Everything else,
/// including `0` / `false` / empty / unset, disables.
fn env_flag_enabled(var: &str) -> bool {
    match std::env::var(var) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Err(_) => false,
    }
}

fn activate_supervisor(
    actions: &Arc<ActionRegistry>,
    event_bus: &Arc<nestty_core::event_bus::EventBus>,
) -> Arc<ServiceSupervisor> {
    let plugins = nestty_core::plugin::discover_plugins();
    log::info!(
        "discovered {} plugin manifest(s); spawning onStartup services",
        plugins.len()
    );
    for p in &plugins {
        log::info!(
            "plugin: {} v{}",
            p.manifest.plugin.name,
            p.manifest.plugin.version
        );
    }
    let reserved: Vec<&str> = LEGACY_DISPATCH_METHODS
        .iter()
        .copied()
        .chain(TRIGGER_ONLY_RESERVED_METHODS.iter().copied())
        .collect();
    ServiceSupervisor::new(
        event_bus.clone(),
        actions.clone(),
        &plugins,
        env!("CARGO_PKG_VERSION"),
        &reserved,
    )
}
