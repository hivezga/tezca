//! `tezca doctor` — verify the machine is set up for a correct, buttery
//! NVIDIA + dual-165 Hz Hyprland/uwsm session.

use crate::{repo, term};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Pass,
    Warn,
    Fail,
}

struct Check {
    level: Level,
    label: String,
    detail: String,
}

impl Check {
    fn pass(label: &str, detail: impl Into<String>) -> Self {
        Self { level: Level::Pass, label: label.into(), detail: detail.into() }
    }
    fn warn(label: &str, detail: impl Into<String>) -> Self {
        Self { level: Level::Warn, label: label.into(), detail: detail.into() }
    }
    fn fail(label: &str, detail: impl Into<String>) -> Self {
        Self { level: Level::Fail, label: label.into(), detail: detail.into() }
    }
}

/// Returns process exit code: 0 if no failures, 1 otherwise.
pub fn run() -> i32 {
    println!("{}", term::header("tezca doctor"));
    println!();

    let mut all = Vec::new();
    section("GPU / NVIDIA", nvidia_checks(), &mut all);
    section("Session & environment", session_checks(), &mut all);
    section("Config linkage", config_checks(), &mut all);
    section("Config validity", config_validity_checks(), &mut all);
    section("Displays", monitor_checks(), &mut all);
    section("Component stack", dependency_checks(), &mut all);

    let fails = all.iter().filter(|l| **l == Level::Fail).count();
    let warns = all.iter().filter(|l| **l == Level::Warn).count();

    println!();
    if fails == 0 && warns == 0 {
        println!("{}", term::green("● all checks passed — Tezca is ready"));
    } else {
        println!(
            "{} {} · {} · {}",
            term::bold("summary:"),
            term::green(&format!("{} ok", all.len() - fails - warns)),
            term::yellow(&format!("{warns} warn")),
            term::red(&format!("{fails} fail")),
        );
    }
    if fails == 0 {
        0
    } else {
        1
    }
}

fn section(title: &str, checks: Vec<Check>, sink: &mut Vec<Level>) {
    println!("{}", term::bold(title));
    for c in &checks {
        let (sym, lbl) = match c.level {
            Level::Pass => (term::green("✓"), c.label.clone()),
            Level::Warn => (term::yellow("●"), c.label.clone()),
            Level::Fail => (term::red("✗"), c.label.clone()),
        };
        if c.detail.is_empty() {
            println!("  {sym} {lbl}");
        } else {
            println!("  {sym} {lbl} {}", term::dim(&format!("— {}", c.detail)));
        }
        sink.push(c.level);
    }
    println!();
}

// ---------------------------------------------------------------------------
// GPU / NVIDIA
// ---------------------------------------------------------------------------

fn nvidia_checks() -> Vec<Check> {
    let mut v = Vec::new();

    // Driver present?
    if Path::new("/proc/driver/nvidia/version").exists() {
        let ver = fs::read_to_string("/proc/driver/nvidia/version")
            .ok()
            .and_then(|s| parse_driver_version(&s))
            .unwrap_or_else(|| "unknown".into());
        v.push(Check::pass("NVIDIA kernel driver loaded", format!("driver {ver}")));

        // Explicit sync (the correct path) needs a recent driver (>= 555).
        match parse_major(&ver) {
            Some(major) if major >= 555 => v.push(Check::pass(
                "driver supports explicit sync",
                format!("{major} ≥ 555 — no legacy stutter hacks needed"),
            )),
            Some(major) => v.push(Check::warn(
                "driver may predate explicit sync",
                format!("{major} < 555 — upgrade for the flicker-free path"),
            )),
            None => v.push(Check::warn("could not parse driver major version", "")),
        }
    } else {
        v.push(Check::fail(
            "NVIDIA kernel driver not loaded",
            "/proc/driver/nvidia/version missing",
        ));
    }

    // nvidia_drm modesetting — required for Wayland.
    v.push(modeset_check());

    v
}

