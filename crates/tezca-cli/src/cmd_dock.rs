//! `tezca dock` — control the bespoke magnifying dock (crates/tezca-dock).
//!
//! The dock is a separate binary (`tezca-dock`) launched at login by
//! conf.d/autostart.conf. This subcommand is a thin lifecycle wrapper so you can
//! start/stop/reload it by hand. Live control uses signals: SIGUSR1 pins it open
//! (also on SUPER+D), SIGUSR2 reloads the palette (sent by `tezca theme`).

use crate::term;
use std::process::Command;

const BIN: &str = "tezca-dock";

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(),
        Some("start") => cmd_start(),
        Some("stop") => cmd_stop(),
        Some("restart") => cmd_restart(),
        Some("toggle") => cmd_toggle(),
        Some(other) => Err(format!(
            "unknown dock subcommand: {other}\n  try: status · start · stop · restart · toggle"
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
