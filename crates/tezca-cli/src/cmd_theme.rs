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

use crate::{atomic, repo, term, util};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The generated files every component imports from ~/.config/tezca/current/.
const FILES: &[&str] = &[
    "colors.css",            // GTK: tezca-bar, swaync, Walker
    "colors-alacritty.toml", // Alacritty (the terminal)
    // Lua data table since the Hyprland config moved off hyprlang; loaded by
    // hypr/conf.d/decoration.lua (borders/shadows).
    "colors-hypr.lua",
    // Still hyprlang: hyprlock keeps the .conf format, so this one does not move.
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
        // Machine-readable name list (one per line) for scripts + tezca-settings.
        Some("names") => cmd_names(),
        // Machine-readable records (name, label, swatch colours) for the
        // Settings theme cards, which draw a real swatch strip per theme.
        Some("info") => cmd_info(),
        Some("set") => match it.next() {
            Some(name) => cmd_set(name),
            None => Err("usage: tezca theme set <name>".into()),
        },
        Some("wallpaper") | Some("wall") => cmd_wallpaper(&args[1..]),
        Some("derive") => cmd_derive(it.next()),
        Some("reload") => cmd_reload(),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown theme subcommand: {other}\n  try: list · set <name> · wallpaper <img> · derive on|off · reload"
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
        println!("  {} seeding default theme {}", term::dim("theme:"), term::cyan(DEFAULT_THEME));
    }
    apply_curated(DEFAULT_THEME, Opts { set_wallpaper: false, reload: false, announce: false })
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

/// Bare curated-theme names, one per line. Consumed by scripts/theme-select.sh
/// and the tezca-settings Appearance tab — no decoration, no ANSI.
fn cmd_names() -> Result<(), String> {
    for (name, _desc) in curated_themes()? {
        println!("{name}");
    }
    Ok(())
}

/// One record per curated theme, for the Settings theme cards.
///
/// The cards draw a real swatch strip, so they need the palette itself and not
/// just the name — and resolving `themes/<name>/colors.css` is the CLI's job,
/// since it is the only side that knows where the repo lives.
fn cmd_info() -> Result<(), String> {
    let root = repo::root()?;
    let active = read_state();
    for (name, desc) in curated_themes()? {
        let dir = root.join("themes").join(&name);
        let meta = read_meta(&dir.join("theme.meta")).unwrap_or_default();
        println!("@theme");
        println!("name={name}");
        println!("label={}", meta_get(&meta, "label").unwrap_or(&capitalize(&name)));
        println!("description={desc}");
        println!("mode={}", meta_get(&meta, "mode").unwrap_or("dark"));
        println!("active={}", active.as_deref() == Some(name.as_str()));
        if let Some(w) = meta_get(&meta, "wallpaper") {
            println!("wallpaper={}", resolve_wallpaper(&root, w).display());
        }
        // The four bars the card's swatch strip draws, widest first.
        let colors = read_tokens(&dir.join("colors.css"));
        for token in ["tz_base", "tz_surface", "tz_accent", "tz_gold", "tz_urgent", "tz_text"] {
            if let Some(hex) = colors.iter().find(|(k, _)| k == token) {
                println!("{}={}", token.trim_start_matches("tz_"), hex.1);
            }
        }
    }
    Ok(())
}

