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

/// True when `bin` is runnable, by walking `$PATH` directly.
///
/// Deliberately not `sh -c "command -v $bin"`: that spawns a shell per probe and
/// interpolates its argument into a shell string. (The `tezca` CLI has the same
/// helper in its own `util` module — the two crates share no library, and a
/// six-line function is not worth restructuring the workspace for.)
pub fn has(bin: &str) -> bool {
    if bin.contains('/') {
        return is_executable(&PathBuf::from(bin));
    }
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .any(|d| is_executable(&d.join(bin)))
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
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

// ---------------------------------------------------------------------------
// Structured helpers for the Displays / Dock / Desktop / Keybinds pages
// ---------------------------------------------------------------------------

/// Result of a `tezca` invocation we need to branch on (e.g. rebind conflicts).
pub struct CmdResult {
    pub code: i32,
    pub stderr: String,
}

/// Run `tezca <args>` synchronously, returning its exit code + stderr.
pub fn tezca_result(args: &[&str]) -> CmdResult {
    match Command::new(tezca_bin()).args(args).output() {
        Ok(o) => CmdResult {
            code: o.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => CmdResult { code: -1, stderr: e.to_string() },
    }
}

/// One connected monitor, from `tezca display list --machine`.
#[derive(Clone, Default)]
pub struct Monitor {
    pub name: String,
    pub desc: String,
    pub res: String,
    pub rate: String,
    pub pos: String,
    pub scale: String,
    pub modes: Vec<String>, // "3440x1440@165.00"
}

pub fn monitors() -> Vec<Monitor> {
    let Some(out) = tezca_out(&["display", "list", "--machine"]) else {
        return Vec::new();
    };
    let mut mons: Vec<Monitor> = Vec::new();
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@monitor ") {
            mons.push(Monitor { name: name.trim().to_string(), ..Default::default() });
            continue;
        }
        let Some(m) = mons.last_mut() else { continue };
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "desc" => m.desc = v.to_string(),
            "res" => m.res = v.to_string(),
            "rate" => m.rate = v.to_string(),
            "pos" => m.pos = v.to_string(),
            "scale" => m.scale = v.to_string(),
            "modes" => m.modes = v.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    mons
}

/// Per-monitor wallpaper targets: (name, is_override, path).
pub fn wallpaper_targets() -> Vec<(String, bool, String)> {
    let Some(out) = tezca_out(&["wallpaper", "list"]) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let name = f.next()?.trim().to_string();
            let source = f.next()?.trim();
            let path = f.next().unwrap_or("").trim().to_string();
            Some((name, source == "override", path))
        })
        .collect()
}

/// DDC/CI brightness (0-100) for a monitor, or None if not DDC-capable.
pub fn brightness(name: &str) -> Option<i32> {
    tezca_out(&["display", "brightness", name])?.trim().parse().ok()
}

/// The effective dock config as key→value strings (`tezca dock config`).
pub fn dock_config() -> Vec<(String, String)> {
    config_pairs(&["dock", "config"])
}

/// The effective bar config as key→value strings (`tezca bar config`).
pub fn bar_config() -> Vec<(String, String)> {
    config_pairs(&["bar", "config"])
}

/// Discovered custom bar modules: (layout id `custom:<name>`, display label).
/// Scans `~/.config/tezca-bar/modules/*.toml` — the same drop-in directory the
/// bar reads — so a manifest dropped in there shows up in the Modules editor.
pub fn custom_bar_modules() -> Vec<(String, String)> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    let Some(dir) = base.map(|b| b.join("tezca-bar").join("modules")) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        // A manifest with no `exec` is a no-op module; skip it here too.
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let has_exec = text.lines().any(|l| {
            let l = l.trim();
            l.split_once('=').map(|(k, _)| matches!(k.trim(), "exec" | "command")).unwrap_or(false)
        });
        if !has_exec {
            continue;
        }
        let label = text
            .lines()
            .find_map(|l| {
                let (k, v) = l.trim().split_once('=')?;
                matches!(k.trim(), "label" | "name")
                    .then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| title_case(stem));
        out.push((format!("custom:{stem}"), label));
    }
    out.sort();
    out
}

/// `weather-city` → `Weather city`.
fn title_case(s: &str) -> String {
    let s = s.replace(['-', '_'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s.clone(),
    }
}

/// Shared `key = value` parse for the `tezca <x> config` commands.
fn config_pairs(args: &[&str]) -> Vec<(String, String)> {
    let Some(out) = tezca_out(args) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// The current value of a Hyprland option (`tezca hypr get`).
pub fn hypr_get(opt: &str) -> Option<String> {
    tezca_out(&["hypr", "get", opt])
}
