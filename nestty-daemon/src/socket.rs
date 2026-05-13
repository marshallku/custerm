//! Socket transport for nesttyd.
//!
//! v0 scope: bind well-known socket path, accept connections, respond to
//! `ping`. No supervisor/trigger/event integration yet — those land in
//! follow-up commits within step 2 of the migration plan.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;

use nestty_core::protocol::{Request, Response};
use serde_json::json;

/// Owner-only permissions for the daemon's runtime directory.
/// `XDG_RUNTIME_DIR` is already 0700 by the XDG spec; the `/tmp/nestty-{uid}/`
/// fallback needs us to set this explicitly, otherwise another local user
/// can list / `cd` into the dir and connect the socket.
const RUNTIME_DIR_MODE: u32 = 0o700;
/// Owner-only permissions for the socket file itself. Defense in depth
/// against an XDG_RUNTIME_DIR misconfigured to weaker than 0700.
const SOCKET_FILE_MODE: u32 = 0o600;

/// Result of `prepare_socket_path` — distinguishes "fresh path, just bind"
/// from "stale socket file removed, safe to bind" from "another live daemon
/// is bound, refuse to start" from "path exists but isn't ours, hands off".
#[derive(Debug, PartialEq, Eq)]
pub enum SocketPrep {
    /// Path didn't exist; parent dir created if needed.
    Fresh,
    /// Stale socket file (no live listener) was removed.
    StaleCleared,
    /// A live `nesttyd` is already listening — caller must not bind.
    InUse,
    /// Path exists but is not a Unix socket — refuse to unlink. Caller
    /// likely pointed `NESTTY_SOCKET` at a regular file by mistake.
    NotSocket,
    /// Filesystem error while preparing.
    Error(String),
}

/// Idempotent prep: ensures the parent dir exists, detects stale sockets,
/// and reports whether it's safe to `bind` next.
///
/// **Safety**: we only unlink an existing path entry if it is itself a Unix
/// socket inode (via `stat` → `S_IFSOCK`). A regular file with the same
/// path — common if the user mistypes `NESTTY_SOCKET` — is left untouched
/// and surfaced as `NotSocket`. Same-user data files are not in our blast
/// radius.
///
/// Stale-ness is probed by trying a non-blocking connect on a verified
/// socket inode: a live listener accepts, a stale path returns
/// `ConnectionRefused`. The latter is safe to unlink.
pub fn prepare_socket_path(path: &Path) -> SocketPrep {
    if let Some(parent) = path.parent() {
        let owns_parent = parent == nestty_core::paths::runtime_dir();
        if owns_parent {
            // Atomic create with mode 0700 closes the race where a permissive
            // umask would briefly expose the new dir to other local users
            // (the `/tmp/nestty-{victim_uid}` pre-creation attack codex
            // flagged). `DirBuilder::mode` applies to every directory
            // newly created by `recursive(true)`; pre-existing parent
            // components (`/run/user/{uid}/`, `/tmp/`) are not modified.
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            builder.mode(RUNTIME_DIR_MODE);
            if let Err(e) = builder.create(parent) {
                return SocketPrep::Error(format!(
                    "create_dir_all({}, mode=0700): {e}",
                    parent.display()
                ));
            }

            // Verify ownership: an attacker could have pre-created the dir
            // before we ran, so the atomic-create no-ops on existing dirs.
            use std::os::unix::fs::MetadataExt;
            match fs::metadata(parent) {
                Ok(meta) => {
                    let current_uid = unsafe { libc::getuid() };
                    if meta.uid() != current_uid {
                        return SocketPrep::Error(format!(
                            "runtime dir {} not owned by uid {current_uid} (got uid={}); refusing to use — investigate before retrying",
                            parent.display(),
                            meta.uid()
                        ));
                    }
                }
                Err(e) => {
                    return SocketPrep::Error(format!("stat({}): {e}", parent.display()));
                }
            }

            // chmod still runs to repair a dir created by an older nesttyd
            // build that did `create_dir_all` without the mode flag — we
            // own it (verified above) so this is safe.
            if let Err(e) =
                fs::set_permissions(parent, fs::Permissions::from_mode(RUNTIME_DIR_MODE))
            {
                return SocketPrep::Error(format!("chmod({}, 0700): {e}", parent.display()));
            }
        } else if let Err(e) = fs::create_dir_all(parent) {
            return SocketPrep::Error(format!("create_dir_all({}): {e}", parent.display()));
        }
    }

    if !path.exists() {
        return SocketPrep::Fresh;
    }

    match is_unix_socket(path) {
        Ok(false) => return SocketPrep::NotSocket,
        Err(e) => return SocketPrep::Error(format!("stat({}): {e}", path.display())),
        Ok(true) => {}
    }

    // Probe stale-ness. Only `ConnectionRefused` is canonical "no listener
    // is alive on this inode". Any other error (PermissionDenied, transient
    // filesystem fault, etc.) is treated as unknown — we refuse to unlink
    // a possibly-live socket file in that case.
    match UnixStream::connect(path) {
        Ok(_) => SocketPrep::InUse,
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            match std::fs::remove_file(path) {
                Ok(()) => SocketPrep::StaleCleared,
                Err(e) => SocketPrep::Error(format!("unlink({}): {e}", path.display())),
            }
        }
        Err(e) => SocketPrep::Error(format!(
            "connect probe failed for {}: {e} ({:?})",
            path.display(),
            e.kind()
        )),
    }
}

