use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use cef::*;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;

use turm_core::plugin::LoadedPlugin;
use turm_core::theme::Theme;

use crate::panel::Panel;
use crate::socket::{EventBus, SocketCommand};

fn build_theme_css(theme: &Theme) -> String {
    format!(
        r#":root {{
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
body {{
    background-color: {bg};
    color: {text};
    font-family: system-ui, -apple-system, sans-serif;
    margin: 0;
    padding: 0;
}}"#,
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
    )
}

fn build_bridge_js(plugin_name: &str, panel_name: &str, panel_id: &str) -> String {
    format!(
        r#"(() => {{
    const _listeners = {{}};
    window.turm = {{
        panel: {{
            id: {id},
            name: {name},
            plugin: {plugin},
        }},
        async call(method, params = {{}}) {{
            return new Promise((resolve, reject) => {{
                window.cefQuery({{
                    request: JSON.stringify({{ method, params }}),
                    onSuccess: (response) => {{
                        try {{
                            const parsed = JSON.parse(response);
                            if (!parsed.ok) reject(new Error(parsed.error?.message || "Unknown error"));
                            else resolve(parsed.result);
                        }} catch (e) {{ reject(e); }}
                    }},
                    onFailure: (code, msg) => reject(new Error(msg)),
                }});
            }});
        }},
        on(type, callback) {{
            if (!_listeners[type]) _listeners[type] = [];
            _listeners[type].push(callback);
        }},
        off(type, callback) {{
            if (!_listeners[type]) return;
            _listeners[type] = _listeners[type].filter(cb => cb !== callback);
        }},
        _handleEvent(type, data) {{
            const cbs = _listeners[type] || [];
            for (const cb of cbs) {{
                try {{ cb(data); }} catch (e) {{ console.error("turm event handler error:", e); }}
            }}
            const wildcards = _listeners["*"] || [];
            for (const cb of wildcards) {{
                try {{ cb(type, data); }} catch (e) {{ console.error("turm event handler error:", e); }}
            }}
        }},
    }};
}})()"#,
        id = serde_json::to_string(panel_id).unwrap(),
        name = serde_json::to_string(panel_name).unwrap(),
        plugin = serde_json::to_string(plugin_name).unwrap(),
    )
}

struct PluginPanelState {
    picture: gtk4::Picture,
    size: (i32, i32),
}

pub struct CefPluginPanel {
    pub id: String,
    pub plugin_name: String,
    pub panel_name: String,
    pub title: String,
    pub container: gtk4::Box,
    state: Rc<RefCell<PluginPanelState>>,
    _browser: Rc<RefCell<Option<cef::Browser>>>,
}