/// The modeset sysfs param is root-only (`-r--------`), so a normal-user run
/// usually can't read it. Prefer the direct read; otherwise infer from
/// user-readable evidence (module loaded + live Wayland session + modprobe.d).
fn modeset_check() -> Check {
    match fs::read_to_string("/sys/module/nvidia_drm/parameters/modeset") {
        Ok(s) if s.trim() == "Y" => {
            return Check::pass("nvidia_drm.modeset=1", "kernel modesetting enabled")
        }
        Ok(s) => {
            return Check::fail(
                "nvidia_drm.modeset is off",
                format!("read '{}', need 'Y' (set nvidia_drm.modeset=1)", s.trim()),
            )
        }
        Err(_) => {} // root-only param — fall through to inference
    }

    let loaded = Path::new("/sys/module/nvidia_drm").exists();
    let wayland = std::env::var("XDG_SESSION_TYPE").ok().as_deref() == Some("wayland");
    let via_modprobe = modprobe_sets_modeset();

    if loaded && (wayland || via_modprobe) {
        let why = if wayland {
            "nvidia_drm loaded + Wayland session active"
        } else {
            "nvidia_drm loaded + modprobe.d sets modeset=1"
        };
        Check::pass("nvidia_drm.modeset (inferred on)", why)
    } else if loaded {
        Check::warn(
            "nvidia_drm.modeset unverifiable",
            "param is root-only; `sudo cat /sys/module/nvidia_drm/parameters/modeset` to confirm",
        )
    } else {
        Check::fail("nvidia_drm not loaded", "modesetting cannot be on")
    }
}

/// Scan user-readable modprobe configs for `options nvidia_drm modeset=1`.
fn modprobe_sets_modeset() -> bool {
    let dirs = ["/etc/modprobe.d", "/usr/lib/modprobe.d", "/run/modprobe.d"];
    for d in dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        for entry in rd.flatten() {
            let Ok(text) = fs::read_to_string(entry.path()) else { continue };
            for line in text.lines() {
                let l = line.trim();
                if l.starts_with('#') {
                    continue;
                }
                if l.contains("nvidia_drm") && l.contains("modeset=1") {
                    return true;
                }
            }
        }
    }
    false
}

fn parse_driver_version(s: &str) -> Option<String> {
    // "NVRM version: NVIDIA UNIX ... Kernel Module  560.35.03  ..."
    s.split_whitespace()
        .find(|tok| tok.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && tok.contains('.'))
        .map(|s| s.to_string())
}

fn parse_major(ver: &str) -> Option<u32> {
    ver.split('.').next()?.parse().ok()
}

// ---------------------------------------------------------------------------
// Session & environment
// ---------------------------------------------------------------------------

fn session_checks() -> Vec<Check> {
    let mut v = Vec::new();

    match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") => v.push(Check::pass("session type", "wayland")),
        Some(other) => v.push(Check::warn(
            "session type is not wayland",
            format!("'{other}' — log into the Hyprland (uwsm) session"),
        )),
        None => v.push(Check::warn(
            "not in a graphical session",
            "run doctor from inside the Tezca session for full checks",
        )),
    }

    let in_hypr = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
    if in_hypr {
        v.push(Check::pass("Hyprland is running", ""));
    } else {
        v.push(Check::warn("Hyprland not detected", "monitor checks will be skipped"));
    }

    // Key NVIDIA env vars (only meaningful once uwsm has exported them).
    for (key, want) in [
        ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
        ("LIBVA_DRIVER_NAME", "nvidia"),
    ] {
        match std::env::var(key).ok() {
            Some(val) if val == want => v.push(Check::pass(key, val)),
            Some(val) => v.push(Check::warn(key, format!("is '{val}', expected '{want}'"))),
            None if in_hypr => v.push(Check::warn(key, "unset in session — check uwsm/env")),
            None => v.push(Check::warn(key, "unset (not in Tezca session)")),
        }
    }

    v
}

// ---------------------------------------------------------------------------
// Config linkage
// ---------------------------------------------------------------------------

fn config_checks() -> Vec<Check> {
    let mut v = Vec::new();
    let cfg = match repo::config_home() {
        Ok(c) => c,
        Err(e) => {
            v.push(Check::fail("cannot resolve ~/.config", e));
            return v;
        }
    };

    for (name, file) in [
        ("hypr", "hyprland.conf"),
        ("uwsm", "env"),
    ] {
        let path = cfg.join(name).join(file);
        if path.exists() {
            let linked = cfg.join(name).read_link().is_ok();
            let how = if linked { "symlinked" } else { "present (not a symlink)" };
            v.push(Check::pass(&format!("{name}/{file}"), how));
        } else {
            v.push(Check::fail(
                &format!("{name}/{file}"),
                "missing — run `tezca link`",
            ));
        }
    }

    v
}

// ---------------------------------------------------------------------------
// Config validity
// ---------------------------------------------------------------------------

