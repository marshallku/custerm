use cef::*;
use std::cell::RefCell;

/// Initialize CEF for the browser process.
/// Sub-processes are handled by the separate `turm-cef-helper` binary.
pub fn initialize() -> CefState {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    let mut app = TurmAppBuilder::build(TurmApp);

    // For the browser process, execute_process returns -1.
    let ret = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    assert_eq!(ret, -1, "Expected browser process, got sub-process");

    // Resolve CEF resource directories and subprocess helper
    let cef_dir = find_cef_dir();
    let resources_path = CefString::from(cef_dir.to_str().unwrap());
    let locales_path = CefString::from(cef_dir.join("locales").to_str().unwrap());

    // Use a separate lightweight helper binary for CEF sub-processes to avoid
    // GTK/VTE library conflicts with CEF's bundled libraries.
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let helper_path = exe_path.with_file_name("turm-cef-helper");
    let subprocess_path = if helper_path.exists() {
        CefString::from(helper_path.to_str().unwrap())
    } else {
        eprintln!("[cef] warning: turm-cef-helper not found, using self as subprocess");
        CefString::from(exe_path.to_str().unwrap())
    };

    eprintln!("[cef] resources: {}", cef_dir.display());

    let settings = Settings {
        windowless_rendering_enabled: true as _,
        external_message_pump: true as _,
        browser_subprocess_path: subprocess_path,
        ..Default::default()
    };

    // CEF's initialize() with external_message_pump may need message pump work
    // to be called concurrently to complete. Pump from a background thread.
    let pump_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let pump_flag_clone = pump_flag.clone();
    let pump_thread = std::thread::spawn(move || {
        while pump_flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
            do_message_loop_work();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    eprintln!("[cef] calling cef::initialize...");
    let result = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    eprintln!("[cef] cef::initialize returned: {result}");

    pump_flag.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = pump_thread.join();

    assert_eq!(result, 1, "CEF initialization failed");

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
