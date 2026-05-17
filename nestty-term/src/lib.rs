//! C-ABI bridge from a future `alacritty_terminal::Term` to
//! `nestty-macos`'s renderer (and eventually any other host that needs
//! a fully-functional terminal emulator core).
//!
//! Phase 1 ships the FFI surface from
//! `docs/macos-renderer-migration-plan.md` §D3 with stubbed bodies —
//! a fixture row matching the Phase 0 spike's coverage of the
//! interesting render cases (red text, inverse, wide CJK, ZWJ emoji,
//! ligature input with underline color override). Phase 2 will
//! replace the fixture data with `alacritty_terminal::Term` snapshots
//! sourced from a real PTY.
//!
//! All pointers returned across the boundary follow these rules:
//!
//! - `*mut NesttyHandle` / `*mut NesttySnapshot` — heap allocations
//!   owned by Rust. Must be freed by their matching `_destroy`
//!   function exactly once. Passing NULL to `_destroy` is a no-op.
//! - Borrowed `*const NesttyRun` / `*const u8` from snapshot
//!   accessors — valid until `nestty_snapshot_destroy`. The Swift
//!   caller must not retain these past the snapshot's lifetime.
//! - Static strings (`nestty_term_version`) — valid for program
//!   lifetime, no free required.

use std::ffi::{CStr, c_char};
use std::ptr;

/// Run-oriented cell attribute block. `#[repr(C)]` so Swift sees the
/// same layout. Per-cell allocation is avoided by indexing into the
/// row's contiguous utf8 buffer via `utf8_offset` + `utf8_len`.
///
/// Layout MUST match `nestty-macos/Sources/CNesttyTerm/include/nestty_term.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NesttyRun {
    pub start_col: u16,
    pub end_col: u16,
    pub utf8_offset: u32,
    pub utf8_len: u32,
    pub fg_rgba: u32,
    pub bg_rgba: u32, // sentinel 0 = default-bg
    pub flags: u16,
    pub underline_style: u8,
    pub reserved: u8,
    pub underline_color_rgba: u32,
    pub hyperlink_id: u32,
}

pub mod flags {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const UNDERLINE: u16 = 1 << 2;
    pub const INVERSE: u16 = 1 << 3;
    pub const DIM: u16 = 1 << 4;
    pub const STRIKE: u16 = 1 << 5;
    pub const BLINK: u16 = 1 << 6;
    pub const WIDE_LEADING: u16 = 1 << 7;
    pub const WIDE_TRAILING: u16 = 1 << 8;
}

/// Cursor position + style. Reported via `nestty_snapshot_cursor`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NesttyCursor {
    pub row: u16,
    pub col: u16,
    /// 0=hidden 1=block 2=bar 3=underline (steady/blink encoded in `blink`)
    pub style: u8,
    pub blink: u8,
    pub _reserved: u16,
}

struct Row {
    utf8: Vec<u8>,
    runs: Vec<NesttyRun>,
}

/// Phase 1 stub. Holds geometry and a fixture-data flag for the
/// initial smoke test; Phase 2 will replace this with a wrapper
/// around `alacritty_terminal::Term` + PTY thread.
pub struct NesttyHandle {
    cols: u16,
    rows: u16,
}

pub struct NesttySnapshot {
    cols: u16,
    rows: Vec<Row>,
    cursor: NesttyCursor,
}

/// Create a terminal handle. Phase 1 stub — `shell` and `cwd` are
/// accepted for API compatibility but unused (no PTY spawn yet).
///
/// # Safety
///
/// `shell` and `cwd` may be NULL or valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_term_create(
    cols: u16,
    rows: u16,
    _shell: *const c_char,
    _cwd: *const c_char,
) -> *mut NesttyHandle {
    Box::into_raw(Box::new(NesttyHandle { cols, rows }))
}

/// Free a handle. Safe to pass NULL.
///
/// # Safety
///
/// Must be called exactly once per handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_term_destroy(handle: *mut NesttyHandle) {
    if handle.is_null() {
        return;
    }
    let _ = unsafe { Box::from_raw(handle) };
}

/// Phase 1 stub — accepts input bytes but doesn't route to a PTY yet.
///
/// # Safety
///
/// `bytes` must point to `len` readable bytes (or be NULL when len=0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_term_input(_handle: *mut NesttyHandle, _bytes: *const u8, _len: usize) {
}

