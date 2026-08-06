//! `tezca wallpaper` — per-monitor wallpaper overrides.
//!
//! The GLOBAL wallpaper (the one that drives the palette via matugen) is owned
//! by `tezca theme`. This command layers per-output overrides ON TOP: a distinct
//! image on one monitor, painted with awww's `--outputs`, purely visual (it does
//! NOT re-theme). Overrides persist in ~/.config/tezca/monitor-wallpapers and are
//! re-applied on login (autostart calls `tezca wallpaper apply`) and after every
//! theme switch (cmd_theme calls apply_overrides), so a monitor keeps its picture.
//!
//!   set <img> --monitor <NAME>   override one output
//!   clear --monitor <NAME>       drop the override (back to the global image)
//!   clear --all                  drop every override
//!   list                         show each monitor's effective wallpaper
//!   fit [mode]                   how an image is scaled onto the screen
//!   apply                        paint global everywhere, then each override

use crate::{atomic, repo, term, util};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How an image is mapped onto a screen, and the awww `--resize` value each
/// choice means. awww has no tiling mode, so the fourth choice centres the image
/// at its own size instead of repeating it.
const FITS: &[(&str, &str, &str)] = &[
    ("fill", "crop", "cover the screen, cropping the overflow"),
    ("fit", "fit", "fit inside the screen, letterboxed"),
    ("stretch", "stretch", "fill the screen, ignoring the aspect ratio"),
    ("center", "no", "centre at the image's own size"),
];

fn state_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("monitor-wallpapers"))
}

fn fit_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("wallpaper-fit"))
}

fn global_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("current").join("wallpaper"))
}

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        Some("set") => cmd_set(&args[1..]),
        Some("clear") | Some("reset") => cmd_clear(&args[1..]),
        None | Some("list") => cmd_list(),
        Some("library") | Some("lib") => cmd_library(),
        Some("fit") => cmd_fit(args.get(1).copied()),
        Some("apply") => cmd_apply(),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown wallpaper subcommand: {other}\n  try: set <img> --monitor <name> · clear · list · fit <mode> · apply"
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

/// `tezca wallpaper set <img> --monitor <NAME>`
fn cmd_set(args: &[&str]) -> Result<(), String> {
    let mut monitor: Option<&str> = None;
    let mut img: Option<&str> = None;
    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--monitor" | "-m" => monitor = it.next(),
            other if !other.starts_with('-') => img = Some(other),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let monitor = monitor.ok_or(
        "set requires --monitor <NAME> (for a global wallpaper use `tezca theme wallpaper <img>`)",
    )?;
    let img = img.ok_or("usage: tezca wallpaper set <img> --monitor <NAME>")?;

    let abs = fs::canonicalize(img).map_err(|e| format!("cannot read image '{img}': {e}"))?;
    paint(&abs, Some(monitor))?;

    let mut overrides = read_overrides()?;
    upsert(&mut overrides, monitor, &abs.to_string_lossy());
    write_overrides(&overrides)?;

    println!(
        "  {} {} → {}",
        term::green("✓"),
        term::bold(monitor),
        term::dim(&abs.display().to_string())
    );
    Ok(())
}

/// `tezca wallpaper clear --monitor <NAME>` / `clear --all`
fn cmd_clear(args: &[&str]) -> Result<(), String> {
    let all = args.contains(&"--all");
    if all {
        write_overrides(&[])?;
        if let Some(g) = read_global() {
            paint(&g, None)?; // repaint global on every output
        }
        println!("  {} cleared all per-monitor overrides", term::green("✓"));
        return Ok(());
    }

    let mut monitor: Option<&str> = None;
    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        if a == "--monitor" || a == "-m" {
            monitor = it.next();
        }
    }
    let monitor = monitor.ok_or("usage: tezca wallpaper clear --monitor <NAME> | --all")?;

    let mut overrides = read_overrides()?;
    overrides.retain(|(n, _)| n != monitor);
    write_overrides(&overrides)?;
    // Restore the global image on that output.
    if let Some(g) = read_global() {
        paint(&g, Some(monitor))?;
    }
    println!("  {} {} back to the global wallpaper", term::green("✓"), term::bold(monitor));
    Ok(())
}

/// `tezca wallpaper list` — machine-readable `NAME<TAB>source<TAB>path`.
fn cmd_list() -> Result<(), String> {
    let overrides = read_overrides()?;
    let global = read_global().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    for name in monitor_names() {
        match overrides.iter().find(|(n, _)| *n == name) {
            Some((_, path)) => println!("{name}\toverride\t{path}"),
            None => println!("{name}\tglobal\t{global}"),
        }
    }
    Ok(())
}

/// `tezca wallpaper library` — every image the picker can offer, one absolute
/// path per line, deduplicated and sorted.
///
/// The same two places `wallpaper.sh` walks — the repo's shipped set and the
/// user's own directory — so cycling with SUPER+ALT+arrows and picking in
/// Settings see one library rather than two.
fn cmd_library() -> Result<(), String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(root) = repo::root() {
        dirs.push(root.join("wallpapers"));
    }
    // Exactly the directory `wallpaper.sh` cycles — the FIRST candidate that
    // exists, not all of them — so SUPER+ALT+arrows and this picker walk one set.
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let candidates: Vec<PathBuf> = std::env::var_os("TEZCA_WALLPAPER_DIR")
        .filter(|s| !s.is_empty())
        .map(|d| vec![PathBuf::from(d)])
        .unwrap_or_else(|| {
            home.iter()
                .flat_map(|h| [h.join("Pictures").join("wallpapers"), h.join("Pictures")])
                .collect()
        });
    if let Some(d) = candidates.into_iter().find(|d| d.is_dir()) {
        dirs.push(d);
    }

    let mut found: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
                continue;
            }
            let abs = fs::canonicalize(&p).unwrap_or(p);
            if !found.contains(&abs) {
                found.push(abs);
            }
        }
    }
    found.sort();
    for p in found {
        println!("{}", p.display());
    }
    Ok(())
}

