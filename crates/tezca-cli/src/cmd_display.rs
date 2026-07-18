//! `tezca display` — per-monitor mode / scale / position + hardware brightness.
//!
//!   list [--machine]   enumerate monitors (current mode, scale, available modes)
//!   set <name> [--mode WxH@R] [--scale S] [--pos XxY] [--transform N]
//!   reset <name>       drop the monitor override and reload
//!   brightness <name> [0-100]   read / set DDC/CI brightness (external monitors)
//!
//! Mode/scale/pos changes apply live via `hyprctl keyword monitor …` and persist
//! in the managed block of local.conf (survive reload/relogin). A bad mode is
//! always recoverable with `hyprctl reload` or `tezca display reset <name>`.
//!
//! Brightness uses `ddcutil` (DDC/CI over the monitor's i2c bus) since desktop
//! monitors have no backlight sysfs. The Hyprland output name → i2c bus mapping
//! is cached at ~/.cache/tezca/ddc.map so we skip a slow `ddcutil detect` on
//! every call.

use crate::{hypr, managed, term};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// Monitor model + `hyprctl monitors` parsing
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Monitor {
    name: String,
    desc: String,
    res: String,   // "3440x1440"
    rate: String,  // "165.00"
    pos: String,   // "0x0"
    scale: String, // "1.00"
    transform: String,
    vrr: String,
    modes: Vec<String>, // "3440x1440@165.00"
    disabled: bool,
}

