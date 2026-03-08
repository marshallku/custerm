use std::cell::RefCell;
use std::rc::Rc;

use cef::*;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;

use crate::panel::Panel;

type JsCallback = Box<dyn FnOnce(Result<String, String>)>;

fn build_toolbar_css(theme: &turm_core::theme::Theme) -> String {
    format!(
        r#"
.turm-url-bar {{
    background-color: {surface2};
    padding: 4px 8px;
}}
.turm-url-entry {{
    background-color: {bg};
    color: {text};
    border: 1px solid {overlay0};
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 12px;
}}
.turm-url-entry:focus {{
    border-color: {accent};
}}
.turm-nav-btn {{
    min-width: 24px;
    min-height: 24px;
    padding: 2px;
    border-radius: 4px;
    color: {text};
}}
.turm-nav-btn:hover {{
    background-color: {overlay0};
}}
"#,
        surface2 = theme.surface2,
        bg = theme.background,
        text = theme.text,
        overlay0 = theme.overlay0,
        accent = theme.accent,
    )
}

/// Shared state between CEF handlers and the GTK panel.
struct CefPanelState {
    /// The gtk4::Picture that displays rendered content.
    picture: gtk4::Picture,
    /// Current view size in logical pixels.
    size: (i32, i32),
    /// Current page title.
    title: String,
    /// Current URL.
    url: String,
    /// URL entry widget (for updating on navigation).
    url_entry: Option<gtk4::Entry>,
    /// Back/Forward button sensitivity.
    can_go_back: bool,
    can_go_forward: bool,
    /// Is loading.
    is_loading: bool,
    /// Pending JS callbacks keyed by DevTools message ID.
    js_callbacks: Vec<(i32, JsCallback)>,
    /// Next DevTools message ID.
    next_js_id: i32,
}

pub struct CefBrowserPanel {
    pub id: String,
    pub container: gtk4::Box,
    state: Rc<RefCell<CefPanelState>>,
    browser: Rc<RefCell<Option<cef::Browser>>>,
}

impl CefBrowserPanel {
    pub fn new(url: &str, theme: &turm_core::theme::Theme) -> Self {
        let id = uuid::Uuid::new_v4().to_string();

        // Create the Picture widget for OSR rendering
        let picture = gtk4::Picture::new();
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_content_fit(gtk4::ContentFit::Fill);

        // Shared state
        let state = Rc::new(RefCell::new(CefPanelState {
            picture: picture.clone(),
            size: (800, 600),
            title: String::new(),
            url: url.to_string(),
            url_entry: None,
            can_go_back: false,
            can_go_forward: false,
            is_loading: false,
            js_callbacks: Vec::new(),
            next_js_id: 1,
        }));

        // Build toolbar
        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        toolbar.add_css_class("turm-url-bar");

        let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_btn.add_css_class("flat");
        back_btn.add_css_class("turm-nav-btn");
        back_btn.set_tooltip_text(Some("Back"));
        back_btn.set_sensitive(false);

        let forward_btn = gtk4::Button::from_icon_name("go-next-symbolic");
        forward_btn.add_css_class("flat");
        forward_btn.add_css_class("turm-nav-btn");
        forward_btn.set_tooltip_text(Some("Forward"));
        forward_btn.set_sensitive(false);

        let reload_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        reload_btn.add_css_class("flat");
        reload_btn.add_css_class("turm-nav-btn");
        reload_btn.set_tooltip_text(Some("Reload"));

        let url_entry = gtk4::Entry::new();
        url_entry.set_hexpand(true);
        url_entry.add_css_class("turm-url-entry");
        url_entry.set_text(url);

        toolbar.append(&back_btn);
        toolbar.append(&forward_btn);
        toolbar.append(&reload_btn);
        toolbar.append(&url_entry);

        state.borrow_mut().url_entry = Some(url_entry.clone());

        // Container
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&toolbar);
        container.append(&picture);

