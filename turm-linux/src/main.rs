mod app;
mod cef_init;
mod cef_panel;
mod cef_plugin_panel;
mod dbus;
mod panel;
mod search;

mod socket;
mod split;
mod tabs;
mod terminal;
mod window;

fn main() {
    // CEF sub-process check — must happen before anything else.
    // When CEF spawns renderer/GPU processes, it re-launches this binary
    // with --type=renderer etc. We detect that and handle it immediately.
    if cef_init::handle_subprocess() {
        return;
    }

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("turm {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--init-config") {
        match turm_core::config::TurmConfig::write_default() {
            Ok(path) => {
                println!("Config written to: {}", path.display());
                return;
            }
            Err(e) => {
                eprintln!("Failed to write config: {e}");
                std::process::exit(1);
            }
        }
    }

    if args.iter().any(|a| a == "--config-path") {
        println!("{}", turm_core::config::TurmConfig::config_path().display());
        return;
    }

    // Initialize CEF for the main browser process
    let _cef = cef_init::initialize();

    app::run();

    // Shutdown CEF after GTK exits
    cef_init::shutdown();
}