/// Parse plain `hyprctl monitors` into structured monitors.
fn parse_monitors(text: &str) -> Vec<Monitor> {
    let mut mons: Vec<Monitor> = Vec::new();
    let mut cur: Option<Monitor> = None;
    let mut saw_mode = false;

    for raw in text.lines() {
        // "Monitor DP-1 (ID 0):"
        if let Some(rest) = raw.strip_prefix("Monitor ") {
            if let Some(m) = cur.take() {
                mons.push(m);
            }
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            cur = Some(Monitor { name, ..Default::default() });
            saw_mode = false;
            continue;
        }
        let Some(m) = cur.as_mut() else { continue };
        let line = raw.trim();

        // The first indented line is the active mode: "3440x1440@165.00000 at 0x0".
        if !saw_mode && line.contains('@') && line.contains(" at ") {
            if let Some((mode, pos)) = line.split_once(" at ") {
                if let Some((res, rate)) = mode.split_once('@') {
                    m.res = res.trim().to_string();
                    m.rate = fmt_rate(rate.trim());
                }
                m.pos = pos.trim().to_string();
            }
            saw_mode = true;
            continue;
        }
        if let Some(v) = line.strip_prefix("description: ") {
            m.desc = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("scale: ") {
            m.scale = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("transform: ") {
            m.transform = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("vrr: ") {
            m.vrr = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("disabled: ") {
            m.disabled = v.trim() == "true";
        } else if let Some(v) = line.strip_prefix("availableModes: ") {
            m.modes = v
                .split_whitespace()
                .filter_map(|tok| {
                    let (res, rate) = tok.split_once('@')?;
                    Some(format!("{res}@{}", fmt_rate(rate.trim_end_matches("Hz"))))
                })
                .collect();
            dedup_keep_order(&mut m.modes);
        }
    }
    if let Some(m) = cur.take() {
        mons.push(m);
    }
    mons
}

/// Normalize a refresh rate to two decimals: "165.00000" / "165" → "165.00".
fn fmt_rate(s: &str) -> String {
    match s.parse::<f64>() {
        Ok(n) => format!("{n:.2}"),
        Err(_) => s.to_string(),
    }
}

fn dedup_keep_order(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
}

fn monitors() -> Result<Vec<Monitor>, String> {
    let out = Command::new("hyprctl")
        .arg("monitors")
        .output()
        .map_err(|e| format!("failed to run hyprctl: {e}"))?;
    if !out.status.success() {
        return Err("hyprctl monitors failed".into());
    }
    Ok(parse_monitors(&String::from_utf8_lossy(&out.stdout)))
}

fn find<'a>(mons: &'a [Monitor], name: &str) -> Option<&'a Monitor> {
    mons.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("list") => cmd_list(args.get(1..).unwrap_or(&[])),
        Some("set") => cmd_set(&args[1..]),
        Some("reset") => cmd_reset(&args[1..]),
        Some("brightness") | Some("bri") => cmd_brightness(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown display subcommand: {other}\n  try: list · set <name> … · reset <name> · brightness <name> [0-100]"
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

fn cmd_list(args: &[&str]) -> Result<(), String> {
    let mons = monitors()?;
    let machine = args.iter().any(|a| *a == "--machine" || *a == "-m");
    if machine {
        for m in &mons {
            if m.disabled {
                continue;
            }
            println!("@monitor {}", m.name);
            println!("desc={}", m.desc);
            println!("res={}", m.res);
            println!("rate={}", m.rate);
            println!("pos={}", m.pos);
            println!("scale={}", m.scale);
            println!("transform={}", m.transform);
            println!("vrr={}", m.vrr);
            println!("modes={}", m.modes.join(" "));
        }
        return Ok(());
    }
    println!("{}", term::header("tezca display"));
    println!();
    for m in &mons {
        if m.disabled {
            continue;
        }
        println!(
            "  {} {}  {}",
            term::green("●"),
            term::bold(&m.name),
            term::dim(&format!("{}@{} @ {}  scale {}", m.res, m.rate, m.pos, m.scale))
        );
        println!("    {}", term::dim(&m.desc));
    }
    println!();
    Ok(())
}

/// `tezca display set <name> [--mode WxH@R] [--scale S] [--pos XxY] [--transform N]`
fn cmd_set(args: &[&str]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .copied()
        .ok_or("usage: tezca display set <name> [--mode WxH@R] [--scale S] [--pos XxY]")?;
    if !hypr::in_session() {
        return Err("not in a Hyprland session (nothing to apply)".into());
    }

    let mons = monitors()?;
    let cur = find(&mons, name).ok_or_else(|| format!("no monitor named '{name}'"))?;

    // Start from the live values so unspecified fields are preserved verbatim.
    let mut mode = format!("{}@{}", cur.res, cur.rate);
    let mut scale = cur.scale.clone();
    let mut pos = cur.pos.clone();
    let mut transform = cur.transform.clone();

    let mut it = args[1..].iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--mode" => mode = it.next().ok_or("--mode needs a value like 3440x1440@165")?.to_string(),
            "--scale" => scale = it.next().ok_or("--scale needs a value")?.to_string(),
            "--pos" => pos = it.next().ok_or("--pos needs a value like 0x0")?.to_string(),
            "--transform" => transform = it.next().ok_or("--transform needs 0-7")?.to_string(),
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    // monitor = NAME, RES@RATE, POS, SCALE [, transform, N]
    let mut spec = format!("{name},{mode},{pos},{scale}");
    if transform != "0" && !transform.is_empty() {
        spec.push_str(&format!(",transform,{transform}"));
    }

    hypr::keyword("monitor", &spec).map_err(|e| format!("hyprctl keyword monitor: {e}"))?;
    managed::set(&format!("monitor:{name}"), &format!("monitor = {spec}"))?;
    println!("  {} {name}  {}", term::green("✓"), term::dim(&spec));
    Ok(())
}

fn cmd_reset(args: &[&str]) -> Result<(), String> {
    let name = args.first().copied().ok_or("usage: tezca display reset <name>")?;
    managed::remove(&format!("monitor:{name}"))?;
    if hypr::in_session() {
        hypr::reload()?;
    }
    println!("  {} reset {name} to the shipped config", term::green("✓"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Brightness (ddcutil / DDC-CI)
// ---------------------------------------------------------------------------

fn cmd_brightness(args: &[&str]) -> Result<(), String> {
    if !which("ddcutil") {
        return Err("ddcutil not found — install it for external-monitor brightness (`paru -S ddcutil`)".into());
    }
    // `brightness` / `brightness list` → print NAME=VALUE for every DDC monitor.
    if args.is_empty() || args[0] == "list" {
        let map = ddc_map(false)?;
        for (name, bus) in &map {
            if let Some(v) = ddc_get(*bus) {
                println!("{name}={v}");
            }
        }
        return Ok(());
    }

    let refresh = args.iter().any(|a| *a == "--refresh");
    let positional: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
    let name = positional.first().ok_or("usage: tezca display brightness <name> [0-100]")?;

    let map = ddc_map(refresh)?;
    let bus = map
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| *b)
        .ok_or_else(|| format!("'{name}' has no DDC/CI bus (not a DDC-capable monitor?)"))?;

    match positional.get(1) {
        // Read.
        None => {
            let v = ddc_get(bus).ok_or("could not read brightness")?;
            println!("{v}");
            Ok(())
        }
        // Set.
        Some(val) => {
            let n: i32 = val.parse().map_err(|_| "brightness must be an integer 0-100")?;
            let n = n.clamp(0, 100);
            let out = Command::new("ddcutil")
                .args(["setvcp", "10", &n.to_string(), "--bus", &bus.to_string()])
                .output()
                .map_err(|e| format!("ddcutil setvcp: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
    }
}

/// Read VCP 0x10 (brightness) on a bus → current value, or None.
/// `ddcutil getvcp 10 --bus N --brief` → "VCP 10 C <current> <max>".
fn ddc_get(bus: u32) -> Option<String> {
    let out = Command::new("ddcutil")
        .args(["getvcp", "10", "--bus", &bus.to_string(), "--brief"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let cur = s.split_whitespace().nth(3)?;
    cur.parse::<i32>().ok().map(|n| n.to_string())
}

/// Hyprland output name → i2c bus number, cached at ~/.cache/tezca/ddc.map.
/// Rebuilt (via `ddcutil detect`) when the cache is missing or `refresh`.
fn ddc_map(refresh: bool) -> Result<Vec<(String, u32)>, String> {
    let cache = cache_path()?;
    if !refresh {
        if let Ok(text) = fs::read_to_string(&cache) {
            let map = parse_ddc_map(&text);
            if !map.is_empty() {
                return Ok(map);
            }
        }
    }
    let out = Command::new("ddcutil")
        .args(["detect", "--brief"])
        .output()
        .map_err(|e| format!("ddcutil detect: {e}"))?;
    if !out.status.success() {
        return Err("ddcutil detect failed (check i2c permissions / DDC support)".into());
    }
    let map = parse_ddc_detect(&String::from_utf8_lossy(&out.stdout));
    // Persist the cache (best-effort).
    if let Some(dir) = cache.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let body: String = map.iter().map(|(n, b)| format!("{n}\t{b}\n")).collect();
    let _ = fs::write(&cache, body);
    Ok(map)
}

fn parse_ddc_detect(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut bus: Option<u32> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("Display ") {
            bus = None;
        } else if let Some(rest) = l.strip_prefix("I2C bus:") {
            // "/dev/i2c-3" → 3
            bus = rest
                .trim()
                .rsplit('-')
                .next()
                .and_then(|n| n.trim().parse().ok());
        } else if let Some(rest) = l.strip_prefix("DRM connector:") {
            // "card1-DP-1" → "DP-1"
            if let Some(b) = bus {
                let conn = rest.trim();
                if let Some((_, name)) = conn.split_once('-') {
                    out.push((name.to_string(), b));
                }
            }
        }
    }
    out
}

fn parse_ddc_map(text: &str) -> Vec<(String, u32)> {
    text.lines()
        .filter_map(|l| {
            let (n, b) = l.split_once('\t')?;
            Some((n.trim().to_string(), b.trim().parse().ok()?))
        })
        .collect()
}

fn cache_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("neither $XDG_CACHE_HOME nor $HOME is set")?;
    Ok(base.join("tezca").join("ddc.map"))
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn print_help() {
    println!("{}", term::header("tezca display"));
    println!();
    println!("  {}                 list monitors + available modes", term::cyan("list"));
    println!("  {}  set mode/scale/position (live + persisted)", term::cyan("set <name> --mode WxH@R"));
    println!("  {}          revert a monitor to the shipped config", term::cyan("reset <name>"));
    println!("  {}   read / set DDC/CI brightness (0-100)", term::cyan("brightness <name> [val]"));
}
