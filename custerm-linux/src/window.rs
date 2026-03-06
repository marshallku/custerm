use std::path::PathBuf;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};
use gtk4::glib;

use custerm_core::config::CustermConfig;

use crate::dbus::{self, DbusCommand};
use crate::terminal::TerminalTab;

pub struct CustermWindow {
    pub window: ApplicationWindow,
}

impl CustermWindow {
    pub fn new(app: &Application, config: &CustermConfig) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("custerm")
            .default_width(1200)
            .default_height(800)
            .build();

        let terminal = TerminalTab::new(config);
        window.set_child(Some(terminal.widget()));

        // Apply CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("window { background-color: #1e1e2e; }");
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Apply initial background from config
        if let Some(path) = config.background.image.as_ref().map(PathBuf::from) {
            if path.exists() {
                eprintln!("[custerm] applying background: {}", path.display());
                terminal.set_background(&path);
            } else {
                eprintln!("[custerm] configured image not found: {}", path.display());
            }
        }

        // Register D-Bus and poll for commands on main thread
        let rx = dbus::register();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    DbusCommand::SetBackground(path) => {
                        terminal.set_background(std::path::Path::new(&path));
                    }
                    DbusCommand::ClearBackground => {
                        terminal.clear_background();
                    }
                    DbusCommand::SetTint(opacity) => {
                        terminal.set_tint(opacity);
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