/// `true` iff the path exists and is a Unix socket (`S_IFSOCK`).
fn is_unix_socket(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::FileTypeExt;
    let meta = std::fs::symlink_metadata(path)?;
    Ok(meta.file_type().is_socket())
}

/// Bind a UnixListener at the prepared path. Caller MUST have called
/// `prepare_socket_path` first and not received `InUse`/`Error`.
///
/// Sets the socket file to mode 0600 after bind. Unix-domain sockets honor
/// filesystem permissions for `connect()`, so this enforces owner-only
/// access even if the surrounding directory mode is lax.
pub fn bind_listener(path: &Path) -> std::io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_FILE_MODE)) {
        // Rollback: socket file is bound but we couldn't lock it down.
        // Refuse to return a permissive listener; the caller would otherwise
        // expose it to other UIDs.
        let _ = fs::remove_file(path);
        return Err(std::io::Error::other(format!(
            "chmod socket 0600 failed: {e}"
        )));
    }
    Ok(listener)
}

/// Accept loop. Spawns one OS thread per connection. Returns when the
/// listener yields a fatal error so the caller can run `cleanup_socket`.
///
/// We intentionally do NOT swallow errors: an `accept(2)` failure on a
/// Unix domain socket is almost always non-recoverable (fd exhaustion,
/// bad listener fd, etc.). Swallowing them would turn a real fault into
/// a tight warning loop and leave the socket inode on disk because
/// cleanup never reaches.
pub fn run_accept_loop(listener: UnixListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || handle_connection(stream));
            }
            Err(e) => {
                log::error!(
                    "nesttyd accept error: {e}; shutting down accept loop so caller can run cleanup"
                );
                break;
            }
        }
    }
}

fn handle_connection(stream: UnixStream) {
    let read_half = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("nesttyd try_clone failed: {e}");
            return;
        }
    };
    let reader = BufReader::new(read_half);
    let mut writer = stream;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                log::debug!("nesttyd connection read err: {e}");
                return;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => dispatch(&req),
            Err(e) => Response::error(
                String::new(),
                "invalid_request",
                &format!("malformed JSON: {e}"),
            ),
        };

        let encoded = match serde_json::to_string(&response) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("nesttyd response serialize error: {e}");
                continue;
            }
        };
        if writeln!(writer, "{encoded}").is_err() {
            return;
        }
    }
}

