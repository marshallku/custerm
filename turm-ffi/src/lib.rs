//! turm-ffi — C-ABI bridge from the shared Rust core to platform UIs that
//! can't link Rust directly (currently `turm-macos`, which is SwiftPM).
//!
//! ## Why this crate exists (PR 1 — Tier 2.1 spike)
//!
//! Before committing to wiring `TriggerEngine` / `ActionRegistry` / supervisor
//! over FFI, we need to prove the boring boundary first: cargo can produce a
//! staticlib, SwiftPM links it, a Swift call lands in Rust, JSON crosses the
//! boundary in both directions, and ownership rules don't leak. This spike
//! exposes the **smallest possible C surface** that demonstrates each of those
//! concerns in isolation, so when something breaks we know whether it's the
//! build wiring, the link, the calling convention, or the data marshalling.
//!
//! Surface (4 symbols):
//!
//! - `turm_ffi_version() -> *const c_char` — points at a static `'static`
//!   string. Caller must NOT free. Proves: lib loads, basic call works,
//!   pointer-back-to-Rust-static is sound, no allocation involved.
//!
//! - `turm_ffi_call_json(input: *const c_char) -> *mut c_char` — accepts a
//!   borrowed JSON string, parses it, attaches `{"echoed_at": <unix epoch
//!   ms>}`, and returns a heap-allocated JSON string the caller owns. Proves:
//!   bidirectional JSON marshalling works, Rust-side allocation that the
//!   Swift side later releases works, error paths produce structured errors
//!   instead of panicking across FFI.
//!
//! - `turm_ffi_free_string(*mut c_char)` — releases a string previously
//!   returned by this crate. Required because Swift's ARC can't free Rust's
//!   heap. Proves: ownership round-trip closes cleanly without leaks.
//!
//! - `turm_ffi_last_error() -> *const c_char` — returns the most recent
//!   error message captured by this thread (or NULL if none). Proves:
//!   thread-local error reporting works without a return-by-pointer pattern
//!   that Swift would have to construct C buffers for.
//!
//! Anything beyond this set is for follow-up PRs (PR 2+: registry seam, then
//! supervisor, then trigger engine). Keep this file boring on purpose.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

thread_local! {
    /// Per-thread last-error slot. Cleared by every successful FFI call.
    /// Threading model note: every entry point writes to this slot before
    /// returning either an error or a success value, so a Swift caller that
    /// got NULL/error can pick up the message without needing a side channel.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error<S: Into<String>>(message: S) {
    let cs = CString::new(message.into()).unwrap_or_else(|_| {
        // Fallback for the (impossible) case where the message contains an
        // interior NUL. Don't lose the failure signal entirely.
        CString::new("FFI error message contained a NUL byte").unwrap()
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(cs));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Returns a pointer to a static, NUL-terminated version string. Caller must
/// NOT free. The string lives for the program's lifetime.
///
/// # Safety
///
/// The returned pointer is always non-null and valid for as long as the
/// process lives. Reading past the NUL terminator is UB.
#[unsafe(no_mangle)]
pub extern "C" fn turm_ffi_version() -> *const c_char {
    // Static C string, no allocation. `c"..."` literal is a Rust 2021+ feature
    // that produces a `&'static CStr`, so .as_ptr() is good for the program
    // lifetime.
    c"turm-ffi 0.1.0".as_ptr()
}

/// Accepts a borrowed JSON string and returns a heap-allocated JSON string
/// that the caller MUST release with `turm_ffi_free_string`.
///
/// On the success path the returned JSON contains the input plus an
/// `echoed_at` Unix-epoch-millis field, so a Swift caller can prove the
/// round-trip with a value that's both Rust-generated AND not constant.
///
/// On the error path returns NULL and stores a human-readable message in
/// `LAST_ERROR` retrievable via `turm_ffi_last_error`.
///
/// # Safety
///
/// `input` must be a valid pointer to a NUL-terminated UTF-8 string. The
/// pointer must remain valid for the duration of this call. The returned
/// pointer (when non-null) must be passed to `turm_ffi_free_string` exactly
/// once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_ffi_call_json(input: *const c_char) -> *mut c_char {
    if input.is_null() {
        set_last_error("turm_ffi_call_json: input pointer is NULL");
        return ptr::null_mut();
    }

    // SAFETY: caller contract requires `input` to be NUL-terminated UTF-8.
    let input_bytes = unsafe { CStr::from_ptr(input) }.to_bytes();
    let input_str = match std::str::from_utf8(input_bytes) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("turm_ffi_call_json: input is not valid UTF-8: {e}"));
            return ptr::null_mut();
        }
    };

    let mut parsed: Value = match serde_json::from_str(input_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(format!("turm_ffi_call_json: input is not valid JSON: {e}"));
            return ptr::null_mut();
        }
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    if let Value::Object(ref mut map) = parsed {
        map.insert("echoed_at".into(), json!(now_ms));
    } else {
        // Non-object input is allowed but loses the echo metadata; wrap it
        // so the response shape is always an object.
        parsed = json!({ "input": parsed, "echoed_at": now_ms });
    }

    let serialized = match serde_json::to_string(&parsed) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("turm_ffi_call_json: serialization failed: {e}"));
            return ptr::null_mut();
        }
    };

    let cs = match CString::new(serialized) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(format!(
                "turm_ffi_call_json: serialized JSON contained NUL byte: {e}"
            ));
            return ptr::null_mut();
        }
    };

    clear_last_error();
    cs.into_raw()
}

/// Releases a string previously returned by a turm-ffi function that
/// allocates (currently `turm_ffi_call_json`).
///
/// # Safety
///
/// `s` must be a pointer previously returned by a turm-ffi function and
/// not yet freed, OR a null pointer (in which case this is a no-op).
/// Passing any other pointer is UB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn turm_ffi_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract requires `s` to come from a previous turm-ffi
    // CString::into_raw call. Reconstructing the CString hands ownership back
    // to Rust which then drops it.
    let _ = unsafe { CString::from_raw(s) };
}

/// Returns the most recent error message recorded on the calling thread,
/// or NULL if no error has been recorded since the last successful call.
///
/// # Safety
///
/// The returned pointer is borrowed from a thread-local slot and remains
/// valid only until the next FFI call on the same thread. Callers that
/// need to retain the message must copy it (e.g. Swift `String(cString:)`).
/// The pointer must NOT be passed to `turm_ffi_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn turm_ffi_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(cs) => cs.as_ptr(),
        None => ptr::null(),
    })
}