impl CefPluginPanel {
    pub fn new(
        plugin: &LoadedPlugin,
        panel_def: &turm_core::plugin::PluginPanelDef,
        theme: &Theme,
        _dispatch_tx: mpsc::Sender<SocketCommand>,
        event_bus: EventBus,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let plugin_name = plugin.manifest.plugin.name.clone();
        let panel_name = panel_def.name.clone();
        let title = panel_def.title.clone();

        let picture = gtk4::Picture::new();
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_content_fit(gtk4::ContentFit::Fill);
        picture.set_focusable(true);
        picture.set_can_focus(true);

        let state = Rc::new(RefCell::new(PluginPanelState {
            picture: picture.clone(),
            size: (800, 600),
        }));

        let browser_cell: Rc<RefCell<Option<cef::Browser>>> = Rc::new(RefCell::new(None));

        // Build CEF handlers
        let render_handler = PluginRenderHandlerBuilder::build(PluginRenderHandler {
            state: state.clone(),
        });
        let life_span_handler = PluginLifeSpanHandlerBuilder::build(PluginLifeSpanHandler {
            browser: browser_cell.clone(),
        });

        let mut client = PluginClientBuilder::new(render_handler, life_span_handler);

        let window_info = WindowInfo::default().set_as_windowless(0);
        let browser_settings = BrowserSettings {
            windowless_frame_rate: 30,
            ..Default::default()
        };

        // Load the plugin HTML file
        let file_path = plugin.dir.join(&panel_def.file);
        let uri = format!("file://{}", file_path.display());

        let browser = browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&CefString::from(uri.as_str())),
            Some(&browser_settings),
            None,
            None,
        );

        if let Some(browser) = &browser {
            *browser_cell.borrow_mut() = Some(browser.clone());

            // Inject theme CSS and bridge JS after page loads
            // We use Frame::execute_java_script to inject after load
            let theme_css = build_theme_css(theme);
            let bridge_js = build_bridge_js(&plugin_name, &panel_name, &id);
            let css_escaped = serde_json::to_string(&theme_css).unwrap();

            let inject_js = format!(
                r#"
                // Inject theme CSS
                const style = document.createElement('style');
                style.textContent = {css};
                document.head.appendChild(style);
                // Inject bridge
                {bridge}
                "#,
                css = css_escaped,
                bridge = bridge_js,
            );

            // Schedule injection after a short delay to wait for page load
            let bc = browser_cell.clone();
            let inject = inject_js.clone();
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(frame) = browser.main_frame()
                {
                    frame.execute_java_script(
                        Some(&CefString::from(inject.as_str())),
                        None,
                        0,
                    );
                }
            });
        }

        // Forward events from EventBus
        {
            let bc = browser_cell.clone();
            let (etx, erx) = mpsc::channel::<String>();
            event_bus.lock().unwrap().push(etx);

            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                while let Ok(event_json) = erx.try_recv() {
                    if let Ok(event) =
                        serde_json::from_str::<turm_core::protocol::Event>(&event_json)
                    {
                        let type_escaped = serde_json::to_string(&event.event_type).unwrap();
                        let data_json = serde_json::to_string(&event.data).unwrap();
                        let js = format!(
                            "if (window.turm && window.turm._handleEvent) turm._handleEvent({type_escaped}, {data_json})"
                        );
                        if let Some(browser) = bc.borrow().as_ref()
                            && let Some(frame) = browser.main_frame()
                        {
                            frame.execute_java_script(
                                Some(&CefString::from(js.as_str())),
                                None,
                                0,
                            );
                        }
                    }
                }
                gtk4::glib::ControlFlow::Continue
            });
        }

        // Input forwarding (keyboard + mouse) for the picture widget
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
            picture.add_controller(key_ctrl);
        }
        {
            let bc = browser_cell.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(0);
            gesture.connect_pressed(move |gesture, _n, x, y| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let button = match gesture.current_button() {
                        1 => MouseButtonType::LEFT,
                        2 => MouseButtonType::MIDDLE,
                        3 => MouseButtonType::RIGHT,
                        _ => MouseButtonType::LEFT,
                    };
                    let event = MouseEvent { x: x as _, y: y as _, modifiers: 0 };
                    host.send_mouse_click_event(Some(&event), button, 0, 1);
                }
            });
            let bc2 = browser_cell.clone();
            gesture.connect_released(move |gesture, _n, x, y| {
                if let Some(browser) = bc2.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let button = match gesture.current_button() {
                        1 => MouseButtonType::LEFT,
                        2 => MouseButtonType::MIDDLE,
                        3 => MouseButtonType::RIGHT,
                        _ => MouseButtonType::LEFT,
                    };
                    let event = MouseEvent { x: x as _, y: y as _, modifiers: 0 };
                    host.send_mouse_click_event(Some(&event), button, 1, 1);
                }
            });
            picture.add_controller(gesture);
        }
        {
            let bc = browser_cell.clone();
            let motion = gtk4::EventControllerMotion::new();
            motion.connect_motion(move |_, x, y| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let event = MouseEvent { x: x as _, y: y as _, modifiers: 0 };
                    host.send_mouse_move_event(Some(&event), 0);
                }
            });
            picture.add_controller(motion);
        }
        {
            let bc = browser_cell.clone();
            let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
            scroll.connect_scroll(move |_, dx, dy| {
                if let Some(browser) = bc.borrow().as_ref()
                    && let Some(host) = browser.host()
                {
                    let event = MouseEvent { x: 0, y: 0, modifiers: 0 };
                    host.send_mouse_wheel_event(Some(&event), (dx * -120.0) as _, (dy * -120.0) as _);
                }
                gtk4::glib::Propagation::Stop
            });
            picture.add_controller(scroll);
        }

        // Track size changes via periodic polling (Picture has no connect_resize)
        {
            let sc = state.clone();
            let bc = browser_cell.clone();
            let pic = picture.clone();
            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                let w = pic.width();
                let h = pic.height();
                if w > 0 && h > 0 {
                    let mut st = sc.borrow_mut();
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

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        container.append(&picture);

        Self {
            id,
            plugin_name,
            panel_name,
            title,
            container,
            state,
            _browser: browser_cell,
        }
    }
}

impl Panel for CefPluginPanel {
    fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn panel_type(&self) -> &str {
        "plugin"
    }

    fn grab_focus(&self) {
        self.state.borrow().picture.grab_focus();
    }

    fn id(&self) -> &str {
        &self.id
    }
}

// -- CEF Handlers for plugin panel --

#[derive(Clone)]
struct PluginRenderHandler {
    state: Rc<RefCell<PluginPanelState>>,
}

wrap_render_handler! {
    struct PluginRenderHandlerBuilder {
        handler: PluginRenderHandler,
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
                width, height,
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

impl PluginRenderHandlerBuilder {
    fn build(handler: PluginRenderHandler) -> cef::RenderHandler {
        Self::new(handler)
    }
}

#[derive(Clone)]
struct PluginLifeSpanHandler {
    browser: Rc<RefCell<Option<cef::Browser>>>,
}

wrap_life_span_handler! {
    struct PluginLifeSpanHandlerBuilder {
        handler: PluginLifeSpanHandler,
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

impl PluginLifeSpanHandlerBuilder {
    fn build(handler: PluginLifeSpanHandler) -> cef::LifeSpanHandler {
        Self::new(handler)
    }
}

wrap_client! {
    struct PluginClientBuilder {
        render_handler: cef::RenderHandler,
        life_span_handler: cef::LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }
    }
}
