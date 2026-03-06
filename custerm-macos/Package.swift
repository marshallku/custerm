// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "custerm-macos",
    platforms: [
        .macOS(.v14),
    ],
    targets: [
        .executableTarget(
            name: "Custerm",
            path: "Sources/Custerm"
        ),
    ]
)
