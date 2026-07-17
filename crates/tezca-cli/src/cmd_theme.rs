//! `tezca theme` — the wallpaper-driven theme engine (DESIGN.md §7).
//!
//! One wallpaper drives the whole desktop's color. Two modes:
//!   * dynamic — `theme wallpaper <img>`: matugen extracts a Material-You palette
//!     and renders templates/ into ~/.config/tezca/current/.
//!   * curated — `theme set <name>`: copies a hand-tuned palette from themes/<name>/
//!     verbatim (matugen not involved), pinning an exact look.
//!
//! Either way we then repoint every component's stable import at current/ and
//! send each its live-reload signal — no restarts. See templates/README.md.

use crate::{repo, term};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The generated files every component imports from ~/.config/tezca/current/.
const FILES: &[&str] = &[
    "colors.css",          // GTK: Waybar, swaync, Walker
    "colors-kitty.conf",   // kitty
    "colors-hypr.conf",    // hypr/conf.d/decoration.conf (borders/shadows)
    "colors-hyprlock.conf", // hypr/hyprlock.conf (+ wallpaper path)
];

/// Token in colors-hyprlock.conf we substitute with the wallpaper's abs path.
const WALLPAPER_TOKEN: &str = "__TZ_WALLPAPER__";

/// The theme applied when none is specified (bootstrap + fallback).
const DEFAULT_THEME: &str = "obsidian";

struct Opts {
    set_wallpaper: bool,
    reload: bool,
    announce: bool,
}

/// CLI entry. Returns a process exit code.
pub fn run(args: &[&str]) -> i32 {
    let mut it = args.iter().copied();
    let r = match it.next() {
        None | Some("list") | Some("ls") => cmd_list(),
        Some("set") => match it.next() {
            Some(name) => cmd_set(name),
            None => Err("usage: tezca theme set <name>".into()),
        },
        Some("wallpaper") | Some("wall") => match it.next() {
            Some(img) => cmd_wallpaper(img),
            None => Err("usage: tezca theme wallpaper <image>".into()),
        },
        Some("reload") => cmd_reload(),
        Some(other) => Err(format!(
            "unknown theme subcommand: {other}\n  try: list · set <name> · wallpaper <img> · reload"
        )),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{} {e}", term::red("error:"));
            1
        }
    }
}

