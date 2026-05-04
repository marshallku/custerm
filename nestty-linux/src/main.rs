mod app;
mod background;
mod panel;
mod plugin_panel;
mod search;
mod service_supervisor;

mod socket;
mod split;
mod statusbar;
mod tabs;
mod terminal;
mod trigger_sink;
mod webview;
mod window;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("nestty {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--init-config") {
        match nestty_core::config::NesttyConfig::write_default() {
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
        println!(
            "{}",
            nestty_core::config::NesttyConfig::config_path().display()
        );
        return;
    }

    app::run();
}