/// Dispatch a single Request → Response. v0 handles only `system.ping`
/// (the method `nestctl ping` issues, registered as a low-frequency synchronous
/// action in the GUI today via `ActionRegistry::register_silent`). Everything
/// else is `unknown_method`. Follow-up commits fold in supervisor and the
/// rest of registry dispatch.
pub fn dispatch(req: &Request) -> Response {
    match req.method.as_str() {
        "system.ping" => Response::success(
            req.id.clone(),
            json!({
                "status": "ok",
                "daemon": "nesttyd",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        ),
        other => Response::error(
            req.id.clone(),
            "unknown_method",
            &format!("nesttyd v0 only handles system.ping; got {other}"),
        ),
    }
}

/// Best-effort cleanup. Called by the binary on exit / panic so the next
/// daemon start sees `prepare_socket_path → Fresh`.
pub fn cleanup_socket(path: &PathBuf) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("nesttyd socket cleanup failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    fn tmp_socket() -> PathBuf {
        let dir = tempfile_dir();
        dir.join("test-sock")
    }

    fn tempfile_dir() -> PathBuf {
        // Avoid pulling tempfile crate as a dep for one test helper.
        let pid = std::process::id();
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("nesttyd-test-{pid}-{nano}"));
        std::fs::create_dir_all(&dir).expect("mkdir tmp");
        dir
    }

    #[test]
    fn dispatch_system_ping_returns_ok() {
        let req = Request::new("abc", "system.ping", json!({}));
        let resp = dispatch(&req);
        assert!(resp.ok);
        assert_eq!(resp.id, "abc");
        let body = resp.result.expect("result");
        assert_eq!(body["status"], json!("ok"));
        assert_eq!(body["daemon"], json!("nesttyd"));
    }

    #[test]
    fn dispatch_unknown_method_returns_error() {
        let req = Request::new("xyz", "nothing.here", json!({}));
        let resp = dispatch(&req);
        assert!(!resp.ok);
        let err = resp.error.expect("error");
        assert_eq!(err.code, "unknown_method");
    }

    #[test]
    fn prepare_socket_path_fresh() {
        let path = tmp_socket();
        let _ = std::fs::remove_file(&path);
        let res = prepare_socket_path(&path);
        assert_eq!(res, SocketPrep::Fresh);
    }

    #[test]
    fn prepare_socket_path_clears_stale_socket_inode() {
        // Bind + drop a UnixListener to leave a stale *socket* inode that
        // is no longer accepting. `prepare_socket_path` should detect that
        // it IS a socket (S_IFSOCK passes) and that connect refuses, then
        // unlink it.
        let path = tmp_socket();
        let _ = std::fs::remove_file(&path);
        {
            let _listener = UnixListener::bind(&path).expect("bind");
        } // dropped → listener closed, but socket inode persists on disk
        assert!(path.exists());
        let res = prepare_socket_path(&path);
        assert_eq!(res, SocketPrep::StaleCleared);
        assert!(!path.exists(), "stale socket inode should be unlinked");
    }

    #[test]
    fn bind_listener_sets_owner_only_perms() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_socket();
        let _ = std::fs::remove_file(&path);
        let _listener = bind_listener(&path).expect("bind");
        let meta = std::fs::metadata(&path).expect("metadata");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket file must be owner-only");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prepare_socket_path_leaves_foreign_parent_perms_alone() {
        use std::os::unix::fs::PermissionsExt;
        // The temp dir we use is NOT the daemon's runtime_dir(), so chmod
        // must NOT fire — verifies the C1 fix that prevents arbitrary
        // NESTTY_SOCKET overrides from locking down user dirs.
        let dir = tempfile_dir();
        let path = dir.join("test-foreign-parent-sock");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("loosen dir");
        let _ = std::fs::remove_file(&path);
        let res = prepare_socket_path(&path);
        assert_eq!(res, SocketPrep::Fresh);
        let mode = std::fs::metadata(&dir)
            .expect("dir metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755, "foreign parent dir perms must NOT be modified");
    }

    #[test]
    fn prepare_socket_path_refuses_regular_file() {
        // A non-socket inode at the path (e.g. mistyped NESTTY_SOCKET).
        // We must NOT unlink it.
        let path = tmp_socket();
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "very important user data").expect("write file");
        let res = prepare_socket_path(&path);
        assert_eq!(res, SocketPrep::NotSocket);
        assert!(path.exists(), "regular file must NOT be unlinked");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "very important user data"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prepare_socket_path_detects_live_listener() {
        let path = tmp_socket();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let res = prepare_socket_path(&path);
        assert_eq!(res, SocketPrep::InUse);
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn end_to_end_ping_roundtrip() {
        let path = tmp_socket();
        let _ = std::fs::remove_file(&path);
        let listener = bind_listener(&path).expect("bind");

        let path_clone = path.clone();
        let _server = thread::spawn(move || run_accept_loop(listener));

        // Give the accept loop a moment to call into thread::spawn.
        // Conservative: 50ms is plenty for local Unix sockets.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut stream = UnixStream::connect(&path_clone).expect("connect");
        let req = Request::new("rt-1", "system.ping", json!({}));
        let line = serde_json::to_string(&req).unwrap() + "\n";
        stream.write_all(line.as_bytes()).expect("write");

        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Response = serde_json::from_str(line.trim()).expect("parse");
        assert!(resp.ok);
        assert_eq!(resp.id, "rt-1");

        // Cleanup. accept loop is daemon-thread; will die with the test
        // process. Unlink the socket so a re-run doesn't see stale.
        drop(stream);
        let _ = std::fs::remove_file(&path_clone);
    }
}