/// Ensure ~/.config/tezca/current/ is populated so components never `@import`
/// or `source` a missing file. Called after `tezca link`. Applies the default
/// curated theme (writing files only — no wallpaper set, no live reload).
pub fn ensure_default(quiet: bool) -> Result<(), String> {
    let current = current_dir()?;
    if current.join("colors.css").is_file() {
        return Ok(()); // already themed — leave the user's choice alone
    }
    if !quiet {
        println!(
            "  {} seeding default theme {}",
            term::dim("theme:"),
            term::cyan(DEFAULT_THEME)
        );
    }
    apply_curated(
        DEFAULT_THEME,
        Opts { set_wallpaper: false, reload: false, announce: false },
    )
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

fn cmd_list() -> Result<(), String> {
    println!("{}", term::header("tezca theme"));
    println!();

    let themes = curated_themes()?;
    let active = read_state();

    println!("{}", term::bold("curated themes"));
    if themes.is_empty() {
        println!("  {}", term::dim("(none found in themes/)"));
    }
    for (name, desc) in &themes {
        let is_active = active.as_deref() == Some(name.as_str());
        let mark = if is_active { term::green("●") } else { term::dim("○") };
        let label = if is_active {
            format!("{} {}", term::bold(name), term::dim("(active)"))
        } else {
            term::bold(name)
        };
        println!("  {mark} {label} {}", term::dim(&format!("— {desc}")));
    }
    println!();

    println!("{}", term::bold("dynamic"));
    let dyn_active = active.as_deref().map(|s| s.starts_with("dynamic:")).unwrap_or(false);
    let mark = if dyn_active { term::green("●") } else { term::dim("○") };
    println!(
        "  {mark} extract a palette from any wallpaper: {}",
        term::cyan("tezca theme wallpaper <image>")
    );
    if let Some(state) = &active {
        if let Some(img) = state.strip_prefix("dynamic:") {
            println!("    {} {}", term::dim("active:"), term::dim(img));
        }
    }
    println!();
    Ok(())
}

fn cmd_set(name: &str) -> Result<(), String> {
    apply_curated(name, Opts { set_wallpaper: true, reload: true, announce: true })
}

fn cmd_wallpaper(img: &str) -> Result<(), String> {
    apply_dynamic(img, Opts { set_wallpaper: true, reload: true, announce: true })
}

/// Re-apply the current theme: set its wallpaper and re-send reload signals.
/// Handy after hand-editing a config, or from session autostart.
fn cmd_reload() -> Result<(), String> {
    announce_header("reload");
    let current = current_dir()?;
    if !current.join("colors.css").is_file() {
        return Err("no active theme — run `tezca theme set obsidian` first".into());
    }
    if let Some(wp) = read_wallpaper() {
        set_wallpaper(&wp);
    }
    reload_components();
    println!();
    println!("  {} theme reloaded", term::green("done:"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Curated: copy themes/<name>/ → current/
// ---------------------------------------------------------------------------

fn apply_curated(name: &str, opts: Opts) -> Result<(), String> {
    let root = repo::root()?;
    let theme_dir = root.join("themes").join(name);
    let meta_path = theme_dir.join("theme.meta");
    if !meta_path.is_file() {
        return Err(format!(
            "no curated theme '{name}' ({} not found)",
            meta_path.display()
        ));
    }
    let meta = read_meta(&meta_path)?;
    let current = current_dir()?;
    fs::create_dir_all(&current)
        .map_err(|e| format!("cannot create {}: {e}", current.display()))?;

    if opts.announce {
        announce_header("set");
        println!("  {} {}", term::dim("theme:"), term::cyan(name));
    }

    // Copy the palette files verbatim.
    for f in FILES {
        let src = theme_dir.join(f);
        if !src.is_file() {
            return Err(format!("theme '{name}' is missing {}", src.display()));
        }
        let dst = current.join(f);
        fs::copy(&src, &dst)
            .map_err(|e| format!("cannot write {}: {e}", dst.display()))?;
    }

    // Resolve the theme's wallpaper (relative to wallpapers/, or an abs path).
    let wallpaper = meta_get(&meta, "wallpaper").map(|w| resolve_wallpaper(&root, w));
    finalize(&current, wallpaper.as_deref(), &format!("{name}"), &opts)
}

// ---------------------------------------------------------------------------
// Dynamic: matugen renders templates/ → current/
// ---------------------------------------------------------------------------

fn apply_dynamic(img: &str, opts: Opts) -> Result<(), String> {
    if !which("matugen") {
        return Err("matugen not found — install it for dynamic theming (`paru -S matugen`)".into());
    }
    let root = repo::root()?;
    let templates = root.join("templates");
    let img_abs = fs::canonicalize(img)
        .map_err(|e| format!("cannot read image '{img}': {e}"))?;

    let current = current_dir()?;
    fs::create_dir_all(&current)
        .map_err(|e| format!("cannot create {}: {e}", current.display()))?;

    if opts.announce {
        announce_header("wallpaper");
        println!("  {} {}", term::dim("image:"), term::dim(&img_abs.display().to_string()));
    }

    // Write a resolved matugen config (abs input/output paths) and run it.
    let mcfg = write_matugen_config(&templates, &current)?;
    let out = Command::new("matugen")
        .arg("-c").arg(&mcfg)
        .arg("image").arg(&img_abs)
        .arg("--prefer").arg("saturation") // non-interactive: no TTY to disambiguate
        .arg("-m").arg("dark")
        .output()
        .map_err(|e| format!("failed to run matugen: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // matugen colorizes errors; strip the noisiest control chars for a clean line.
        let msg = err.lines().rev().find(|l| l.contains("rror") || l.contains("ailed"))
            .unwrap_or_else(|| err.lines().last().unwrap_or(""))
            .trim();
        return Err(format!("matugen failed: {}", strip_ansi(msg)));
    }

    finalize(&current, Some(&img_abs), &format!("dynamic:{}", img_abs.display()), &opts)
}

// ---------------------------------------------------------------------------
// Shared finish: wallpaper-path injection, state files, reload
// ---------------------------------------------------------------------------

/// After the palette files land in current/, inject the wallpaper path into
/// colors-hyprlock.conf, record the wallpaper + active-theme state, then
/// (optionally) set the wallpaper and reload every component.
fn finalize(
    current: &Path,
    wallpaper: Option<&Path>,
    state: &str,
    opts: &Opts,
) -> Result<(), String> {
    // Substitute __TZ_WALLPAPER__ in colors-hyprlock.conf.
    let hl = current.join("colors-hyprlock.conf");
    if let Ok(text) = fs::read_to_string(&hl) {
        let path_str = wallpaper.map(|p| p.display().to_string()).unwrap_or_default();
        let patched = text.replace(WALLPAPER_TOKEN, &path_str);
        fs::write(&hl, patched)
            .map_err(|e| format!("cannot write {}: {e}", hl.display()))?;
    }

    // Record the active wallpaper (autostart reads this) and theme state.
    if let Some(wp) = wallpaper {
        fs::write(current.join("wallpaper"), format!("{}\n", wp.display()))
            .map_err(|e| format!("cannot write wallpaper marker: {e}"))?;
    }
    fs::write(current.join("theme.state"), format!("{state}\n"))
        .map_err(|e| format!("cannot write theme.state: {e}"))?;

    if opts.set_wallpaper {
        if let Some(wp) = wallpaper {
            if wp.is_file() {
                set_wallpaper(wp);
            } else if opts.announce {
                println!(
                    "  {} wallpaper not found: {}",
                    term::yellow("!"),
                    term::dim(&wp.display().to_string())
                );
            }
        }
    }
    if opts.reload {
        reload_components();
    }
    if opts.announce {
        println!();
        println!("  {} theme applied", term::green("done:"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Component reload — best-effort, reported per component
// ---------------------------------------------------------------------------

fn reload_components() {
    println!();
    println!("{}", term::bold("reloading components"));

    // Hyprland — only meaningful inside a live session.
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        report("hyprland", run_ok("hyprctl", &["reload"]));
    } else {
        report("hyprland", Outcome::Skipped("not in a Hyprland session".into()));
    }

    // Waybar — SIGUSR2 reloads its stylesheet.
    report("waybar", signal("waybar", "USR2"));

    // swaync — reload the CSS via its control client.
    if which("swaync-client") {
        report("swaync", run_ok("swaync-client", &["--reload-css"]));
    } else {
        report("swaync", Outcome::Skipped("swaync-client not found".into()));
    }

    // kitty — SIGUSR1 makes every instance re-read its config.
    report("kitty", signal("kitty", "USR1"));

    // Walker re-reads its theme CSS on each on-demand launch — nothing to signal.
    report("walker", Outcome::Skipped("re-reads on next launch".into()));
}

enum Outcome {
    Done(String),
    Skipped(String),
    Failed(String),
}

fn report(name: &str, o: Outcome) {
    match o {
        Outcome::Done(d) => {
            let detail = if d.is_empty() { String::new() } else { format!(" — {d}") };
            println!("  {} {name}{}", term::green("✓"), term::dim(&detail));
        }
        Outcome::Skipped(d) => {
            println!("  {} {name} {}", term::dim("·"), term::dim(&format!("— {d}")));
        }
        Outcome::Failed(d) => {
            println!("  {} {name} {}", term::yellow("!"), term::dim(&format!("— {d}")));
        }
    }
}

/// Send a signal to all processes with the given name via `pkill`.
/// pkill exits 0 when it signalled ≥1 process, 1 when none matched.
fn signal(proc: &str, sig: &str) -> Outcome {
    match Command::new("pkill").arg(format!("-{sig}")).arg("-x").arg(proc).status() {
        Ok(s) if s.success() => Outcome::Done(format!("SIG{sig}")),
        Ok(_) => Outcome::Skipped("not running".into()),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

fn run_ok(prog: &str, args: &[&str]) -> Outcome {
    match Command::new(prog).args(args).output() {
        Ok(o) if o.status.success() => Outcome::Done(String::new()),
        Ok(o) => Outcome::Failed(
            strip_ansi(String::from_utf8_lossy(&o.stderr).trim()).chars().take(60).collect(),
        ),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// Paint the wallpaper with awww (the swww successor). Best-effort.
fn set_wallpaper(path: &Path) {
    if !which("awww") {
        report("wallpaper", Outcome::Skipped("awww not found".into()));
        return;
    }
    let o = Command::new("awww")
        .arg("img")
        .arg(path)
        .arg("--transition-type").arg("grow")
        .arg("--transition-pos").arg("center")
        .output();
    match o {
        Ok(s) if s.status.success() => report("wallpaper", Outcome::Done(String::new())),
        Ok(s) => report(
            "wallpaper",
            Outcome::Failed(
                strip_ansi(String::from_utf8_lossy(&s.stderr).trim())
                    .lines().next().unwrap_or("awww img failed").to_string(),
            ),
        ),
        Err(e) => report("wallpaper", Outcome::Failed(e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// matugen config generation
// ---------------------------------------------------------------------------

/// Write a resolved matugen config (absolute input/output paths) to the cache
/// dir and return its path. Regenerated each run so it always tracks the real
/// repo + config locations (robust to a repo whose path contains a ':').
fn write_matugen_config(templates: &Path, current: &Path) -> Result<PathBuf, String> {
    let cache = cache_dir()?.join("tezca");
    fs::create_dir_all(&cache)
        .map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
    let cfg_path = cache.join("matugen.toml");

    let mut body = String::from("# Generated by `tezca theme` — do not edit.\n[config]\n\n");
    for f in FILES {
        // matugen renders only file-based templates; a plain key per file.
        let key = f.replace(['.', '-'], "_");
        body.push_str(&format!("[templates.{key}]\n"));
        body.push_str(&format!("input_path = \"{}\"\n", templates.join(f).display()));
        body.push_str(&format!("output_path = \"{}\"\n\n", current.join(f).display()));
    }
    fs::write(&cfg_path, body)
        .map_err(|e| format!("cannot write {}: {e}", cfg_path.display()))?;
    Ok(cfg_path)
}

// ---------------------------------------------------------------------------
// Paths, metadata, small helpers
// ---------------------------------------------------------------------------

fn current_dir() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("current"))
}

fn cache_dir() -> Result<PathBuf, String> {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x));
        }
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "neither $XDG_CACHE_HOME nor $HOME is set".to_string())?;
    Ok(PathBuf::from(home).join(".cache"))
}

/// A wallpaper reference in theme.meta is relative to the repo's wallpapers/
/// dir unless it's already an absolute path.
fn resolve_wallpaper(root: &Path, w: &str) -> PathBuf {
    let p = PathBuf::from(w);
    if p.is_absolute() {
        p
    } else {
        root.join("wallpapers").join(w)
    }
}

fn curated_themes() -> Result<Vec<(String, String)>, String> {
    let dir = repo::root()?.join("themes");
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(&dir) else { return Ok(out) };
    for e in rd.flatten() {
        let meta = e.path().join("theme.meta");
        if !meta.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let m = read_meta(&meta).unwrap_or_default();
        let desc = meta_get(&m, "description").unwrap_or("").to_string();
        out.push((name, desc));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn read_meta(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok(out)
}

fn meta_get<'a>(meta: &'a [(String, String)], key: &str) -> Option<&'a str> {
    meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn read_state() -> Option<String> {
    let p = current_dir().ok()?.join("theme.state");
    Some(fs::read_to_string(p).ok()?.trim().to_string()).filter(|s| !s.is_empty())
}

fn read_wallpaper() -> Option<PathBuf> {
    let p = current_dir().ok()?.join("wallpaper");
    let s = fs::read_to_string(p).ok()?;
    let t = s.trim();
    if t.is_empty() { None } else { Some(PathBuf::from(t)) }
}

/// Print the section header for a theme subcommand.
fn announce_header(sub: &str) {
    println!("{}", term::header(&format!("tezca theme {sub}")));
    println!();
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Strip ANSI SGR sequences so captured error text prints cleanly.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until the SGR terminator 'm' (or end).
            while let Some(&n) = chars.peek() {
                chars.next();
                if n == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
