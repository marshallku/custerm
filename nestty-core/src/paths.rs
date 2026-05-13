//! Platform-aware filesystem paths for nesttyd and its clients.

use std::env;
use std::path::PathBuf;

/// - Linux: `$XDG_RUNTIME_DIR/nestty/` or `/tmp/nestty-{uid}/` (uid-namespaced
///   so multi-user `/tmp` doesn't race on first-binder).
/// - macOS: `~/Library/Caches/nestty/` (no XDG_RUNTIME_DIR equivalent).
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

/// `nesttyd` listens here; `nestctl` connects here unless `NESTTY_SOCKET`
/// overrides.
pub fn socket_path() -> PathBuf {
    if let Ok(override_path) = env::var("NESTTY_SOCKET")
        && !override_path.is_empty()
    {
        return PathBuf::from(override_path);
    }
    runtime_dir().join("socket")
}

/// Persistent state (handoffs, indices) — Linux `~/.local/state/nestty/`,
/// macOS `~/Library/Application Support/nestty/`.
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

/// Regenerable cache (wallpaper lists, derived indices).
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

/// Dir exists, is owned by current uid, and grants no group/other access.
/// Blocks the `/tmp/nestty-{victim_uid}` pre-creation attack on systems
/// without `XDG_RUNTIME_DIR`.
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
        // SAFETY: env in tests is fine if no other thread reads it.
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
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("loosen");
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
