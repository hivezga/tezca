//! `tezca bar` — control the bespoke top menubar (crates/tezca-bar).
//!
//! The bar is a separate binary (`tezca-bar`) launched at login by
//! conf.d/autostart.conf, replacing Waybar. This subcommand is a thin lifecycle
//! wrapper — parallel to `tezca dock` — so you can start/stop/reload it by hand.
//! Live control uses signals: SIGUSR1 toggles visibility, SIGUSR2 reloads the
//! palette (sent by `tezca theme`).

use crate::{repo, term};
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = "tezca-bar";

/// The bar's tunable keys and built-in defaults — mirrors
/// crates/tezca-bar/src/config.rs so `config` reports a complete picture even
/// when config.toml omits a field.
const SCALARS: &[(&str, &str)] = &[
    ("shape", "floating"),
    ("height", "40"),
    ("margin_top", "6"),
    ("margin_side", "10"),
    ("cpu_interval", "3"),
    ("mem_interval", "5"),
    ("gpu_interval", "3"),
    ("net_interval", "5"),
    ("clock_format", "%a %d %b   %H:%M"),
    ("compact_width", "3000"),
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
            "unknown bar subcommand: {other}\n  try: status · start · stop · restart · toggle · config · set"
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
    println!("{}", term::header("tezca bar"));
    println!();
    if running() {
        println!("  {} {} is running", term::green("●"), term::bold(BIN));
    } else {
        println!("  {} {} is not running", term::dim("○"), term::bold(BIN));
        println!("    {}", term::dim("start it with `tezca bar start`"));
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

/// Toggle bar visibility (SIGUSR1). Starts it first if it isn't running.
fn cmd_toggle() -> Result<(), String> {
    if !running() {
        return cmd_start();
    }
    pkill(&["-USR1", "-x", BIN]);
    println!("  {} toggled {}", term::green("✓"), BIN);
    Ok(())
}

// --- config (config.toml) --------------------------------------------------

fn bar_toml() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca-bar").join("config.toml"))
}

/// `tezca bar config` — the effective values (file over defaults), one per line
/// (`key = value`). Machine-readable for tezca-settings.
fn cmd_config() -> Result<(), String> {
    let text = std::fs::read_to_string(bar_toml()?).unwrap_or_default();
    for (key, default) in SCALARS {
        let val = read_scalar(&text, key).unwrap_or_else(|| default.to_string());
        println!("{key} = {val}");
    }
    Ok(())
}

/// `tezca bar set <key> <value> [<key> <value>…]` — edit config.toml (preserving
/// comments) then restart the bar if it's running so the change takes effect.
fn cmd_set(args: &[&str]) -> Result<(), String> {
    if args.is_empty() || args.len() % 2 != 0 {
        return Err("usage: tezca bar set <key> <value> [<key> <value>…]".into());
    }
    let path = bar_toml()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    for pair in args.chunks(2) {
        let (key, val) = (pair[0], pair[1]);
        if SCALARS.iter().any(|(k, _)| *k == key) {
            set_line(&mut lines, key, val);
        } else {
            return Err(format!("unknown bar key: {key}"));
        }
    }

    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(&path, body).map_err(|e| format!("cannot write config.toml: {e}"))?;

    if running() {
        let _ = cmd_restart();
    } else {
        println!("  {} saved (bar not running — starts with your settings next time)", term::green("✓"));
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
                    return Some(val.trim_matches(|c| c == '"' || c == '\'').to_string());
                }
            }
        }
    }
    None
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

/// Launch the bar. Prefer `uwsm app --` so it lands in the session's systemd
/// slice (matching autostart.conf), and wrap in `setsid` so the bar starts in a
/// fresh session — otherwise the launched process stays in this CLI's (and its
/// terminal's) process group and dies when that terminal closes. At login via
/// `exec-once` this is moot, but `tezca bar start` from a shell needs it.
fn spawn() -> Result<(), String> {
    if which(BIN).is_none() {
        return Err(format!(
            "{BIN} not found on PATH — build + install it (install.sh) first"
        ));
    }
    let has_setsid = which("setsid").is_some();
    let has_uwsm = which("uwsm").is_some();
    let mut cmd = if has_setsid {
        let mut c = Command::new("setsid");
        if has_uwsm {
            c.args(["uwsm", "app", "--", BIN]);
        } else {
            c.arg(BIN);
        }
        c
    } else if has_uwsm {
        let mut c = Command::new("uwsm");
        c.args(["app", "--", BIN]);
        c
    } else {
        Command::new(BIN)
    };
    cmd.spawn().map_err(|e| format!("failed to launch bar: {e}"))?;
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
