// swift-tools-version: 6.0
import PackageDescription

// MARK: - nestty-ffi linkage

//
// The Nestty executable links a Rust staticlib (`libnestty_ffi.a`) produced by the
// nestty-ffi crate at the workspace root. SwiftPM has no first-class way to
// invoke cargo as a prebuild step from this manifest shape, so the build
// pipeline is split:
//
//   1. `cargo build --release -p nestty-ffi`   → workspace_root/target/release/libnestty_ffi.a
//   2. `swift build`                          → links libnestty_ffi.a via the linker flags below
//
// scripts/install-macos.sh + nestty-macos/run.sh wrap both steps. Running
// `swift build` alone after a clean target/ directory will fail with an
// undefined-symbol link error — the build script is the source of truth.
//
// The `-L../target/release` is a relative path interpreted at link time from
// the package root (`nestty-macos/`), resolving to the cargo workspace target
// directory. `linkedLibrary("nestty_ffi")` adds `-lnestty_ffi` to find the
// staticlib by its base name.

let package = Package(
    name: "nestty-macos",
    platforms: [
        .macOS(.v14),
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm", from: "1.2.0"),
        .package(url: "https://github.com/LebJe/TOMLKit", from: "0.6.0"),
    ],
    targets: [
        // C wrapper that exposes nestty-ffi's C symbols to Swift via a clang
        // module. The header + module.modulemap live under include/, the
        // dummy.c forces SwiftPM to actually emit a target object so the
        // linker settings flow through to the final executable.
        .target(
            name: "CNesttyFFI",
            path: "Sources/CNesttyFFI",
            publicHeadersPath: "include",
        ),
        .executableTarget(
            name: "Nestty",
            dependencies: [
                .product(name: "SwiftTerm", package: "SwiftTerm"),
                .product(name: "TOMLKit", package: "TOMLKit"),
                "CNesttyFFI",
            ],
            path: "Sources/Nestty",
            linkerSettings: [
                .unsafeFlags(["-L../target/release"]),
                .linkedLibrary("nestty_ffi"),
            ],
        ),
    ],
)