/// Parse the whole Hyprland config with `Hyprland --verify-config` — it reports
/// every error WITHOUT launching a compositor, so it runs from a tty. This is
/// the gate that catches API drift (renamed/removed options, changed windowrule
/// grammar) *before* a login lands you in a red error overlay with dead keybinds.
fn config_validity_checks() -> Vec<Check> {
    let mut v = Vec::new();

    if !which("Hyprland") {
        v.push(Check::warn("hyprland config not verified", "Hyprland binary not found"));
        return v;
    }

    let out = Command::new("Hyprland").arg("--verify-config").output();
    match out {
        Ok(o) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr),
            );
            // Hyprland prints "config ok" after a clean parse, or one
            // "Config error in file …" line per problem.
            if text.contains("config ok") {
                v.push(Check::pass(
                    "hyprland config valid",
                    "`Hyprland --verify-config` → config ok",
                ));
            } else {
                let errs: Vec<String> = text
                    .lines()
                    .filter(|l| l.contains("Config error in file"))
                    .map(|l| l.trim().to_string())
                    .collect();
                let count = errs.len();
                let first = errs
                    .into_iter()
                    .next()
                    // Trim the noisy absolute-path prefix for a readable one-liner.
                    .map(|l| l.replace("Config error in file ", ""))
                    .unwrap_or_else(|| {
                        "run `Hyprland --verify-config` to see the errors".into()
                    });
                let detail = if count > 1 {
                    format!("{count} errors — first: {first}")
                } else {
                    first
                };
                v.push(Check::fail("hyprland config INVALID", detail));
            }
        }
        Err(e) => v.push(Check::warn("hyprland config not verified", e.to_string())),
    }

    v
}

// ---------------------------------------------------------------------------
// Displays
// ---------------------------------------------------------------------------

fn monitor_checks() -> Vec<Check> {
    let mut v = Vec::new();
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        v.push(Check::warn(
            "monitor checks skipped",
            "not in a Hyprland session",
        ));
        return v;
    }

    let out = Command::new("hyprctl").arg("monitors").output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let count = text.matches("Monitor ").count();
            match count {
                0 => v.push(Check::fail("no monitors reported", "")),
                2 => v.push(Check::pass("monitor count", "2 (dual display)")),
                n => v.push(Check::warn("monitor count", format!("{n} (expected 2)"))),
            }
            // Refresh rate — hyprctl prints e.g. "3440x1440@164.90100Hz".
            let hi_refresh = text
                .split_whitespace()
                .filter_map(|t| t.strip_suffix("Hz"))
                .filter_map(|t| t.split('@').nth(1))
                .filter_map(|t| t.parse::<f32>().ok())
                .any(|hz| hz >= 160.0);
            if hi_refresh {
                v.push(Check::pass("high refresh active", "≥160 Hz mode detected"));
            } else {
                v.push(Check::warn(
                    "no ≥160 Hz mode detected",
                    "check monitors.conf refresh rates",
                ));
            }
        }
        Ok(o) => v.push(Check::warn(
            "hyprctl monitors failed",
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        )),
        Err(e) => v.push(Check::warn("hyprctl not runnable", e.to_string())),
    }

    v
}

// ---------------------------------------------------------------------------
// Component stack
// ---------------------------------------------------------------------------

fn dependency_checks() -> Vec<Check> {
    // (probe-name, required-for-phase-1?, fallback-paths)
    // Most components are PATH binaries, but some aren't: hyprpolkitagent is a
    // /usr/lib helper + systemd user service, and swww shipped as `awww` (its
    // renamed successor) whose daemon binary is `awww-daemon`. Probe fallbacks
    // so a working install isn't reported as missing.
    let deps: &[(&str, bool, &[&str])] = &[
        ("Hyprland", true, &[]),
        ("uwsm", true, &[]),
        ("hyprctl", true, &[]),
        ("kitty", true, &[]),
        ("hyprpolkitagent", false, &["/usr/lib/hyprpolkitagent/hyprpolkitagent"]),
        ("waybar", false, &[]),
        ("swaync", false, &[]),
        ("walker", false, &[]),
        ("awww", false, &["/usr/bin/swww-daemon"]), // wallpaper daemon (swww successor)
        ("matugen", false, &[]),
        ("hyprlock", false, &[]),
        ("hypridle", false, &[]),
    ];

    deps.iter()
        .map(|(bin, required, alts)| {
            let present = which(bin) || alts.iter().any(|p| Path::new(p).exists());
            if present {
                Check::pass(bin, "installed")
            } else if *required {
                Check::fail(bin, "missing — required for a bootable session")
            } else {
                Check::warn(bin, "missing (install for the aesthetic stack)")
            }
        })
        .collect()
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
