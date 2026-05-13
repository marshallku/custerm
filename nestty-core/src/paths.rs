//! Platform-aware filesystem paths for the daemon and its clients.
//!
//! These helpers replace the previous PID-tied `/tmp/nestty-{PID}.sock` so
//! `nesttyd` can own a stable, well-known socket location across restarts.
//! Discovery from `nestctl` and external integrations (hooks, life-assistant
//! bridge) becomes "does this path exist and connect".
//!
//! See `docs/gui-daemon-protocol.md` § Baseline → Transport and
//! `docs/harness-integration.md` § Platform abstraction.

use std::env;
use std::path::PathBuf;

/// Per-user runtime directory — short-lived sockets, pids.
///
/// - Linux: `${XDG_RUNTIME_DIR}/nestty/` if set, else `/tmp/nestty-{uid}/`.
///   `/tmp` fallback is namespaced by uid so concurrent users on the same
///   box don't collide.
/// - macOS: `~/Library/Caches/nestty/` (Apple's blessed transient cache
///   location; macOS has no XDG_RUNTIME_DIR equivalent).
pub fn runtime_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = env::var("XDG_RUNTIME_DIR")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("nestty");
        }
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/nestty-{uid}"))
    }
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Library/Caches/nestty")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp/nestty")
    }
}

/// Well-known daemon socket path. `nesttyd` listens here; `nestctl` and
/// other clients connect here unless `NESTTY_SOCKET` overrides.
pub fn socket_path() -> PathBuf {
    if let Ok(override_path) = env::var("NESTTY_SOCKET")
        && !override_path.is_empty()
    {
        return PathBuf::from(override_path);
    }
    runtime_dir().join("socket")
}

/// Persistent per-user state (handoffs, indices, anything that should
/// survive reboot but isn't config).
///
/// - Linux: `~/.local/state/nestty/` (XDG state dir).
/// - macOS: `~/Library/Application Support/nestty/`.
pub fn state_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = env::var("XDG_STATE_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("nestty");
        }
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state/nestty")
    }
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Application Support/nestty")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from(".nestty")
    }
}

/// Cache dir for things that can be regenerated (wallpaper lists,
/// derived indices, etc).
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = env::var("XDG_CACHE_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("nestty");
        }
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache/nestty")
    }
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Caches/nestty")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from(".nestty-cache")
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// Verify a directory is safe to host the daemon's well-known socket:
///
/// - exists and is a directory,
/// - owned by the current user, and
/// - has no group/other permission bits set (mode `& 0o077 == 0`).
///
/// This blocks the "attacker pre-creates `/tmp/nestty-{victim_uid}` before
/// first daemon start" attack on systems without `XDG_RUNTIME_DIR`. Both
/// `nesttyd` (before binding) and `nestctl` (before connecting to the
/// daemon well-known path) consult this so neither uses an attacker-owned
/// dir.
pub fn is_trusted_dir(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    let current_uid = unsafe { libc::getuid() };
    if meta.uid() != current_uid {
        return false;
    }
    (meta.mode() & 0o077) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_respects_env_override() {
        // SAFETY: env mutation in tests is fine if no other thread reads it.
        unsafe {
            env::set_var("NESTTY_SOCKET", "/custom/path/sock");
        }
        assert_eq!(socket_path(), PathBuf::from("/custom/path/sock"));
        unsafe {
            env::remove_var("NESTTY_SOCKET");
        }
    }

    #[test]
    fn runtime_dir_returns_nonempty() {
        let dir = runtime_dir();
        assert!(!dir.as_os_str().is_empty());
        assert!(dir.to_string_lossy().contains("nestty"));
    }

    #[test]
    fn is_trusted_dir_rejects_missing() {
        let nonexistent = PathBuf::from("/tmp/nestty-test-does-not-exist-123456");
        assert!(!is_trusted_dir(&nonexistent));
    }

    #[test]
    fn is_trusted_dir_rejects_world_accessible() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "nestty-trust-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("loosen perms");
        assert!(!is_trusted_dir(&dir), "0755 dir must NOT be trusted");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("tighten");
        assert!(is_trusted_dir(&dir), "0700 dir owned by us IS trusted");
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn paths_are_distinct() {
        unsafe {
            env::remove_var("NESTTY_SOCKET");
        }
        let sock = socket_path();
        let state = state_dir();
        let cache = cache_dir();
        assert_ne!(sock, state);
        assert_ne!(state, cache);
    }
}