/// Phase 1 stub — Phase 2 will resize the PTY + Term grid.
#[unsafe(no_mangle)]
pub extern "C" fn nestty_term_resize(handle: *mut NesttyHandle, cols: u16, rows: u16) {
    if let Some(h) = unsafe { handle.as_mut() } {
        h.cols = cols;
        h.rows = rows;
    }
}

/// Take a snapshot of the current terminal state. Phase 1 returns a
/// fixture row matching the spike (red R, inverse I, wide CJK 한,
/// ZWJ family emoji, ligature `fi != !=` with pink underline), plus
/// `rows-1` empty rows. Cursor sits at the end of the fixture
/// content. Phase 2 will source this from `Term::grid()`.
#[unsafe(no_mangle)]
pub extern "C" fn nestty_term_snapshot(handle: *mut NesttyHandle) -> *mut NesttySnapshot {
    let Some(h) = (unsafe { handle.as_ref() }) else {
        return ptr::null_mut();
    };

    let fixture_utf8: Vec<u8> = b"RI\xed\x95\x9c\xf0\x9f\x91\xa8\xe2\x80\x8d\xf0\x9f\x91\xa9\xe2\x80\x8d\xf0\x9f\x91\xa7fi != !=".to_vec();
    let fixture_runs = vec![
        NesttyRun {
            start_col: 0,
            end_col: 1,
            utf8_offset: 0,
            utf8_len: 1,
            fg_rgba: 0xff5555ff,
            bg_rgba: 0,
            flags: 0,
            underline_style: 0,
            reserved: 0,
            underline_color_rgba: 0,
            hyperlink_id: 0,
        },
        NesttyRun {
            start_col: 1,
            end_col: 2,
            utf8_offset: 1,
            utf8_len: 1,
            fg_rgba: 0xffffffff,
            bg_rgba: 0,
            flags: flags::INVERSE,
            underline_style: 0,
            reserved: 0,
            underline_color_rgba: 0,
            hyperlink_id: 0,
        },
        NesttyRun {
            start_col: 2,
            end_col: 4,
            utf8_offset: 2,
            utf8_len: 3,
            fg_rgba: 0x55ffffff,
            bg_rgba: 0,
            flags: flags::WIDE_LEADING,
            underline_style: 0,
            reserved: 0,
            underline_color_rgba: 0,
            hyperlink_id: 0,
        },
        NesttyRun {
            start_col: 4,
            end_col: 6,
            utf8_offset: 5,
            utf8_len: 18,
            fg_rgba: 0xffffffff,
            bg_rgba: 0,
            flags: flags::WIDE_LEADING,
            underline_style: 0,
            reserved: 0,
            underline_color_rgba: 0,
            hyperlink_id: 0,
        },
        NesttyRun {
            start_col: 6,
            end_col: 14,
            utf8_offset: 23,
            utf8_len: (fixture_utf8.len() - 23) as u32,
            fg_rgba: 0xeeeeeeff,
            bg_rgba: 0,
            flags: 0,
            underline_style: 1,
            reserved: 0,
            underline_color_rgba: 0xff55aaff,
            hyperlink_id: 0,
        },
    ];

    let mut rows = Vec::with_capacity(h.rows as usize);
    rows.push(Row {
        utf8: fixture_utf8,
        runs: fixture_runs,
    });
    for _ in 1..h.rows {
        rows.push(Row {
            utf8: Vec::new(),
            runs: Vec::new(),
        });
    }

    Box::into_raw(Box::new(NesttySnapshot {
        cols: h.cols,
        rows,
        cursor: NesttyCursor {
            row: 0,
            col: 14,
            style: 1, // block
            blink: 0,
            _reserved: 0,
        },
    }))
}

/// Free a snapshot. Safe to pass NULL. Calling twice is UB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_snapshot_destroy(snap: *mut NesttySnapshot) {
    if snap.is_null() {
        return;
    }
    let _ = unsafe { Box::from_raw(snap) };
}

#[unsafe(no_mangle)]
pub extern "C" fn nestty_snapshot_rows(snap: *const NesttySnapshot) -> u16 {
    let Some(s) = (unsafe { snap.as_ref() }) else { return 0 };
    s.rows.len() as u16
}

#[unsafe(no_mangle)]
pub extern "C" fn nestty_snapshot_cols(snap: *const NesttySnapshot) -> u16 {
    let Some(s) = (unsafe { snap.as_ref() }) else { return 0 };
    s.cols
}

