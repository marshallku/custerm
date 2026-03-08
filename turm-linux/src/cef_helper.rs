//! Minimal CEF subprocess helper binary.
//! CEF spawns renderer/GPU/utility processes by re-launching a binary with --type=...
//! This lightweight helper avoids loading GTK/VTE libraries that conflict with CEF.

fn main() {
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let args = cef::args::Args::new();
    let ret = cef::execute_process(
        Some(args.as_main_args()),
        None::<&mut cef::App>,
        std::ptr::null_mut(),
    );
    std::process::exit(ret);
}
