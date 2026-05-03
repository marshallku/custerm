import CTurmFFI
import Foundation

/// Thin Swift facade over the turm-ffi C-ABI staticlib.
///
/// PR 1 (Tier 2.1 spike) scope: prove the build/link path end-to-end with the
/// smallest possible call surface. PR 2+ (registry seam, supervisor, trigger
/// engine) will grow this file or split it per concern; for now everything
/// the Rust side exposes lands here so Swift callers don't poke at C pointers
/// directly.
///
/// Memory model:
/// - `version()` returns a Swift String copy; no Rust allocation involved.
/// - `callJSON(_:)` round-trips JSON. The C call returns a heap-allocated C
///   string that this facade copies into a Swift String and immediately
///   frees via `turm_ffi_free_string`. Callers never see a raw pointer.
/// - `lastError()` reads a thread-local slot in Rust. The string is borrowed
///   from Rust and copied to Swift on read; the Rust slot stays owned by Rust.
enum TurmFFI {
    /// Static version string baked into the Rust crate. Cheap, no allocation.
    static func version() -> String {
        // turm_ffi_version() returns a static C string we don't own.
        guard let cstr = turm_ffi_version() else { return "<null>" }
        return String(cString: cstr)
    }

    /// Round-trip a JSON document through the Rust crate. The Rust side adds
    /// an `echoed_at` field with a unix-epoch-millis timestamp so callers can
    /// confirm the value came from Rust (not a Swift-side passthrough).
    ///
    /// On success returns the parsed JSON object.
    /// On failure returns nil and writes the FFI error to stderr.
    static func callJSON(_ payload: [String: Any]) -> [String: Any]? {
        guard let inputData = try? JSONSerialization.data(withJSONObject: payload),
              let inputStr = String(data: inputData, encoding: .utf8)
        else {
            FileHandle.standardError.write(Data("[turm-ffi] failed to serialize input JSON\n".utf8))
            return nil
        }

        return inputStr.withCString { (cstr: UnsafePointer<CChar>) -> [String: Any]? in
            guard let resultPtr = turm_ffi_call_json(cstr) else {
                let err = lastError() ?? "<no error message>"
                FileHandle.standardError.write(Data("[turm-ffi] call_json returned NULL: \(err)\n".utf8))
                return nil
            }
            // Important: copy to Swift String first, then free. If we passed
            // resultPtr to JSONSerialization directly via a Data wrapper we'd
            // have to manage the lifetime around the JSON parse, and Swift's
            // Data(bytesNoCopy:) is easy to misuse. Copy-then-free is boring
            // and obviously correct.
            defer { turm_ffi_free_string(resultPtr) }
            let resultStr = String(cString: resultPtr)
            guard let resultData = resultStr.data(using: .utf8),
                  let parsed = try? JSONSerialization.jsonObject(with: resultData) as? [String: Any]
            else {
                FileHandle.standardError.write(Data("[turm-ffi] failed to parse FFI response: \(resultStr)\n".utf8))
                return nil
            }
            return parsed
        }
    }

    /// Most recent error from the calling thread, copied into Swift.
    /// Returns nil if no error has been recorded since the last successful call.
    static func lastError() -> String? {
        guard let cstr = turm_ffi_last_error() else { return nil }
        return String(cString: cstr)
    }

    /// Smoke-test the FFI bridge. Called once during app launch (PR 1 spike)
    /// to confirm the staticlib linked and basic round-trip works. Logs to
    /// stderr; doesn't crash the app on failure (the FFI is non-load-bearing
    /// at this stage — Tier 2.4 will replace this with real engine startup).
    static func runSmokeTest() {
        FileHandle.standardError.write(Data("[turm-ffi] version = \(version())\n".utf8))
        if let echo = callJSON(["hello": "from swift", "spike": true]) {
            FileHandle.standardError.write(Data("[turm-ffi] echo round-trip = \(echo)\n".utf8))
        }
    }
}