/// Hand the caller a borrowed pointer to the row's run array.
/// Pointer valid until `nestty_snapshot_destroy`. Returns 0 if row is
/// out of range; `*out_runs` is set to NULL in that case.
///
/// # Safety
///
/// `out_runs` must point to writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_snapshot_row_runs(
    snap: *const NesttySnapshot,
    row: u16,
    out_runs: *mut *const NesttyRun,
) -> usize {
    if out_runs.is_null() {
        return 0;
    }
    let Some(s) = (unsafe { snap.as_ref() }) else {
        unsafe { *out_runs = ptr::null() };
        return 0;
    };
    let Some(row_data) = s.rows.get(row as usize) else {
        unsafe { *out_runs = ptr::null() };
        return 0;
    };
    unsafe { *out_runs = row_data.runs.as_ptr() };
    row_data.runs.len()
}

/// Borrowed pointer to the row's utf8 bytes + length. Same lifetime
/// contract as `nestty_snapshot_row_runs`. Returns NULL with
/// `*out_len = 0` on out-of-range row.
///
/// # Safety
///
/// `out_len` must point to writable storage for one `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_snapshot_row_utf8(
    snap: *const NesttySnapshot,
    row: u16,
    out_len: *mut usize,
) -> *const u8 {
    if out_len.is_null() {
        return ptr::null();
    }
    let Some(s) = (unsafe { snap.as_ref() }) else {
        unsafe { *out_len = 0 };
        return ptr::null();
    };
    match s.rows.get(row as usize) {
        Some(row_data) => {
            unsafe { *out_len = row_data.utf8.len() };
            row_data.utf8.as_ptr()
        }
        None => {
            unsafe { *out_len = 0 };
            ptr::null()
        }
    }
}

/// Fill `*out` with the snapshot's cursor state. No-op if snap or out
/// is NULL.
///
/// # Safety
///
/// `out` must point to writable storage for one `NesttyCursor`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nestty_snapshot_cursor(snap: *const NesttySnapshot, out: *mut NesttyCursor) {
    if out.is_null() {
        return;
    }
    let Some(s) = (unsafe { snap.as_ref() }) else { return };
    unsafe { *out = s.cursor };
}

/// Static version + phase tag so a Swift host can verify which
/// scaffold level it's linked against.
#[unsafe(no_mangle)]
pub extern "C" fn nestty_term_version() -> *const c_char {
    static VERSION: &CStr = c"nestty-term 0.1.0 (Phase 1 scaffold, fixture data only)";
    VERSION.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_create_destroy_round_trip() {
        unsafe {
            let h = nestty_term_create(80, 24, std::ptr::null(), std::ptr::null());
            assert!(!h.is_null());
            nestty_term_destroy(h);
        }
    }

    #[test]
    fn snapshot_exposes_fixture_row() {
        unsafe {
            let h = nestty_term_create(80, 24, std::ptr::null(), std::ptr::null());
            let snap = nestty_term_snapshot(h);
            assert!(!snap.is_null());

            assert_eq!(nestty_snapshot_rows(snap), 24);
            assert_eq!(nestty_snapshot_cols(snap), 80);

            let mut runs_ptr: *const NesttyRun = std::ptr::null();
            let n = nestty_snapshot_row_runs(snap, 0, &mut runs_ptr);
            assert_eq!(n, 5);
            let runs = std::slice::from_raw_parts(runs_ptr, n);
            assert_eq!(runs[0].start_col, 0);
            assert_eq!(runs[1].flags & flags::INVERSE, flags::INVERSE);
            assert_eq!(runs[2].end_col, 4); // wide CJK spans 2 cols
            assert_eq!(runs[4].underline_style, 1);

            let mut utf8_len: usize = 0;
            let utf8_ptr = nestty_snapshot_row_utf8(snap, 0, &mut utf8_len);
            assert!(!utf8_ptr.is_null());
            assert!(utf8_len > 0);
            let bytes = std::slice::from_raw_parts(utf8_ptr, utf8_len);
            assert_eq!(bytes[0], b'R');
            assert_eq!(bytes[1], b'I');

            let mut cur = NesttyCursor {
                row: 99,
                col: 99,
                style: 99,
                blink: 99,
                _reserved: 0,
            };
            nestty_snapshot_cursor(snap, &mut cur);
            assert_eq!(cur.col, 14);
            assert_eq!(cur.style, 1);

            nestty_snapshot_destroy(snap);
            nestty_term_destroy(h);
        }
    }

    #[test]
    fn null_destroy_no_op() {
        unsafe {
            nestty_term_destroy(std::ptr::null_mut());
            nestty_snapshot_destroy(std::ptr::null_mut());
        }
    }
}
