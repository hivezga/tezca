//! `tezca dock` — control the bespoke magnifying dock (crates/tezca-dock).
//!
//! The dock is a separate binary (`tezca-dock`) launched at login by
//! conf.d/autostart.conf. This subcommand is a thin lifecycle wrapper so you can
//! start/stop/reload it by hand. Live control uses signals: SIGUSR1 pins it open
//! (also on SUPER+D), SIGUSR2 reloads the palette (sent by `tezca theme`).

use crate::{repo, term};
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = "tezca-dock";

/// The dock's tunable keys and their built-in defaults — mirrors
/// crates/tezca-dock/src/config.rs so `config` reports a complete picture even
/// when dock.toml omits a field. `pinned` is handled separately (a list).
const SCALARS: &[(&str, &str)] = &[
    ("icon_size", "48"),
    ("max_scale", "1.6"),
    ("influence", "110"),
    ("gap", "10"),
    ("pad_x", "12"),
    ("pad_y", "8"),
    ("margin_bottom", "8"),
    ("hotspot_height", "6"),
    ("hide_delay_ms", "350"),
];

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(),
        Some("start") => cmd_start(),
        Some("stop") => cmd_stop(),
        Some("restart") => cmd_restart(),
        Some("toggle") => cmd_toggle(),
        Some("config") => cmd_config(),
        Some("set") => cmd_set(&args[1..]),
        Some(other) => Err(format!(
            "unknown dock subcommand: {other}\n  try: status · start · stop · restart · toggle · config · set"
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

fn cmd_status() -> Result<(), String> {
    println!("{}", term::header("tezca dock"));
    println!();
    if running() {
        println!("  {} {} is running", term::green("●"), term::bold(BIN));
    } else {
        println!("  {} {} is not running", term::dim("○"), term::bold(BIN));
        println!("    {}", term::dim("start it with `tezca dock start`"));
    }
    Ok(())
}

fn cmd_start() -> Result<(), String> {
    if running() {
        println!("  {} {} already running", term::dim("·"), BIN);
        return Ok(());
    }
    spawn()?;
    println!("  {} started {}", term::green("→"), BIN);
    Ok(())
}

fn cmd_stop() -> Result<(), String> {
    if !running() {
        println!("  {} {} not running", term::dim("·"), BIN);
        return Ok(());
    }
    pkill(&["-x", BIN]);
    println!("  {} stopped {}", term::green("✓"), BIN);
    Ok(())
}

fn cmd_restart() -> Result<(), String> {
    if running() {
        pkill(&["-TERM", "-x", BIN]);
        for _ in 0..50 {
            if !running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    spawn()?;
    println!("  {} restarted {}", term::green("✓"), BIN);
    Ok(())
}

/// Pin/unpin the dock (SIGUSR1). Starts it first if it isn't running.
fn cmd_toggle() -> Result<(), String> {
    if !running() {
        return cmd_start();
    }
    pkill(&["-USR1", "-x", BIN]);
    println!("  {} toggled {}", term::green("✓"), BIN);
    Ok(())
}

// --- config (dock.toml) ----------------------------------------------------

fn dock_toml() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca-dock").join("dock.toml"))
}

/// `tezca dock config` — the effective values (file over defaults), one per line
/// (`key = value`, `pinned = a, b, c`). Machine-readable for tezca-settings.
fn cmd_config() -> Result<(), String> {
    let text = std::fs::read_to_string(dock_toml()?).unwrap_or_default();
    for (key, default) in SCALARS {
        let val = read_scalar(&text, key).unwrap_or_else(|| default.to_string());
        println!("{key} = {val}");
    }
    let pinned = read_pinned(&text);
    println!("pinned = {}", pinned.join(", "));
    Ok(())
}

/// `tezca dock set <key> <value> [<key> <value>…]` — edit dock.toml (preserving
/// comments) then restart the dock if it's running so the change takes effect.
fn cmd_set(args: &[&str]) -> Result<(), String> {
    if args.is_empty() || args.len() % 2 != 0 {
        return Err("usage: tezca dock set <key> <value> [<key> <value>…]".into());
    }
    let path = dock_toml()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    for pair in args.chunks(2) {
        let (key, val) = (pair[0], pair[1]);
        if key == "pinned" {
            let list: Vec<String> = val
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| format!("\"{s}\""))
                .collect();
            set_line(&mut lines, "pinned", &format!("[{}]", list.join(", ")));
        } else if SCALARS.iter().any(|(k, _)| *k == key) {
            set_line(&mut lines, key, val);
        } else {
            return Err(format!("unknown dock key: {key}"));
        }
    }

    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(&path, body).map_err(|e| format!("cannot write dock.toml: {e}"))?;

    if running() {
        let _ = cmd_restart();
    } else {
        println!("  {} saved (dock not running — starts with your settings next time)", term::green("✓"));
    }
    Ok(())
}

/// Read a scalar `key = value` (ignoring any trailing `# comment`).
fn read_scalar(text: &str, key: &str) -> Option<String> {
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix('=') {
                let val = after.split('#').next().unwrap_or("").trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// Read the `pinned = [...]` / `pinned = a, b` list.
fn read_pinned(text: &str) -> Vec<String> {
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("pinned") {
            let rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix('=') {
                let v = after.split('#').next().unwrap_or("");
                return v
                    .trim()
                    .trim_matches(|c| c == '[' || c == ']')
                    .split(',')
                    .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Upsert `key = value`, preserving indentation and any trailing `# comment`.
fn set_line(lines: &mut Vec<String>, key: &str, value: &str) {
    for l in lines.iter_mut() {
        let trimmed = l.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix('=') {
                let indent = &l[..l.len() - trimmed.len()];
                let comment = after
                    .split_once('#')
                    .map(|(_, c)| format!("  # {}", c.trim()))
                    .unwrap_or_default();
                *l = format!("{indent}{key} = {value}{comment}");
                return;
            }
        }
    }
    lines.push(format!("{key} = {value}"));
}

// --- helpers ---------------------------------------------------------------

/// Launch the dock. Prefer `uwsm app --` so it lands in the session's systemd
/// slice (matching autostart.conf); fall back to a plain spawn. `uwsm app`
/// detaches the target into its own scope and returns, so tezca-dock outlives
/// this CLI process either way.
fn spawn() -> Result<(), String> {
    if which(BIN).is_none() {
        return Err(format!(
            "{BIN} not found on PATH — build + install it (install.sh) first"
        ));
    }
    let launched = if which("uwsm").is_some() {
        Command::new("uwsm").args(["app", "--", BIN]).spawn()
    } else {
        Command::new(BIN).spawn()
    };
    launched.map_err(|e| format!("failed to launch dock: {e}"))?;
    Ok(())
}

fn running() -> bool {
    Command::new("pkill")
        .args(["-0", "-x", BIN])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pkill(args: &[&str]) {
    let _ = Command::new("pkill").args(args).status();
}

fn which(bin: &str) -> Option<()> {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| ())
}
