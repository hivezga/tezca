//! Thin wrappers over `hyprctl` shared by `tezca hypr` and `tezca display`.

use std::process::Command;

/// True when we're inside a live Hyprland session (so `hyprctl` is meaningful).
pub fn in_session() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// Apply a keyword live: `hyprctl keyword <kw> <value>`. `value` may contain
/// spaces (e.g. a monitor spec or a gaps tuple) — passed as one argument.
pub fn keyword(kw: &str, value: &str) -> Result<(), String> {
    let out = Command::new("hyprctl")
        .arg("keyword")
        .arg(kw)
        .arg(value)
        .output()
        .map_err(|e| format!("failed to run hyprctl: {e}"))?;
    if out.status.success() {
        // hyprctl prints "ok" on success; some keywords print a note on stdout
        // even when they fail, so also sanity-check stderr.
        let err = String::from_utf8_lossy(&out.stderr);
        if err.to_lowercase().contains("error") {
            return Err(err.trim().to_string());
        }
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Read the current value of an option as a normalized scalar. Handles every
/// `hyprctl getoption` shape (int / float / custom "a b c d" / str) by taking
/// the first whitespace token of the value after the leading `type:` label.
///   `int: 12`            → "12"
///   `float: 0.980000`    → "0.980000"
///   `custom type: 5 5 5 5` → "5"
pub fn getoption(kw: &str) -> Option<String> {
    let out = Command::new("hyprctl")
        .args(["getoption", kw])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let first = s.lines().next()?;
    let (_ty, val) = first.split_once(':')?;
    Some(val.trim().split_whitespace().next()?.to_string())
}

/// `hyprctl reload` — re-source the whole config (best-effort).
pub fn reload() -> Result<(), String> {
    let out = Command::new("hyprctl")
        .arg("reload")
        .output()
        .map_err(|e| format!("failed to run hyprctl reload: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
