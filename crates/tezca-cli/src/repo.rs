//! Locate the Tezca repository root and derive well-known paths.

use std::path::{Path, PathBuf};

const MARKER: &str = ".tezca-root";

/// Resolve the repository root, in priority order:
/// 1. `$TEZCA_REPO` (explicit override)
/// 2. walk up from the current working directory looking for `.tezca-root`
/// 3. walk up from the running executable's directory
pub fn root() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("TEZCA_REPO") {
        let p = PathBuf::from(dir);
        if p.join(MARKER).is_file() {
            return Ok(p);
        }
        return Err(format!(
            "$TEZCA_REPO is set to {} but no {MARKER} marker was found there",
            p.display()
        ));
    }

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(p) = walk_up(&cwd) {
            return Ok(p);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(p) = walk_up(dir) {
                return Ok(p);
            }
        }
    }

    Err(format!(
        "could not locate the Tezca repo (no {MARKER} found above the current \
         directory). Run from inside the repo or set $TEZCA_REPO."
    ))
}

fn walk_up(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(dir) = cur {
        if dir.join(MARKER).is_file() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// `~/.config` (honouring `$XDG_CONFIG_HOME`).
pub fn config_home() -> Result<PathBuf, String> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x));
        }
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "neither $XDG_CONFIG_HOME nor $HOME is set".to_string())?;
    Ok(PathBuf::from(home).join(".config"))
}
