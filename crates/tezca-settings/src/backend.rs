//! Shelling out to `tezca` + reading its state files. The panel does no real
//! work itself — every action is a `tezca` / hyprctl / script call, the same
//! thing the keybinds do, so the GUI and keyboard paths stay identical.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Absolute path to the `tezca` binary — prefer ~/.local/bin (where install.sh
/// puts it; not always on a GUI process's PATH), else fall back to PATH lookup.
pub fn tezca_bin() -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/bin/tezca");
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    "tezca".to_string()
}

/// Spawn `tezca <args>` detached, ignoring output (theme set, game toggle, …).
pub fn tezca(args: &[&str]) {
    spawn(&tezca_bin(), args);
}

/// Run `tezca <args>` and capture trimmed stdout (theme names, …).
pub fn tezca_out(args: &[&str]) -> Option<String> {
    output(&tezca_bin(), args)
}

/// Spawn an arbitrary command detached (hyprctl, scripts, wlogout, hyprlock, …).
pub fn spawn(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Capture trimmed stdout of an arbitrary command (None on failure).
pub fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A path under ~/.config/tezca/…
fn config_tezca(rel: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tezca").join(rel))
}

/// The active curated theme name — `current/theme.state` holds "obsidian" or
/// "dynamic:/path". Returns the curated name, or None when dynamic/unset.
pub fn active_theme() -> Option<String> {
    let s = std::fs::read_to_string(config_tezca("current/theme.state")?)
        .ok()?
        .trim()
        .to_string();
    if s.is_empty() || s.starts_with("dynamic:") {
        None
    } else {
        Some(s)
    }
}

/// Current wallpaper path from `current/wallpaper`.
pub fn current_wallpaper() -> Option<PathBuf> {
    let s = std::fs::read_to_string(config_tezca("current/wallpaper")?)
        .ok()?
        .trim()
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

/// Whether game mode is on — `game.state` contains "on" when active.
pub fn game_on() -> bool {
    config_tezca("game.state")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim() == "on")
        .unwrap_or(false)
}

/// `command -v <bin>` succeeds.
pub fn has(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run one of the hypr/scripts/*.sh helpers by name, detached.
pub fn run_script(name: &str, args: &[&str]) {
    let Some(home) = std::env::var_os("HOME") else { return };
    let path = PathBuf::from(home).join(".config/hypr/scripts").join(name);
    if let Some(p) = path.to_str() {
        spawn(p, args);
    }
}
