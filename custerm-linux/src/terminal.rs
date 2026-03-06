use gtk4::prelude::*;
use vte4::prelude::*;
use vte4::Terminal;

use custerm_core::config::CustermConfig;

// Catppuccin Mocha palette
const PALETTE: &[&str] = &[
    "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af",
    "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de",
    "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af",
    "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8",
];

pub struct TerminalTab {
    terminal: Terminal,
}

impl TerminalTab {
    pub fn new(config: &CustermConfig) -> Self {
        let terminal = Terminal::new();

        // Font
        let font_desc = gtk4::pango::FontDescription::from_string(
            &format!("{} {}", config.terminal.font_family, config.terminal.font_size),
        );
        terminal.set_font(Some(&font_desc));

        // Colors - Catppuccin Mocha
        let fg = parse_color("#cdd6f4");
        let bg = parse_color("#1e1e2e");
        let palette: Vec<gtk4::gdk::RGBA> = PALETTE.iter()
            .map(|c| parse_color(c))
            .collect();
        let palette_refs: Vec<&gtk4::gdk::RGBA> = palette.iter().collect();
        terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);

        // Cursor
        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::On);
        terminal.set_cursor_shape(vte4::CursorShape::Block);

        // Scrollback
        terminal.set_scrollback_lines(10000);

        // Spawn shell
        let shell = config.terminal.shell.clone();
        terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            None::<&str>,                  // working directory (inherit)
            &[&shell],                     // argv
            &[] as &[&str],               // envv (inherit)
            gtk4::glib::SpawnFlags::DEFAULT,
            || {},                         // child_setup
            -1,                            // timeout
            gtk4::gio::Cancellable::NONE,
            |_result| {},                  // callback
        );

        // Handle child exit
        terminal.connect_child_exited(|terminal, _status| {
            if let Some(toplevel) = terminal.root() {
                if let Some(window) = toplevel.downcast_ref::<gtk4::Window>() {
                    window.close();
                }
            }
        });

        // Make terminal expand to fill space
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        Self { terminal }
    }

    pub fn widget(&self) -> &Terminal {
        &self.terminal
    }
}

fn parse_color(hex: &str) -> gtk4::gdk::RGBA {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    gtk4::gdk::RGBA::new(r, g, b, 1.0)
}
