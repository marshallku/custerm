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
