import CNesttyTerm
import Foundation

/// Thin Swift facade over the nestty-term C-ABI staticlib.
///
/// Phase 1 scaffold (see docs/macos-renderer-migration-plan.md): the
/// underlying Rust crate is dependency-free and returns fixture data.
/// Phase 2 will swap in `alacritty_terminal::Term` + a real PTY.
///
/// Memory model:
///
/// - `nestty_term_create` returns an opaque handle pointer that this
///   facade boxes into a `Handle` class. The class's `deinit` calls
///   `nestty_term_destroy` exactly once.
/// - `snapshot()` returns a `Snapshot` class wrapping the opaque
///   snapshot pointer. Its `deinit` calls `nestty_snapshot_destroy`.
/// - `Snapshot.rowRuns(_:)` and `Snapshot.rowUtf8(_:)` hand back
///   borrowed pointers valid until the `Snapshot` is deallocated.
///   Callers must finish using them before the Snapshot goes out of
///   scope (RAII via Swift's ARC).
enum NesttyTermFFI {
    /// Static version string. No allocation.
    static func version() -> String {
        guard let cstr = nestty_term_version() else { return "<null>" }
        return String(cString: cstr)
    }

    /// Smoke-test the bridge end-to-end: create handle, take snapshot,
    /// walk the fixture row, destroy in reverse order. Asserts the
    /// known fixture shape. Called once during app launch alongside
    /// `NesttyFFI.runSmokeTest()`. `@MainActor` because the `Handle`
    /// + `Snapshot` classes are main-actor-isolated.
    @MainActor
    static func runSmokeTest() {
        FileHandle.standardError.write(Data("[nestty-term] version = \(version())\n".utf8))
        guard let handle = Handle(cols: 80, rows: 24) else {
            FileHandle.standardError.write(Data("[nestty-term] smoke FAIL: handle create returned NULL\n".utf8))
            return
        }
        guard let snap = handle.snapshot() else {
            FileHandle.standardError.write(Data("[nestty-term] smoke FAIL: snapshot returned NULL\n".utf8))
            return
        }
        let rows = snap.rows
        let cols = snap.cols
        let runs = snap.rowRuns(0)
        let utf8 = snap.rowUtf8(0)
        let cursor = snap.cursor
        FileHandle.standardError.write(Data("[nestty-term] snapshot \(cols)x\(rows), row0: \(runs.count) runs / \(utf8.count) utf8 bytes, cursor at (\(cursor.row),\(cursor.col)) style=\(cursor.style)\n".utf8))
    }

    @MainActor
    final class Handle {
        /// `nonisolated(unsafe)` so deinit (Swift 6 nonisolated) can
        /// free without crossing the actor boundary. Rust destroy has
        /// no thread-affinity.
        private nonisolated(unsafe) var ptr: OpaquePointer?

        init?(cols: UInt16, rows: UInt16, shell: String? = nil, cwd: String? = nil) {
            let p = shell.withCStringOrNull { shellPtr in
                cwd.withCStringOrNull { cwdPtr in
                    nestty_term_create(cols, rows, shellPtr, cwdPtr)
                }
            }
            guard let p else { return nil }
            ptr = p
        }

        deinit {
            if let p = ptr {
                nestty_term_destroy(p)
            }
        }

        func resize(cols: UInt16, rows: UInt16) {
            guard let ptr else { return }
            nestty_term_resize(ptr, cols, rows)
        }

        func input(_ bytes: [UInt8]) {
            guard let ptr else { return }
            bytes.withUnsafeBufferPointer { buf in
                nestty_term_input(ptr, buf.baseAddress, buf.count)
            }
        }

        func snapshot() -> Snapshot? {
            guard let ptr, let snap = nestty_term_snapshot(ptr) else { return nil }
            return Snapshot(ptr: snap)
        }

        /// True if the terminal grid has any damage since the previous
        /// call (and resets the internal damage state). False means
        /// nothing changed — renderer can skip the snapshot + draw.
        func takeDamage() -> Bool {
            guard let ptr else { return false }
            return nestty_term_take_damage(ptr)
        }
    }

    /// Wraps a Rust-owned snapshot pointer. Borrowed runs/utf8 are
    /// valid for the lifetime of this Snapshot object.
    @MainActor
    final class Snapshot {
        private nonisolated(unsafe) var ptr: OpaquePointer?

        fileprivate init(ptr: OpaquePointer) {
            self.ptr = ptr
        }

        deinit {
            if let p = ptr {
                nestty_snapshot_destroy(p)
            }
        }

        var rows: UInt16 {
            guard let ptr else { return 0 }
            return nestty_snapshot_rows(ptr)
        }

        var cols: UInt16 {
            guard let ptr else { return 0 }
            return nestty_snapshot_cols(ptr)
        }

        /// Borrowed view into the row's run array. Buffer is valid
        /// only while this Snapshot is alive; copy out anything you
        /// need to keep.
        func rowRuns(_ row: UInt16) -> UnsafeBufferPointer<NesttyRun> {
            guard let ptr else { return UnsafeBufferPointer(start: nil, count: 0) }
            var runsPtr: UnsafePointer<NesttyRun>?
            let n = nestty_snapshot_row_runs(ptr, row, &runsPtr)
            return UnsafeBufferPointer(start: runsPtr, count: n)
        }

        /// Borrowed view into the row's utf8 bytes. Same lifetime.
        func rowUtf8(_ row: UInt16) -> UnsafeBufferPointer<UInt8> {
            guard let ptr else { return UnsafeBufferPointer(start: nil, count: 0) }
            var len = 0
            let bytesPtr = nestty_snapshot_row_utf8(ptr, row, &len)
            return UnsafeBufferPointer(start: bytesPtr, count: len)
        }

        var cursor: NesttyCursor {
            guard let ptr else {
                return NesttyCursor(row: 0, col: 0, style: 0, blink: 0, reserved: 0)
            }
            var out = NesttyCursor(row: 0, col: 0, style: 0, blink: 0, reserved: 0)
            nestty_snapshot_cursor(ptr, &out)
            return out
        }
    }
}

private extension String? {
    /// Run `body` with a borrowed C string (or NULL if self is nil).
    /// Used for optional `shell` / `cwd` parameters.
    func withCStringOrNull<R>(_ body: (UnsafePointer<CChar>?) -> R) -> R {
        switch self {
        case .none: body(nil)
        case let .some(str): str.withCString { body($0) }
        }
    }
}