/// `@define-color <name> <value>;` pairs out of a GTK palette file.
fn read_tokens(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("@define-color")?;
            let rest = rest.trim().strip_suffix(';')?;
            let (k, v) = rest.split_once(char::is_whitespace)?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn cmd_set(name: &str) -> Result<(), String> {
    apply_curated(name, Opts { set_wallpaper: true, reload: true, announce: true })
}

/// `tezca theme wallpaper <img> [--derive|--no-derive]`.
///
/// Whether a new picture also re-derives the palette is a preference, not a
/// property of the command: someone running a curated theme wants the picture to
/// change without matugen repainting the desktop under them. The flags force
/// either behaviour for scripts that need to be explicit.
fn cmd_wallpaper(args: &[&str]) -> Result<(), String> {
    let mut img: Option<&str> = None;
    let mut derive: Option<bool> = None;
    for a in args {
        match *a {
            "--derive" => derive = Some(true),
            "--no-derive" | "--keep-palette" => derive = Some(false),
            other if !other.starts_with('-') => img = Some(other),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let img = img.ok_or("usage: tezca theme wallpaper <image> [--no-derive]")?;
    let opts = Opts { set_wallpaper: true, reload: true, announce: true };
    if derive.unwrap_or_else(read_derive) {
        apply_dynamic(img, opts)
    } else {
        set_picture_only(img, &opts)
    }
}

/// Paint `img` and record it as the session wallpaper, leaving the palette — and
/// so `theme.state` — exactly as it was.
fn set_picture_only(img: &str, opts: &Opts) -> Result<(), String> {
    let abs = fs::canonicalize(img).map_err(|e| format!("cannot read image '{img}': {e}"))?;
    let current = current_dir()?;
    fs::create_dir_all(&current)
        .map_err(|e| format!("cannot create {}: {e}", current.display()))?;

    if opts.announce {
        announce_header("wallpaper");
        println!("  {} {}", term::dim("picture:"), term::cyan(&abs.display().to_string()));
        println!("  {}", term::dim("palette unchanged (`tezca theme derive on` to re-derive)"));
    }
    atomic::write(&current.join("wallpaper"), &format!("{}\n", abs.display()))?;
    set_wallpaper(&abs);
    crate::cmd_wallpaper::apply_overrides();
    if opts.announce {
        println!();
        println!("  {} wallpaper set", term::green("done:"));
    }
    Ok(())
}

/// `tezca theme derive [on|off]` — whether a new wallpaper re-derives the palette.
fn cmd_derive(arg: Option<&str>) -> Result<(), String> {
    match arg {
        None => {
            println!("{}", if read_derive() { "on" } else { "off" });
            Ok(())
        }
        Some(v) => {
            let on = match v {
                "on" | "true" | "yes" | "1" => true,
                "off" | "false" | "no" | "0" => false,
                other => return Err(format!("expected on|off, got '{other}'")),
            };
            atomic::write(&derive_path()?, if on { "true\n" } else { "false\n" })?;
            println!(
                "  {} new wallpapers {} the palette",
                term::green("✓"),
                if on { "re-derive" } else { "leave" }
            );
            Ok(())
        }
    }
}

fn derive_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("wallpaper-derive"))
}

/// The stored preference, defaulting to on — matugen theming is the headline
/// behaviour of `tezca theme`, so an unset preference keeps it.
pub fn read_derive() -> bool {
    derive_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|s| s.trim() != "false")
        .unwrap_or(true)
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
        return Err(format!("no curated theme '{name}' ({} not found)", meta_path.display()));
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
        fs::copy(&src, &dst).map_err(|e| format!("cannot write {}: {e}", dst.display()))?;
    }

    // Resolve the theme's wallpaper (relative to wallpapers/, or an abs path).
    let wallpaper = meta_get(&meta, "wallpaper").map(|w| resolve_wallpaper(&root, w));
    finalize(&current, wallpaper.as_deref(), name, &opts)
}

// ---------------------------------------------------------------------------
// Dynamic: matugen renders templates/ → current/
// ---------------------------------------------------------------------------

