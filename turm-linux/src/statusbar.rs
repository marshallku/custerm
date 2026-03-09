use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use webkit6::prelude::*;

use turm_core::config::TurmConfig;
use turm_core::plugin::LoadedPlugin;
use turm_core::theme::Theme;

struct ModuleHandle {
    /// DOM element id in the WebView
    dom_id: String,
    exec: String,
    interval: u64,
    plugin_dir: std::path::PathBuf,
    socket_path: String,
}

pub struct StatusBar {
    pub container: gtk4::Box,
    webview: webkit6::WebView,
    #[allow(dead_code)]
    modules: Rc<RefCell<Vec<ModuleHandle>>>,
}

/// Build the shell HTML with empty module containers.
/// Modules are just <span> elements that get updated via JS.
fn build_bar_html(plugins: &[LoadedPlugin], theme: &Theme, height: u32) -> String {
    let mut left = Vec::new();
    let mut center = Vec::new();
    let mut right = Vec::new();

    for plugin in plugins {
        for module in &plugin.manifest.modules {
            let dom_id = format!("mod-{}-{}", plugin.manifest.plugin.name, module.name);
            let class = module.class.as_deref().unwrap_or("");
            let entry = (
                module.order,
                format!(r#"<span id="{dom_id}" class="turm-module {class}" title="">...</span>"#),
            );
            match module.position.as_str() {
                "left" => left.push(entry),
                "center" => center.push(entry),
                _ => right.push(entry),
            }
        }
    }

    left.sort_by_key(|(o, _)| *o);
    center.sort_by_key(|(o, _)| *o);
    right.sort_by_key(|(o, _)| *o);

    eprintln!(
        "[turm] statusbar modules: left={}, center={}, right={}",
        left.len(),
        center.len(),
        right.len()
    );

    let render = |items: &[(i32, String)]| -> String {
        items
            .iter()
            .map(|(_, html)| html.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Collect plugin style.css files
    let mut plugin_css = String::new();
    for plugin in plugins {
        let css_path = plugin.dir.join("style.css");
        if let Ok(css) = std::fs::read_to_string(&css_path) {
            plugin_css.push_str(&css);
            plugin_css.push('\n');
        }
    }

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<style>
:root {{
    --turm-bg: {bg};
    --turm-fg: {text};
    --turm-surface0: {surface0};
    --turm-surface1: {surface1};
    --turm-surface2: {surface2};
    --turm-overlay0: {overlay0};
    --turm-text: {text};
    --turm-subtext0: {subtext0};
    --turm-subtext1: {subtext1};
    --turm-accent: {accent};
    --turm-red: {red};
}}
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
html, body {{
    height: {height}px;
    overflow: hidden;
    background: {surface0};
    color: {subtext0};
    font-family: system-ui, -apple-system, sans-serif;
    font-size: 12px;
}}
body {{
    display: flex;
    align-items: center;
    border-top: 1px solid {overlay0};
}}
#left, #center, #right {{
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
}}
#left {{ flex: 1 1 0; min-width: 0; justify-content: flex-start; overflow: hidden; }}
#center {{ flex: 0 0 auto; justify-content: center; }}
#right {{ flex: 1 1 0; min-width: 0; justify-content: flex-end; overflow: hidden; }}
.turm-module {{
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
}}
{plugin_css}
</style>
</head>
<body>
<div id="left">{left}</div>
<div id="center">{center}</div>
<div id="right">{right}</div>
</body>
</html>"#,
        bg = theme.background,
        text = theme.text,
        surface0 = theme.surface0,
        surface1 = theme.surface1,
        surface2 = theme.surface2,
        overlay0 = theme.overlay0,
        subtext0 = theme.subtext0,
        subtext1 = theme.subtext1,
        accent = theme.accent,
        red = theme.red,
        height = height,
        left = render(&left),
        center = render(&center),
        right = render(&right),
        plugin_css = plugin_css,
    )
}

/// Parse module script output. Supports:
/// - JSON: {"text": "...", "tooltip": "..."}
/// - Plain text: used as-is
fn parse_output(output: &str) -> (String, Option<String>) {
    let trimmed = output.trim();
    if trimmed.starts_with('{')
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        let text = val["text"].as_str().unwrap_or(trimmed).to_string();
        let tooltip = val["tooltip"].as_str().map(|s| s.to_string());
        return (text, tooltip);
    }
    (trimmed.to_string(), None)
}

/// Run a module's exec command in a thread, send result back via channel.
fn run_module_exec(
    exec: &str,
    plugin_dir: &std::path::Path,
    socket_path: &str,
) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let exec = exec.to_string();
    let dir = plugin_dir.to_path_buf();
    let sock = socket_path.to_string();

    std::thread::spawn(move || {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&exec)
            .current_dir(&dir)
            .env("TURM_SOCKET", &sock)
            .env("TURM_PLUGIN_DIR", &dir)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let _ = tx.send(stdout);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                eprintln!("[turm] statusbar module error: {stderr}");
                let _ = tx.send(String::new());
            }
            Err(e) => {
                eprintln!("[turm] statusbar module exec failed: {e}");
                let _ = tx.send(String::new());
            }
        }
    });

    rx
}

