use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, Orientation};

use custerm_core::config::CustermConfig;

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

        let container = GtkBox::new(Orientation::Vertical, 0);

        let terminal = TerminalTab::new(config);
        container.append(terminal.widget());

        window.set_child(Some(&container));

        // Apply CSS for Catppuccin Mocha background
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("window { background-color: #1e1e2e; }");
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
