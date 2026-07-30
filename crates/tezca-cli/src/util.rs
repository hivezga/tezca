//! Small shared helpers.

use std::path::{Path, PathBuf};

/// Resolve `bin` on `$PATH` without a shell.
///
/// This replaces seven byte-identical copies of a `sh -c "command -v {bin}"`
/// helper (`cmd_theme`, `cmd_doctor`, `cmd_dock`, `cmd_wallpaper`, `cmd_display`,
/// `cmd_game`, `cmd_bar`). Each copy spawned a shell per probe and
/// interpolated its argument into a shell string — a shape that is only safe for
/// as long as every caller happens to pass a literal. Walking `$PATH` has no
/// quoting semantics to get wrong and no process to spawn.
pub fn which(bin: &str) -> Option<PathBuf> {
    // An argument containing a slash is a path, not a name to look up.
    if bin.contains('/') {
        let p = PathBuf::from(bin);
        return is_executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(bin))
        .find(|p| is_executable(p))
}

/// True when `bin` is runnable — the common case, where the path is irrelevant.
pub fn has(bin: &str) -> bool {
    which(bin).is_some()
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_binary_that_is_certainly_present() {
        // `sh` is required by POSIX and by this project's own scripts.
        assert!(has("sh"), "sh should be on PATH");
    }

    #[test]
    fn reports_a_missing_binary_as_absent() {
        assert!(!has("tezca-definitely-not-installed-zzz"));
    }

    #[test]
    fn accepts_an_explicit_path_without_searching_path() {
        assert_eq!(which("/bin/sh").is_some(), Path::new("/bin/sh").exists());
        assert!(which("/nonexistent/zzz").is_none());
    }

    #[test]
    fn a_directory_on_path_is_not_mistaken_for_an_executable() {
        // `/bin/.` resolves to a directory; the old `command -v` helper would
        // also reject it, and so must this one.
        assert!(which("/bin/.").is_none());
    }
}