        // CSS
        let css = gtk4::CssProvider::new();
        css.load_from_string(&build_toolbar_css(theme));
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );

        let browser_cell: Rc<RefCell<Option<cef::Browser>>> = Rc::new(RefCell::new(None));

        // Create CEF browser (windowless/OSR)
        let render_handler = TurmRenderHandlerBuilder::build(TurmRenderHandler {
            state: state.clone(),
        });
        let display_handler = TurmDisplayHandlerBuilder::build(TurmDisplayHandler {
            state: state.clone(),
        });
        let load_handler = TurmLoadHandlerBuilder::build(TurmLoadHandler {
            state: state.clone(),
        });
        let life_span_handler = TurmLifeSpanHandlerBuilder::build(TurmLifeSpanHandler {
            browser: browser_cell.clone(),
        });

        let mut client = TurmClientBuilder::new(render_handler, display_handler, load_handler, life_span_handler);

        let window_info = WindowInfo::default().set_as_windowless(0);
        let browser_settings = BrowserSettings {
            windowless_frame_rate: 30,
            ..Default::default()
        };

        let browser = browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&CefString::from(url)),
            Some(&browser_settings),
            None,
            None,
        );

        if let Some(browser) = &browser {
            *browser_cell.borrow_mut() = Some(browser.clone());
        }

        // Wire up toolbar buttons
        {
            let bc = browser_cell.clone();
            url_entry.connect_activate(move |entry| {
                if let Some(browser) = bc.borrow().as_ref() {
                    let text = entry.text();
                    let url = if text.contains("://") || text.starts_with("about:") {
                        text.to_string()
                    } else if text.contains('.') && !text.contains(' ') {
                        format!("https://{text}")
                    } else {
                        text.to_string()
                    };
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(url.as_str())));
                    }
                }
            });
        }
        {
            let bc = browser_cell.clone();
            back_btn.connect_clicked(move |_| {
                if let Some(browser) = bc.borrow().as_ref() {
                    browser.go_back();
                }
            });
        }
        {
            let bc = browser_cell.clone();
            forward_btn.connect_clicked(move |_| {
                if let Some(browser) = bc.borrow().as_ref() {
                    browser.go_forward();
                }
            });
        }
        {
            let bc = browser_cell.clone();
            reload_btn.connect_clicked(move |btn| {
                if let Some(browser) = bc.borrow().as_ref() {
                    if browser.is_loading() != 0 {
                        browser.stop_load();
                        btn.set_icon_name("view-refresh-symbolic");
                    } else {
                        browser.reload();
                    }
                }
            });
        }

        // Input event forwarding: keyboard
        {
            let bc = browser_cell.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, keyval, keycode, modifier| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    use cef::sys::cef_event_flags_t;
                    let mut modifiers: u32 = 0;
                    if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                        modifiers |= cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0;
                    }
                    if modifier.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                        modifiers |= cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0;
                    }
                    if modifier.contains(gtk4::gdk::ModifierType::ALT_MASK) {
                        modifiers |= cef_event_flags_t::EVENTFLAG_ALT_DOWN.0;
                    }

                    let event = KeyEvent {
                        type_: KeyEventType::RAWKEYDOWN,
                        modifiers,
                        windows_key_code: keyval.into_glib() as _,
                        native_key_code: keycode as _,
                        ..Default::default()
                    };
                    host.send_key_event(Some(&event));

                    // Also send CHAR event for text input
                    if let Some(ch) = keyval.to_unicode() {
                        let char_event = KeyEvent {
                            type_: KeyEventType::CHAR,
                            modifiers,
                            character: ch as u16,
                            unmodified_character: ch as u16,
                            windows_key_code: ch as _,
                            ..Default::default()
                        };
                        host.send_key_event(Some(&char_event));
                    }
                }
                gtk4::glib::Propagation::Stop
            });
            key_ctrl.connect_key_released(move |_, _keyval, _keycode, _modifier| {
                // Could send KEYUP event here if needed
            });
            picture.add_controller(key_ctrl);
        }

        // Input: mouse click
        {
            let bc = browser_cell.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(0); // All buttons
            gesture.connect_pressed(move |gesture, _n_press, x, y| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let button = match gesture.current_button() {
                        1 => MouseButtonType::LEFT,
                        2 => MouseButtonType::MIDDLE,
                        3 => MouseButtonType::RIGHT,
                        _ => MouseButtonType::LEFT,
                    };
                    let event = MouseEvent {
                        x: x as _,
                        y: y as _,
                        modifiers: 0,
                    };
                    host.send_mouse_click_event(Some(&event), button, 0, 1);
                }
            });
            let bc2 = browser_cell.clone();
            gesture.connect_released(move |gesture, _n_press, x, y| {
                if let Some(browser) = bc2.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let button = match gesture.current_button() {
                        1 => MouseButtonType::LEFT,
                        2 => MouseButtonType::MIDDLE,
                        3 => MouseButtonType::RIGHT,
                        _ => MouseButtonType::LEFT,
                    };
                    let event = MouseEvent {
                        x: x as _,
                        y: y as _,
                        modifiers: 0,
                    };
                    host.send_mouse_click_event(Some(&event), button, 1, 1);
                }
            });
            picture.add_controller(gesture);
        }

        // Input: mouse move
        {
            let bc = browser_cell.clone();
            let motion = gtk4::EventControllerMotion::new();
            motion.connect_motion(move |_, x, y| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let event = MouseEvent {
                        x: x as _,
                        y: y as _,
                        modifiers: 0,
                    };
                    host.send_mouse_move_event(Some(&event), 0);
                }
            });
            picture.add_controller(motion);
        }

        // Input: scroll
        {
            let bc = browser_cell.clone();
            let scroll = gtk4::EventControllerScroll::new(
                gtk4::EventControllerScrollFlags::BOTH_AXES,
            );
            scroll.connect_scroll(move |_, dx, dy| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let event = MouseEvent {
                        x: 0,
                        y: 0,
                        modifiers: 0,
                    };
                    host.send_mouse_wheel_event(
                        Some(&event),
                        (dx * -120.0) as _,
                        (dy * -120.0) as _,
                    );
                }
                gtk4::glib::Propagation::Stop
            });
            picture.add_controller(scroll);
        }

        // Make picture focusable for keyboard input
        picture.set_focusable(true);
        picture.set_can_focus(true);

        // Track size changes via periodic polling (Picture has no connect_resize)
        {
            let state_clone = state.clone();
            let bc = browser_cell.clone();
            let pic = picture.clone();
            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                let w = pic.width();
                let h = pic.height();
                if w > 0 && h > 0 {
                    let mut st = state_clone.borrow_mut();
                    if st.size != (w, h) {
                        st.size = (w, h);
                        drop(st);
                        if let Some(browser) = bc.borrow().as_ref()
                            && let Some(host) = browser.host()
                        {
                            host.was_resized();
                        }
                    }
                }
                gtk4::glib::ControlFlow::Continue
            });
        }

        Self {
            id,
            container,
            state,
            browser: browser_cell,
        }
    }

    pub fn navigate(&self, url: &str) {
        if let Some(browser) = self.browser.borrow().as_ref()
            && let Some(frame) = browser.main_frame()
        {
            frame.load_url(Some(&CefString::from(url)));
        }
    }

    pub fn go_back(&self) {
        if let Some(browser) = self.browser.borrow().as_ref() {
            browser.go_back();
        }
    }

    pub fn go_forward(&self) {
        if let Some(browser) = self.browser.borrow().as_ref() {
            browser.go_forward();
        }
    }

    pub fn reload(&self) {
        if let Some(browser) = self.browser.borrow().as_ref() {
            browser.reload();
        }
    }

    /// Execute JavaScript and get the result via DevTools protocol.
    pub fn execute_js(&self, code: &str, callback: impl FnOnce(Result<String, String>) + 'static) {
        let browser = self.browser.borrow();
        let Some(browser) = browser.as_ref() else {
            callback(Err("No browser".to_string()));
            return;
        };
        let Some(host) = browser.host() else {
            callback(Err("No browser host".to_string()));
            return;
        };

        let mut state = self.state.borrow_mut();
        let msg_id = state.next_js_id;
        state.next_js_id += 1;
        state.js_callbacks.push((msg_id, Box::new(callback)));

        // Use DevTools protocol Runtime.evaluate
        let expression = serde_json::to_string(code).unwrap();
        let cdp_msg = format!(
            r#"{{"id":{msg_id},"method":"Runtime.evaluate","params":{{"expression":{expression},"returnByValue":true}}}}"#
        );
        host.send_dev_tools_message(Some(cdp_msg.as_bytes()));
    }

    /// Execute JS without waiting for result (fire and forget).
    #[allow(dead_code)]
    pub fn execute_js_fire(&self, code: &str) {
        if let Some(browser) = self.browser.borrow().as_ref()
            && let Some(frame) = browser.main_frame()
        {
            frame.execute_java_script(
                Some(&CefString::from(code)),
                None,
                0,
            );
        }
    }

    pub fn snapshot(&self, callback: impl FnOnce(Result<String, String>) + 'static) {
        // For CEF, we capture the current Picture paintable as PNG
        let state = self.state.borrow();
        if let Some(paintable) = state.picture.paintable()
            && let Some(texture) = paintable.downcast_ref::<gtk4::gdk::Texture>()
        {
            let bytes = texture.save_to_png_bytes();
            callback(Ok(gtk4::glib::base64_encode(&bytes).to_string()));
            return;
        }
        callback(Err("No content to snapshot".to_string()));
    }

    pub fn current_url(&self) -> String {
        self.state.borrow().url.clone()
    }
}

