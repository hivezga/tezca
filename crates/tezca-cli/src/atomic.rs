//! Atomic config writes: temp file → fsync → rename.
//!
//! Every file this CLI rewrites — the Hyprland managed block, the keybind
//! override layer, the bar/dock config — is `source`d or parsed by a *running*
//! session. A plain `fs::write` truncates the file before it writes the new
//! bytes, so an interruption in that window (OOM kill, power loss, a full disk)
//! leaves a truncated config behind. For `keybinds.lua` that means logging in to
//! a session with no keybindings and no way to open a terminal.
//!
//! `rename(2)` is atomic within a filesystem, so a concurrent reader observes
//! either the entire old file or the entire new one — never a half-written one.
//! Writing the temp file into the *same directory* as the target is what keeps
//! the rename on one filesystem; a temp file in `/tmp` would fall back to a
//! copy and lose the guarantee.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Replace `path`'s contents with `body`, atomically. Creates parent directories
/// and preserves the existing file's permissions.
pub fn write(path: &Path, body: &str) -> Result<(), String> {
    let target = resolve(path);
    let dir =
        target.parent().ok_or_else(|| format!("{} has no parent directory", target.display()))?;
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("tezca");
    // The pid keeps two concurrent `tezca` invocations from sharing a temp name.
    let tmp = dir.join(format!(".{name}.tezca-tmp.{}", std::process::id()));

    let written = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        // Flush to the device before the rename, so a crash immediately after the
        // rename can't leave the new name pointing at unwritten blocks.
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", tmp.display()));
    }

    // A fresh temp file gets whatever the umask allows; carrying the old mode
    // over means rewriting a 0600 config doesn't quietly widen it to 0644.
    #[cfg(unix)]
    if let Ok(meta) = fs::metadata(&target) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(mode));
    }

    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace {}: {e}", target.display())
    })
}

/// Follow a symlinked *file* so we replace what it points at rather than the link.
///
/// This matters because `rename` over a symlink replaces the link itself with a
/// regular file. Without this, atomically writing a config that `tezca link`
/// symlinked into the repo would silently detach it from the repo — the opposite
/// of what the caller asked for.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    // The file may not exist yet, which makes `canonicalize` fail on the whole
    // path. Resolve the directory instead so the temp file still lands on the
    // same filesystem as the final name.
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => {
            fs::canonicalize(dir).map(|d| d.join(name)).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch directory per test — std has no tempdir helper and this
    /// CLI is deliberately dependency-free.
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tezca-atomic-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn creates_a_missing_file_and_its_parents() {
        let d = scratch("create");
        let p = d.join("nested/deeper/conf");
        write(&p, "hello\n").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "hello\n");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn replaces_contents_and_leaves_no_temp_file() {
        let d = scratch("replace");
        let p = d.join("conf");
        write(&p, "one\n").unwrap();
        write(&p, "two\n").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two\n");
        // The whole point is that nothing is left mid-flight.
        let leftovers: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("tezca-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn preserves_the_existing_mode() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("mode");
        let p = d.join("secret");
        write(&p, "a\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        write(&p, "b\n").unwrap();
        let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "rewriting a 0600 config must not widen it");
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_a_symlinked_file_instead_of_replacing_the_link() {
        let d = scratch("symlink");
        let real = d.join("real.conf");
        let link = d.join("link.conf");
        write(&real, "original\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write(&link, "updated\n").unwrap();

        // The link must still be a link, and the target must hold the new bytes.
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "updated\n");
        fs::remove_dir_all(&d).unwrap();
    }
}
