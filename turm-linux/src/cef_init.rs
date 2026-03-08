use cef::*;
use std::cell::RefCell;

/// Check if the current process is a CEF sub-process (renderer, GPU, etc.).
/// If so, handle it and return true. The caller should exit immediately.
/// If this is the main browser process, return false.
pub fn handle_subprocess() -> bool {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    let Some(cmd) = args.as_cmd_line() else {
        return false;
    };

    let switch = CefString::from("type");
    if cmd.has_switch(Some(&switch)) == 1 {
        // This is a CEF sub-process — handle and exit
        execute_process(Some(args.as_main_args()), None::<&mut App>, std::ptr::null_mut());
        return true;
    }

    false
}

/// Initialize CEF for the main browser process.
/// Must be called before creating any browsers.
/// Returns the CefState that must be kept alive for the duration of the app.
pub fn initialize() -> CefState {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    let mut app = TurmAppBuilder::build(TurmApp);

    // Execute process first (returns -1 for the browser process)
    let ret = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    assert_eq!(ret, -1, "Expected browser process, got sub-process");

    let settings = Settings {
        windowless_rendering_enabled: true as _,
        external_message_pump: true as _,
        log_file: CefString::from("/tmp/turm-cef.log"),
        log_severity: LogSeverity::VERBOSE,
        no_sandbox: true as _,
        ..Default::default()
    };

    let result = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    assert_eq!(result, 1, "CEF initialization failed");
    eprintln!("[cef] initialized");

    CefState { _app: app }
}

/// Find the CEF directory containing libcef.so and resource files.
fn find_cef_dir() -> std::path::PathBuf {
    // 1. CEF_PATH env var
    if let Ok(path) = std::env::var("CEF_PATH") {
        let p = std::path::PathBuf::from(&path);
        if p.join("libcef.so").exists() {
            return p;
        }
    }

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let exe_dir = exe_path.parent().unwrap();

    // 2. Alongside the binary (installed layout)
    if exe_dir.join("libcef.so").exists() {
        return exe_dir.to_path_buf();
    }

    // 3. Development: scan target/debug/build/cef-dll-sys-*/out/cef_linux_*
    let build_dir = exe_dir.join("build");
    if build_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&build_dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_str().is_some_and(|n| n.starts_with("cef-dll-sys-")) {
                    let out_dir = entry.path().join("out");
                    if let Ok(out_entries) = std::fs::read_dir(&out_dir) {
                        for out_entry in out_entries.flatten() {
                            let candidate = out_entry.path();
                            if candidate.join("libcef.so").exists() {
                                return candidate;
                            }
                        }
                    }
                }
            }
        }
    }

    exe_dir.to_path_buf()
}

/// Start the CEF message pump integration with GTK4's main loop.
/// Call this after GTK4's main loop is running.
pub fn start_message_pump() {
    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        do_message_loop_work();
        gtk4::glib::ControlFlow::Continue
    });
}

/// Shut down CEF. Call after GTK4's main loop has exited.
pub fn shutdown() {
    cef::shutdown();
}

/// Holds CEF state that must be kept alive.
pub struct CefState {
    _app: App,
}

// -- CEF App implementation --

#[derive(Clone)]
struct TurmApp;

wrap_app! {
    struct TurmAppBuilder {
        app: TurmApp,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefStringUtf16>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(command_line) = command_line else {
                return;
            };
            command_line.append_switch(Some(&"no-startup-window".into()));
            command_line.append_switch(Some(&"noerrdialogs".into()));
            command_line.append_switch(Some(&"disable-gpu".into()));
            command_line.append_switch(Some(&"disable-gpu-compositing".into()));
            command_line.append_switch(Some(&"enable-logging=stderr".into()));
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(TurmBrowserProcessHandlerBuilder::build(
                TurmBrowserProcessHandler {
                    ready: RefCell::new(false),
                },
            ))
        }
    }
}

impl TurmAppBuilder {
    fn build(app: TurmApp) -> App {
        Self::new(app)
    }
}

#[derive(Clone)]
struct TurmBrowserProcessHandler {
    ready: RefCell<bool>,
}

wrap_browser_process_handler! {
    struct TurmBrowserProcessHandlerBuilder {
        handler: TurmBrowserProcessHandler,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            *self.handler.ready.borrow_mut() = true;
            eprintln!("[cef] Context initialized");
        }
    }
}

impl TurmBrowserProcessHandlerBuilder {
    fn build(handler: TurmBrowserProcessHandler) -> BrowserProcessHandler {
        Self::new(handler)
    }
}
