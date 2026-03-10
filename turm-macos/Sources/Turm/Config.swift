import Foundation

struct TurmConfig {
    let shell: String
    let fontFamily: String
    let fontSize: Int
    let themeName: String

    static func load() -> TurmConfig {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let configURL = home
            .appendingPathComponent(".config")
            .appendingPathComponent("turm")
            .appendingPathComponent("config.toml")

        guard let contents = try? String(contentsOf: configURL, encoding: .utf8) else {
            return TurmConfig.defaults
        }

        return TurmConfig.parse(contents)
    }

    static func parse(_ contents: String) -> TurmConfig {
        var shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        var fontFamily = "JetBrainsMono Nerd Font Mono"
        var fontSize = 14
        var themeName = "catppuccin-mocha"

        var currentSection = ""

        for line in contents.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed.isEmpty || trimmed.hasPrefix("#") { continue }

            if trimmed.hasPrefix("["), trimmed.hasSuffix("]") {
                currentSection = String(trimmed.dropFirst().dropLast())
                continue
            }

            guard let eqRange = trimmed.range(of: "=") else { continue }
            let key = trimmed[..<eqRange.lowerBound].trimmingCharacters(in: .whitespaces)
            var value = String(trimmed[eqRange.upperBound...]).trimmingCharacters(in: .whitespaces)

            // Strip inline comments
            if let commentRange = value.range(of: " #") {
                value = String(value[..<commentRange.lowerBound]).trimmingCharacters(in: .whitespaces)
            }

            // Strip surrounding quotes
            if value.hasPrefix("\""), value.hasSuffix("\""), value.count >= 2 {
                value = String(value.dropFirst().dropLast())
            }

            switch (currentSection, key) {
            case ("terminal", "shell"):
                shell = value
            case ("terminal", "font_family"):
                fontFamily = value
            case ("terminal", "font_size"):
                if let n = Int(value) { fontSize = n }
            case ("theme", "name"):
                themeName = value
            default:
                break
            }
        }

        return TurmConfig(shell: shell, fontFamily: fontFamily, fontSize: fontSize, themeName: themeName)
    }

    static var defaults: TurmConfig {
        TurmConfig(
            shell: ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh",
            fontFamily: "JetBrainsMono Nerd Font Mono",
            fontSize: 14,
            themeName: "catppuccin-mocha",
        )
    }
}
