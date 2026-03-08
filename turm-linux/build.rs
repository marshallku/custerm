fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();

    // OUT_DIR is like: target/debug/build/turm-linux-<hash>/out
    // CEF libs are in: target/debug/build/cef-dll-sys-<hash>/out/cef_linux_x86_64
    // Navigate up to the build/ directory and scan for cef-dll-sys output.
    let build_dir = std::path::Path::new(&out_dir)
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
        .expect("Could not find build/ directory in OUT_DIR");

    let cef_dir = std::fs::read_dir(build_dir)
        .expect("Failed to read build directory")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("cef-dll-sys-"))
        })
        .find_map(|e| {
            let candidate = e.path().join("out/cef_linux_x86_64");
            if candidate.join("libcef.so").exists() {
                Some(candidate)
            } else {
                None
            }
        })
        .expect("Could not find CEF directory in build output");

    let cef_dir = cef_dir.display();
    println!("cargo::rustc-link-arg=-Wl,-rpath,{cef_dir}");

    // CEF requires the host binary to export symbols (e.g. libc's `close`)
    // so that libcef.so can dlsym them. Without this, sub-processes crash
    // with "close symbol missing".
    println!("cargo::rustc-link-arg=-Wl,--export-dynamic");
}