impl Panel for CefBrowserPanel {
    fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    fn title(&self) -> String {
        let state = self.state.borrow();
        if state.title.is_empty() {
            "WebView".to_string()
        } else {
            state.title.clone()
        }
    }

    fn panel_type(&self) -> &str {
        "webview"
    }

    fn grab_focus(&self) {
        self.state.borrow().picture.grab_focus();
    }

    fn id(&self) -> &str {
        &self.id
    }
}

// -- CEF Handler implementations --

#[derive(Clone)]
struct TurmRenderHandler {
    state: Rc<RefCell<CefPanelState>>,
}

wrap_render_handler! {
    struct TurmRenderHandlerBuilder {
        handler: TurmRenderHandler,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut cef::Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let state = self.handler.state.borrow();
                let (w, h) = state.size;
                rect.width = w.max(1);
                rect.height = h.max(1);
            }
        }

        fn on_paint(
            &self,
            _browser: Option<&mut cef::Browser>,
            _type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }

            let buffer_size = (width * height * 4) as usize;
            let buffer_slice = unsafe { std::slice::from_raw_parts(buffer, buffer_size) };
            let gbytes = gtk4::glib::Bytes::from(buffer_slice);

            let texture = gtk4::gdk::MemoryTexture::new(
                width,
                height,
                gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied,
                &gbytes,
                (width * 4) as usize,
            );

            let state = self.handler.state.borrow();
            state.picture.set_paintable(Some(&texture));
        }

        fn screen_info(
            &self,
            _browser: Option<&mut cef::Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            if let Some(info) = screen_info {
                info.device_scale_factor = 1.0;
                return 1;
            }
            0
        }
    }
}