/// `tezca wallpaper fit [mode]` — read or set how images are scaled, repainting
/// so the change is visible immediately rather than at the next wallpaper switch.
fn cmd_fit(mode: Option<&str>) -> Result<(), String> {
    let Some(mode) = mode else {
        println!("{}", read_fit());
        return Ok(());
    };
    let (name, _, desc) = FITS.iter().find(|(n, _, _)| *n == mode).ok_or_else(|| {
        let names: Vec<&str> = FITS.iter().map(|(n, _, _)| *n).collect();
        format!("unknown fit mode '{mode}' — try: {}", names.join(" · "))
    })?;
    atomic::write(&fit_path()?, &format!("{name}\n"))?;
    println!("  {} {} {}", term::green("✓"), term::bold(name), term::dim(&format!("— {desc}")));
    cmd_apply()
}

/// The stored fit mode, defaulting to awww's own default.
pub fn read_fit() -> String {
    let stored = fit_path().ok().and_then(|p| fs::read_to_string(p).ok());
    let stored = stored.as_deref().map(str::trim).unwrap_or("");
    FITS.iter()
        .find(|(n, _, _)| *n == stored)
        .map(|(n, _, _)| (*n).to_string())
        .unwrap_or_else(|| "fill".into())
}

/// The awww `--resize` value for the stored fit mode. Public so `tezca theme`
/// paints the global wallpaper the same way this command paints an override —
/// one setting, one behaviour, whichever side set the picture.
pub fn resize_arg() -> &'static str {
    let fit = read_fit();
    FITS.iter().find(|(n, _, _)| *n == fit).map(|(_, r, _)| *r).unwrap_or("crop")
}

/// `tezca wallpaper apply` — paint the global wallpaper on all outputs, then each
/// override on its output. Called from session autostart (best-effort).
fn cmd_apply() -> Result<(), String> {
    if let Some(g) = read_global() {
        let _ = paint(&g, None);
    }
    apply_overrides();
    Ok(())
}

/// Re-paint only the per-monitor overrides. Public so `tezca theme` can call it
/// after it sets the global wallpaper, keeping each monitor's picture.
pub fn apply_overrides() {
    let Ok(overrides) = read_overrides() else { return };
    for (name, path) in overrides {
        let p = PathBuf::from(&path);
        if p.is_file() {
            let _ = paint(&p, Some(&name));
        }
    }
}

// ---------------------------------------------------------------------------
// awww + state helpers
// ---------------------------------------------------------------------------

/// Paint `path` via awww, optionally restricted to one output.
fn paint(path: &Path, output: Option<&str>) -> Result<(), String> {
    if !util::has("awww") {
        return Err("awww not found (the wallpaper daemon)".into());
    }
    let mut cmd = Command::new("awww");
    cmd.arg("img").arg(path);
    if let Some(o) = output {
        cmd.arg("--outputs").arg(o);
    }
    cmd.arg("--resize").arg(resize_arg());
    cmd.arg("--transition-type").arg("grow").arg("--transition-pos").arg("center");
    let out = cmd.output().map_err(|e| format!("awww img: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .next()
            .unwrap_or("awww img failed")
            .to_string())
    }
}

fn read_global() -> Option<PathBuf> {
    let s = fs::read_to_string(global_path().ok()?).ok()?;
    let t = s.trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

fn read_overrides() -> Result<Vec<(String, String)>, String> {
    let p = state_path()?;
    let Ok(text) = fs::read_to_string(&p) else { return Ok(Vec::new()) };
    Ok(text
        .lines()
        .filter_map(|l| {
            let (n, path) = l.split_once('\t')?;
            let (n, path) = (n.trim(), path.trim());
            (!n.is_empty() && !path.is_empty()).then(|| (n.to_string(), path.to_string()))
        })
        .collect())
}

fn write_overrides(overrides: &[(String, String)]) -> Result<(), String> {
    let p = state_path()?;
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let body: String = overrides.iter().map(|(n, path)| format!("{n}\t{path}\n")).collect();
    atomic::write(&p, &body)
}

fn upsert(overrides: &mut Vec<(String, String)>, name: &str, path: &str) {
    if let Some(slot) = overrides.iter_mut().find(|(n, _)| n == name) {
        slot.1 = path.to_string();
    } else {
        overrides.push((name.to_string(), path.to_string()));
    }
}

/// Monitor connector names from `hyprctl monitors` (empty if not in a session).
fn monitor_names() -> Vec<String> {
    let Ok(out) = Command::new("hyprctl").arg("monitors").output() else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("Monitor "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

fn print_help() {
    println!("{}", term::header("tezca wallpaper"));
    println!("{}", term::dim("  per-monitor wallpaper overrides (global image → `tezca theme`)"));
    println!();
    println!("  {}  override one output", term::cyan("set <img> --monitor <NAME>"));
    println!("  {}       drop an override (or --all)", term::cyan("clear --monitor <NAME>"));
    println!("  {}                       show each monitor's wallpaper", term::cyan("list"));
    println!("  {}                    every image the picker offers", term::cyan("library"));
    println!(
        "  {}                 how images are scaled ({})",
        term::cyan("fit <mode>"),
        FITS.iter().map(|(n, _, _)| *n).collect::<Vec<_>>().join(" · ")
    );
    println!(
        "  {}                      repaint global + overrides (autostart)",
        term::cyan("apply")
    );
}
