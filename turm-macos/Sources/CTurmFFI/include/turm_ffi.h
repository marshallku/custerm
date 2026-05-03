// turm_ffi.h — C declarations for symbols exported by the turm-ffi staticlib.
//
// Hand-maintained to match turm-ffi/src/lib.rs. The crate has no cbindgen
// step yet because the surface is small and the spike doesn't justify the
// build-system overhead. Keep this file in lockstep with the Rust source —
// any new `extern "C"` symbol there needs a declaration here, with the same
// ownership/safety contract documented.

#ifndef TURM_FFI_H
#define TURM_FFI_H

#ifdef __cplusplus
extern "C" {
#endif

/// Returns a NUL-terminated static version string. DO NOT free.
const char *turm_ffi_version(void);

/// Echo a JSON string back with an `echoed_at` timestamp added. Returns a
/// heap-allocated NUL-terminated string the caller MUST free with
/// `turm_ffi_free_string`. Returns NULL on error; call `turm_ffi_last_error`
/// for the message.
char *turm_ffi_call_json(const char *input);

/// Free a string previously returned by a turm-ffi function. Pass NULL is OK.
void turm_ffi_free_string(char *s);

/// Returns the most recent error message recorded on the calling thread,
/// or NULL if none. The pointer is borrowed (do NOT free) and is invalidated
/// by the next FFI call on the same thread.
const char *turm_ffi_last_error(void);

#ifdef __cplusplus
}
#endif

#endif // TURM_FFI_H
