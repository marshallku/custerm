use gtk4::prelude::*;
use webkit6::prelude::*;

use crate::panel::Panel;

pub struct WebViewPanel {
    pub id: String,
    pub container: gtk4::Box,
    pub webview: webkit6::WebView,
}

impl WebViewPanel {
    pub fn new(url: &str) -> Self {
        let webview = webkit6::WebView::new();

        // Sane defaults
        if let Some(settings) = webkit6::prelude::WebViewExt::settings(&webview) {
            settings.set_enable_javascript(true);
            settings.set_allow_file_access_from_file_urls(false);
            settings.set_allow_universal_access_from_file_urls(false);
            settings.set_enable_developer_extras(true);
        }

        webview.set_hexpand(true);
        webview.set_vexpand(true);
        webview.load_uri(url);

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&webview);

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            container,
            webview,
        }
    }

    pub fn navigate(&self, url: &str) {
        self.webview.load_uri(url);
    }

    pub fn go_back(&self) {
        self.webview.go_back();
    }

    pub fn go_forward(&self) {
        self.webview.go_forward();
    }

    pub fn reload(&self) {
        self.webview.reload();
    }

    pub fn execute_js(&self, code: &str, callback: impl FnOnce(Result<String, String>) + 'static) {
        self.webview.evaluate_javascript(
            code,
            None,
            None,
            gtk4::gio::Cancellable::NONE,
            move |result| {
                let outcome = match result {
                    Ok(value) => {
                        let s = value.to_str();
                        Ok(s.to_string())
                    }
                    Err(e) => Err(e.to_string()),
                };
                callback(outcome);
            },
        );
    }

    pub fn snapshot(&self, callback: impl FnOnce(Result<String, String>) + 'static) {
        self.webview.snapshot(
            webkit6::SnapshotRegion::Visible,
            webkit6::SnapshotOptions::NONE,
            gtk4::gio::Cancellable::NONE,
            move |result| {
                let outcome = match result {
                    Ok(texture) => {
                        let bytes = texture.save_to_png_bytes();
                        Ok(gtk4::glib::base64_encode(&bytes).to_string())
                    }
                    Err(e) => Err(e.to_string()),
                };
                callback(outcome);
            },
        );
    }

    pub fn current_url(&self) -> String {
        self.webview
            .uri()
            .map(|u| u.to_string())
            .unwrap_or_default()
    }
}

impl Panel for WebViewPanel {
    fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    fn title(&self) -> String {
        self.webview
            .title()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "WebView".to_string())
    }

    fn panel_type(&self) -> &str {
        "webview"
    }

    fn grab_focus(&self) {
        self.webview.grab_focus();
    }

    fn id(&self) -> &str {
        &self.id
    }
}

/// Pre-built JS snippets for AI agent DOM inspection.
/// These return JSON strings so results are structured.
pub mod js {
    /// Query a single element, return its text, tag, attributes, bounding rect
    pub fn query_selector(selector: &str) -> String {
        format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return JSON.stringify(null);
                const r = el.getBoundingClientRect();
                return JSON.stringify({{
                    tag: el.tagName.toLowerCase(),
                    text: el.innerText?.slice(0, 2000) || "",
                    value: el.value || null,
                    href: el.href || null,
                    src: el.src || null,
                    class: el.className || "",
                    id: el.id || "",
                    rect: {{ x: r.x, y: r.y, w: r.width, h: r.height }},
                    visible: r.width > 0 && r.height > 0,
                }});
            }})()"#,
            sel = serde_json::to_string(selector).unwrap()
        )
    }

    /// Query all matching elements, return array of summaries
    pub fn query_selector_all(selector: &str, limit: u32) -> String {
        format!(
            r#"(() => {{
                const els = [...document.querySelectorAll({sel})].slice(0, {limit});
                return JSON.stringify(els.map((el, i) => {{
                    const r = el.getBoundingClientRect();
                    return {{
                        index: i,
                        tag: el.tagName.toLowerCase(),
                        text: el.innerText?.slice(0, 500) || "",
                        value: el.value || null,
                        href: el.href || null,
                        class: el.className || "",
                        id: el.id || "",
                        rect: {{ x: r.x, y: r.y, w: r.width, h: r.height }},
                    }};
                }}));
            }})()"#,
            sel = serde_json::to_string(selector).unwrap(),
            limit = limit,
        )
    }

    /// Get computed styles for an element
    pub fn get_styles(selector: &str, properties: &[&str]) -> String {
        let props_json = serde_json::to_string(properties).unwrap();
        format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return JSON.stringify(null);
                const cs = getComputedStyle(el);
                const props = {props};
                const result = {{}};
                props.forEach(p => result[p] = cs.getPropertyValue(p));
                return JSON.stringify(result);
            }})()"#,
            sel = serde_json::to_string(selector).unwrap(),
            props = props_json,
        )
    }

    /// Click an element by selector
    pub fn click(selector: &str) -> String {
        format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return JSON.stringify({{ ok: false, error: "not found" }});
                el.click();
                return JSON.stringify({{ ok: true }});
            }})()"#,
            sel = serde_json::to_string(selector).unwrap(),
        )
    }

    /// Type text into an input element
    pub fn fill(selector: &str, value: &str) -> String {
        format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return JSON.stringify({{ ok: false, error: "not found" }});
                el.focus();
                el.value = {val};
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return JSON.stringify({{ ok: true }});
            }})()"#,
            sel = serde_json::to_string(selector).unwrap(),
            val = serde_json::to_string(value).unwrap(),
        )
    }

    /// Scroll to position or element
    pub fn scroll(selector: Option<&str>, x: i32, y: i32) -> String {
        match selector {
            Some(sel) => format!(
                r#"(() => {{
                    const el = document.querySelector({sel});
                    if (!el) return JSON.stringify({{ ok: false, error: "not found" }});
                    el.scrollIntoView({{ behavior: "smooth", block: "center" }});
                    return JSON.stringify({{ ok: true }});
                }})()"#,
                sel = serde_json::to_string(sel).unwrap(),
            ),
            None => format!(
                r#"(() => {{
                    window.scrollTo({x}, {y});
                    return JSON.stringify({{ ok: true, scrollX: window.scrollX, scrollY: window.scrollY }});
                }})()"#,
                x = x,
                y = y,
            ),
        }
    }

    /// Get page metadata (title, url, dimensions, forms, links count)
    pub fn page_info() -> String {
        r#"(() => {
            return JSON.stringify({
                title: document.title,
                url: location.href,
                width: document.documentElement.scrollWidth,
                height: document.documentElement.scrollHeight,
                viewportWidth: window.innerWidth,
                viewportHeight: window.innerHeight,
                scrollX: window.scrollX,
                scrollY: window.scrollY,
                forms: document.forms.length,
                links: document.links.length,
                images: document.images.length,
                inputs: document.querySelectorAll('input, textarea, select').length,
                buttons: document.querySelectorAll('button, [role="button"], input[type="submit"]').length,
            });
        })()"#.to_string()
    }
}