impl TurmRenderHandlerBuilder {
    fn build(handler: TurmRenderHandler) -> cef::RenderHandler {
        Self::new(handler)
    }
}

#[derive(Clone)]
struct TurmDisplayHandler {
    state: Rc<RefCell<CefPanelState>>,
}

wrap_display_handler! {
    struct TurmDisplayHandlerBuilder {
        handler: TurmDisplayHandler,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut cef::Browser>, title: Option<&CefString>) {
            let mut state = self.handler.state.borrow_mut();
            state.title = title.map(|t| t.to_string()).unwrap_or_default();
        }

        fn on_address_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            let mut state = self.handler.state.borrow_mut();
            if let Some(url) = url {
                let url_str = url.to_string();
                state.url = url_str.clone();
                if let Some(entry) = &state.url_entry {
                    entry.set_text(&url_str);
                }
            }
        }
    }
}

impl TurmDisplayHandlerBuilder {
    fn build(handler: TurmDisplayHandler) -> cef::DisplayHandler {
        Self::new(handler)
    }
}

#[derive(Clone)]
struct TurmLoadHandler {
    state: Rc<RefCell<CefPanelState>>,
}

wrap_load_handler! {
    struct TurmLoadHandlerBuilder {
        handler: TurmLoadHandler,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            let mut state = self.handler.state.borrow_mut();
            state.is_loading = is_loading != 0;
            state.can_go_back = can_go_back != 0;
            state.can_go_forward = can_go_forward != 0;
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let error_text = error_text.map(|t| t.to_string()).unwrap_or_default();
            let failed_url = failed_url.map(|u| u.to_string()).unwrap_or_default();
            eprintln!("[cef] Load error for {failed_url}: {error_text}");
        }
    }
}

impl TurmLoadHandlerBuilder {
    fn build(handler: TurmLoadHandler) -> cef::LoadHandler {
        Self::new(handler)
    }
}

#[derive(Clone)]
struct TurmLifeSpanHandler {
    browser: Rc<RefCell<Option<cef::Browser>>>,
}

wrap_life_span_handler! {
    struct TurmLifeSpanHandlerBuilder {
        handler: TurmLifeSpanHandler,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
            if let Some(browser) = browser {
                *self.handler.browser.borrow_mut() = Some(browser.clone());
            }
        }

        fn on_before_close(&self, _browser: Option<&mut cef::Browser>) {
            *self.handler.browser.borrow_mut() = None;
        }
    }
}

impl TurmLifeSpanHandlerBuilder {
    fn build(handler: TurmLifeSpanHandler) -> cef::LifeSpanHandler {
        Self::new(handler)
    }
}

// Client that holds all handlers
wrap_client! {
    struct TurmClientBuilder {
        render_handler: cef::RenderHandler,
        display_handler: cef::DisplayHandler,
        load_handler: cef::LoadHandler,
        life_span_handler: cef::LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display_handler.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }
    }
}

/// Pre-built JS snippets for AI agent DOM inspection (same as webview.rs).
pub mod js {
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
