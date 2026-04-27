use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, gio};

use crate::window::TurmWindow;

const APP_ID: &str = "com.marshall.turm";

pub fn run() {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|_| {
        if let Some(settings) = gtk4::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }
    });

    app.connect_activate(|app| {
        let config = turm_core::config::TurmConfig::load().unwrap_or_default();
        let window = TurmWindow::new(app, &config);
        window.present();

        // Ctrl-C in the foreground or `kill <pid>` from another shell
        // would otherwise kill the GTK process *without* running the
        // window's `connect_destroy` callback, leaving the plugin
        // subprocesses orphaned to init. Handle SIGTERM and SIGINT
        // by closing all windows — that fires connect_destroy →
        // ServiceSupervisor::shutdown_all() through the existing
        // graceful path. PR_SET_PDEATHSIG covers the SIGKILL /
        // segfault case where we never get a chance to run user code.
        let signal_app = app.downgrade();
        glib::unix_signal_add_local(libc::SIGTERM, move || {
            if let Some(app) = signal_app.upgrade() {
                eprintln!("[turm] SIGTERM received — closing windows for graceful shutdown");
                close_all_windows(&app);
            }
            glib::ControlFlow::Continue
        });
        let signal_app = app.downgrade();
        glib::unix_signal_add_local(libc::SIGINT, move || {
            if let Some(app) = signal_app.upgrade() {
                eprintln!("[turm] SIGINT received — closing windows for graceful shutdown");
                close_all_windows(&app);
            }
            glib::ControlFlow::Continue
        });
    });

    app.run();
}

/// Trigger window destroy on every window the application owns.
/// Prefers `window.close()` (dispatches the standard delete-event +
/// destroy chain) over `app.quit()` because the latter exits the
/// main loop without giving widgets a chance to fire their destroy
/// signals — and the supervisor's `shutdown_all` is wired to the
/// window's destroy signal.
fn close_all_windows(app: &Application) {
    for w in app.windows() {
        w.close();
    }
}