fn apply_dynamic(img: &str, opts: Opts) -> Result<(), String> {
    if !util::has("matugen") {
        return Err("matugen not found — install it for dynamic theming (`paru -S matugen`)".into());
    }
    let root = repo::root()?;
    let templates = root.join("templates");
    let img_abs = fs::canonicalize(img).map_err(|e| format!("cannot read image '{img}': {e}"))?;

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
        .arg("-c")
        .arg(&mcfg)
        .arg("image")
        .arg(&img_abs)
        .arg("--prefer")
        .arg("saturation") // non-interactive: no TTY to disambiguate
        .arg("-m")
        .arg("dark")
        .output()
        .map_err(|e| format!("failed to run matugen: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // matugen colorizes errors; strip the noisiest control chars for a clean line.
        let msg = err
            .lines()
            .rev()
            .find(|l| l.contains("rror") || l.contains("ailed"))
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
        atomic::write(&hl, &patched)?;
    }

    // Record the active wallpaper (autostart reads this) and theme state.
    if let Some(wp) = wallpaper {
        atomic::write(&current.join("wallpaper"), &format!("{}\n", wp.display()))?;
    }
    atomic::write(&current.join("theme.state"), &format!("{state}\n"))?;

    if opts.set_wallpaper {
        if let Some(wp) = wallpaper {
            if wp.is_file() {
                set_wallpaper(wp);
                // Re-apply per-monitor overrides on top of the fresh global
                // image so a monitor keeps its own picture across theme switches.
                crate::cmd_wallpaper::apply_overrides();
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

    // tezca-bar — SIGUSR2 re-reads colors.css (CSS + parsed palette) live, no
    // restart. Its 9-char comm matches `pkill -x` cleanly.
    report("bar", signal("tezca-bar", "USR2"));

    // swaync — reload the CSS via its control client.
    if util::has("swaync-client") {
        report("swaync", run_ok("swaync-client", &["--reload-css"]));
    } else {
        report("swaync", Outcome::Skipped("swaync-client not found".into()));
    }

    // Alacritty — the default terminal. `live_config_reload` (alacritty.toml)
    // watches the imported palette, so open terminals recolor on their own the
    // moment the file lands. Nothing to signal; report it so the absence of a
    // line here doesn't read as "the terminal was forgotten".
    report(
        "alacritty",
        if proc_running("alacritty") {
            Outcome::Done("live_config_reload picks it up".into())
        } else {
            Outcome::Skipped("not running".into())
        },
    );

    // Walker now runs as a resident GApplication service (autostart.conf) so the
    // launcher opens in ~90ms instead of cold-starting per keypress. The tradeoff
    // is that it no longer re-reads the theme CSS on launch — it has no reload
    // signal and no config watcher — so a theme switch has to restart it.
    report("walker", reload_walker());

    // tezca-dock re-reads the theme palette on SIGUSR2 — a live recolor with no
    // restart and no flicker.
    report("dock", reload_dock());
}

/// Tell the running dock to re-read the theme palette (SIGUSR2). Best-effort:
/// tezca-dock's name is ≤15 chars so `-x` matches its comm cleanly. No restart,
/// so the recolor is flicker-free.
fn reload_dock() -> Outcome {
    if !proc_running("tezca-dock") {
        return Outcome::Skipped("not running".into());
    }
    match Command::new("pkill").arg("-USR2").arg("-x").arg("tezca-dock").status() {
        Ok(s) if s.success() => Outcome::Done("palette reloaded".into()),
        Ok(_) => Outcome::Skipped("not running".into()),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// `pkill -f` pattern matching the resident walker service (autostart.conf).
///
/// Matched by full command line, not by name: `pkill -x walker` would also hit a
/// transient client instance — the ~90ms process a keybind spawns to activate the
/// service — and `--gapplication-service` is the distinctive part.
///
/// The leading `[^ ]*` is load-bearing: systemd/uwsm exec the service by absolute
/// path, so the live cmdline is `/usr/bin/walker --gapplication-service`, not
/// `walker …`. It still won't match the `uwsm app -- walker …` launcher wrapper,
/// since that has spaces before `walker`.
///
/// Shared with `cmd_doctor`, which reports whether the service is up — one source
/// of truth so the two can't drift apart.
pub const WALKER_SERVICE_PATTERN: &str = "^[^ ]*walker --gapplication-service$";

/// True if the resident walker service is alive.
pub fn walker_service_running() -> bool {
    Command::new("pkill")
        .args(["-0", "-f", WALKER_SERVICE_PATTERN])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Restart the resident walker service so it picks up the new theme CSS.
///
/// If the service isn't running, walker is being cold-started per keypress and
/// will read the new CSS on its own — nothing to do.
fn reload_walker() -> Outcome {
    if !walker_service_running() {
        return Outcome::Skipped("service not running — re-reads on next launch".into());
    }

    if let Err(e) = Command::new("pkill").arg("-f").arg(WALKER_SERVICE_PATTERN).status() {
        return Outcome::Failed(e.to_string());
    }

    // Relaunch through uwsm so the service lands back in the same systemd slice
    // autostart.conf put it in. Detached: the service is long-lived, so waiting
    // on it would hang the theme switch.
    let spawned = Command::new("uwsm")
        .args(["app", "--", "walker", "--gapplication-service"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => Outcome::Done("service restarted".into()),
        Err(e) => Outcome::Failed(format!("restart failed: {e}")),
    }
}

/// True if at least one process with the exact name `name` is alive.
fn proc_running(name: &str) -> bool {
    Command::new("pkill")
        .arg("-0")
        .arg("-x")
        .arg(name)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    if !util::has("awww") {
        report("wallpaper", Outcome::Skipped("awww not found".into()));
        return;
    }
    let o = Command::new("awww")
        .arg("img")
        .arg(path)
        // The fit mode is `tezca wallpaper`'s setting, but it has to apply to the
        // global image too — otherwise the picture Settings previews as "fit"
        // comes back cropped the moment a theme switch repaints it.
        .arg("--resize")
        .arg(crate::cmd_wallpaper::resize_arg())
        .arg("--transition-type")
        .arg("grow")
        .arg("--transition-pos")
        .arg("center")
        .output();
    match o {
        Ok(s) if s.status.success() => report("wallpaper", Outcome::Done(String::new())),
        Ok(s) => report(
            "wallpaper",
            Outcome::Failed(
                strip_ansi(String::from_utf8_lossy(&s.stderr).trim())
                    .lines()
                    .next()
                    .unwrap_or("awww img failed")
                    .to_string(),
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
    fs::create_dir_all(&cache).map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
    let cfg_path = cache.join("matugen.toml");

    let mut body = String::from("# Generated by `tezca theme` — do not edit.\n[config]\n\n");
    for f in FILES {
        // matugen renders only file-based templates; a plain key per file.
        let key = f.replace(['.', '-'], "_");
        body.push_str(&format!("[templates.{key}]\n"));
        body.push_str(&format!("input_path = \"{}\"\n", templates.join(f).display()));
        body.push_str(&format!("output_path = \"{}\"\n\n", current.join(f).display()));
    }
    atomic::write(&cfg_path, &body)?;
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
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
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
    if t.is_empty() {
        None
    } else {
        Some(PathBuf::from(t))
    }
}

/// Print the section header for a theme subcommand.
fn announce_header(sub: &str) {
    println!("{}", term::header(&format!("tezca theme {sub}")));
    println!();
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

/// `tezca theme --help`.
fn print_help() {
    println!("{}", term::header("tezca theme"));
    println!("{}", term::dim("  wallpaper-driven theming — one image drives every colour"));
    println!();
    println!("  {}             curated themes, marking the active one", term::cyan("list"));
    println!("  {}            bare names, one per line (for scripts)", term::cyan("names"));
    println!("  {}             one record per theme, with its palette", term::cyan("info"));
    println!("  {}       apply a curated palette", term::cyan("set <name>"));
    println!(
        "  {}  set the picture (and, if deriving, the palette)",
        term::cyan("wallpaper <img>")
    );
    println!(
        "  {}      whether a new wallpaper re-derives the palette",
        term::cyan("derive [on|off]")
    );
    println!(
        "  {}           re-apply the active theme and re-send reload signals",
        term::cyan("reload")
    );
    println!();
    println!(
        "{}",
        term::dim("  e.g. tezca theme set obsidian · tezca theme wallpaper ~/Pictures/a.jpg")
    );
}