impl StatusBar {
    pub fn new(config: &TurmConfig, plugins: &[LoadedPlugin]) -> Self {
        let theme = Theme::by_name(&config.theme.name).unwrap_or_default();
        let height = config.statusbar.height;
        let socket_path = format!("/tmp/turm-{}.sock", std::process::id());

        let webview = webkit6::WebView::new();

        if let Some(settings) = webkit6::prelude::WebViewExt::settings(&webview) {
            settings.set_enable_javascript(true);
            settings.set_allow_file_access_from_file_urls(true);
            settings.set_allow_universal_access_from_file_urls(false);
            settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Always);
        }

        webview.set_hexpand(true);
        webview.set_vexpand(false);
        webview.set_size_request(-1, height as i32);

        // Build and load the shell HTML
        let html = build_bar_html(plugins, &theme, height);
        webview.load_html(&html, None);

        // Collect module handles
        let modules: Rc<RefCell<Vec<ModuleHandle>>> = Rc::new(RefCell::new(Vec::new()));
        for plugin in plugins {
            for module in &plugin.manifest.modules {
                let dom_id = format!("mod-{}-{}", plugin.manifest.plugin.name, module.name);
                modules.borrow_mut().push(ModuleHandle {
                    dom_id,
                    exec: module.exec.clone(),
                    interval: module.interval,
                    plugin_dir: plugin.dir.clone(),
                    socket_path: socket_path.clone(),
                });
            }
        }

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(false);
        container.append(&webview);

        if !config.statusbar.enabled {
            container.set_visible(false);
        }

        // Schedule module execution after WebView loads
        let modules_ref = modules.clone();
        let wv = webview.clone();
        webview.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                let modules_ref2 = modules_ref.clone();
                let wv2 = wv.clone();
                // Small delay to ensure DOM is ready
                glib::timeout_add_local_once(Duration::from_millis(100), move || {
                    schedule_modules(&modules_ref2, &wv2);
                });
            }
        });

        Self {
            container,
            webview,
            modules,
        }
    }

    pub fn set_visible(&self, visible: bool) {
        self.container.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }

    pub fn toggle(&self) -> bool {
        let new_visible = !self.is_visible();
        self.set_visible(new_visible);
        new_visible
    }

    pub fn reload(&self, config: &TurmConfig, plugins: &[LoadedPlugin]) {
        let theme = Theme::by_name(&config.theme.name).unwrap_or_default();
        let html = build_bar_html(plugins, &theme, config.statusbar.height);
        self.webview.load_html(&html, None);
        // Modules will re-schedule via connect_load_changed
    }
}

fn schedule_modules(modules: &Rc<RefCell<Vec<ModuleHandle>>>, webview: &webkit6::WebView) {
    let modules_ref = modules.borrow();
    eprintln!("[turm] statusbar: scheduling {} modules", modules_ref.len());
    for module in modules_ref.iter() {
        eprintln!(
            "[turm] statusbar: module {} exec={} interval={}s",
            module.dom_id, module.exec, module.interval
        );
        let ctx = ModuleRunCtx {
            dom_id: module.dom_id.clone(),
            exec: module.exec.clone(),
            plugin_dir: module.plugin_dir.clone(),
            socket_path: module.socket_path.clone(),
            interval: module.interval,
            webview: webview.clone(),
        };
        run_and_schedule(ctx);
    }
}

#[derive(Clone)]
struct ModuleRunCtx {
    dom_id: String,
    exec: String,
    plugin_dir: std::path::PathBuf,
    socket_path: String,
    interval: u64,
    webview: webkit6::WebView,
}

fn run_and_schedule(ctx: ModuleRunCtx) {
    let rx = run_module_exec(&ctx.exec, &ctx.plugin_dir, &ctx.socket_path);

    glib::timeout_add_local(Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(output) => {
                let (text, tooltip) = parse_output(&output);
                eprintln!("[turm] statusbar: {} -> {:?}", ctx.dom_id, text);

                // Update DOM via JavaScript
                let escaped_text = text
                    .replace('\\', "\\\\")
                    .replace('\'', "\\'")
                    .replace('\n', "\\n");
                let escaped_tooltip = tooltip
                    .as_deref()
                    .unwrap_or("")
                    .replace('\\', "\\\\")
                    .replace('\'', "\\'")
                    .replace('\n', "\\n");
                let dom_id = &ctx.dom_id;

                let js = format!(
                    r#"(() => {{
                        const el = document.getElementById('{dom_id}');
                        if (el) {{
                            el.textContent = '{escaped_text}';
                            el.title = '{escaped_tooltip}';
                        }}
                    }})()"#,
                );
                ctx.webview.evaluate_javascript(
                    &js,
                    None,
                    None,
                    gtk4::gio::Cancellable::NONE,
                    |_| {},
                );

                // Schedule next run
                let next = ctx.clone();
                glib::timeout_add_local_once(Duration::from_secs(ctx.interval), move || {
                    run_and_schedule(next);
                });

                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}
