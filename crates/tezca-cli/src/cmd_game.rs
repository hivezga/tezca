//! `tezca game` — the gaming profile (DESIGN.md §9, §11).
//!
//! Two halves:
//!   * a **runtime compositor toggle** — `on` strips the eye-candy that costs
//!     latency (blur, shadows, animations) so game windows present as fast as
//!     possible; `off` restores it with a single `hyprctl reload` (which
//!     re-sources the committed config, theme colours included).
//!   * a **launch wrapper** — `run -- <cmd>` starts a title under
//!     `gamemoderun mangohud` (each applied only if installed) so you get the
//!     CPU governor / scheduler tweaks and the MangoHud overlay for free.
//!
//! The static, always-on side of the profile (tearing/immediate present, no-blur
//! window rules, auto-move to the games workspace) lives in
//! conf.d/gaming.conf — this command layers the *runtime* tweaks on top.
//!
//! Mode is persisted to `~/.config/tezca/game.state` (outside the theme-owned
//! `current/` dir) so the Waybar indicator and `tezca doctor` can read it.

use crate::{repo, term};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(),
        Some("on") => set_mode(true),
        Some("off") => set_mode(false),
        Some("toggle") => cmd_toggle(),
        Some("run") => cmd_run(&args[1..]),
        Some(other) => Err(format!(
            "unknown game subcommand: {other}\n  try: on · off · toggle · status · run -- <cmd>"
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

/// Runtime overrides applied by `on`. Restored wholesale by `off` via
/// `hyprctl reload`, so this list only needs the "turn off" direction.
const OVERRIDES: &[&str] = &[
    "keyword decoration:blur:enabled false",
    "keyword decoration:shadow:enabled false",
    "keyword animations:enabled false",
];

fn set_mode(on: bool) -> Result<(), String> {
    println!("{}", term::header("tezca game"));
    println!();

    if on {
        report_hypr(apply_overrides());
        write_state(true)?;
        notify("Game mode ON", "blur · shadows · animations off");
        println!("  {} game mode {}", term::green("→"), term::bold("ON"));
        println!(
            "    {}",
            term::dim("blur, shadows and animations disabled for lowest latency")
        );
    } else {
        report_hypr(restore());
        write_state(false)?;
        notify("Game mode OFF", "desktop eye-candy restored");
        println!("  {} game mode {}", term::green("✓"), term::bold("OFF"));
        println!("    {}", term::dim("compositor config reloaded — eye-candy restored"));
    }
    Ok(())
}

fn cmd_toggle() -> Result<(), String> {
    set_mode(!is_on())
}

/// `tezca game run [--] <cmd> [args...]` — launch under gamemode + MangoHud.
/// Enables game mode first so the session is already lean when the title maps.
fn cmd_run(rest: &[&str]) -> Result<(), String> {
    // Tolerate an optional `--` separator (`tezca game run -- steam`).
    let cmd: Vec<&str> = rest
        .iter()
        .copied()
        .skip_while(|a| *a == "--")
        .collect();
    if cmd.is_empty() {
        return Err(
            "usage: tezca game run -- <command> [args...]\n  e.g. tezca game run -- steam steam://rungameid/570"
                .into(),
        );
    }

    // Compose the wrapper: gamemoderun → mangohud → the command. Each layer is
    // applied only if present, so this degrades gracefully on a bare box.
    let mut argv: Vec<String> = Vec::new();
    let mut layers: Vec<&str> = Vec::new();
    if which("gamemoderun") {
        argv.push("gamemoderun".into());
        layers.push("gamemode");
    }
    if which("mangohud") {
        argv.push("mangohud".into());
        layers.push("mangohud");
    }
    argv.extend(cmd.iter().map(|s| s.to_string()));

    // Turn the compositor profile on for the session, then launch.
    let _ = set_mode(true);
    println!();

    let wrap = if layers.is_empty() {
        term::dim("no wrapper (gamemode/mangohud not installed)").to_string()
    } else {
        layers.join(" + ")
    };
    println!("  {} {}", term::dim("wrapper:"), wrap);
    println!("  {} {}", term::dim("launch: "), argv.join(" "));

    // Prefer `uwsm app --` so the game lands in its own systemd scope (clean
    // accounting + teardown), matching how autostart launches everything else.
    // `spawn()` detaches; the game outlives this CLI process.
    let launched = if which("uwsm") {
        let mut c = Command::new("uwsm");
        c.arg("app").arg("--");
        c.args(&argv);
        c.spawn()
    } else {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        c.spawn()
    };
    launched.map_err(|e| format!("failed to launch: {e}"))?;
    println!("  {} launched", term::green("→"));
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    println!("{}", term::header("tezca game"));
    println!();
    if is_on() {
        println!("  {} game mode is {}", term::green("●"), term::bold("ON"));
    } else {
        println!("  {} game mode is {}", term::dim("○"), term::bold("OFF"));
    }
    println!();
    println!("{}", term::bold("tooling"));
    tool_line("gamemode", which("gamemoderun"));
    tool_line("mangohud", which("mangohud"));
    tool_line("gamescope", which("gamescope"));
    println!();
    println!("  {}", term::dim("toggle: `tezca game toggle` (SUPER+G)"));
    println!("  {}", term::dim("launch: `tezca game run -- <cmd>`"));
    Ok(())
}

fn tool_line(name: &str, present: bool) {
    if present {
        println!("  {} {name}", term::green("✓"));
    } else {
        println!("  {} {name} {}", term::yellow("!"), term::dim("— not installed"));
    }
}

// --- compositor state ------------------------------------------------------

/// Apply the low-latency overrides in one batched hyprctl call. Skipped (Ok)
/// when not inside a live Hyprland session — the state file still flips so the
/// mode is correct on next reload.
fn apply_overrides() -> Result<(), String> {
    if !in_session() {
        return Ok(());
    }
    let batch = OVERRIDES.join(" ; ");
    hyprctl(&["--batch", &batch])
}

fn restore() -> Result<(), String> {
    if !in_session() {
        return Ok(());
    }
    hyprctl(&["reload"])
}

fn in_session() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

fn hyprctl(args: &[&str]) -> Result<(), String> {
    match Command::new("hyprctl").args(args).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

fn report_hypr(r: Result<(), String>) {
    match r {
        Ok(()) if in_session() => println!("  {} compositor updated", term::green("✓")),
        Ok(()) => println!(
            "  {} {}",
            term::dim("·"),
            term::dim("not in a Hyprland session — state saved, applies on next reload")
        ),
        Err(e) => println!("  {} hyprctl: {}", term::yellow("!"), term::dim(&e)),
    }
}

// --- persisted mode --------------------------------------------------------

/// `~/.config/tezca/game.state`. Present + "on" means game mode is active.
fn state_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("game.state"))
}

pub fn is_on() -> bool {
    state_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|s| s.trim() == "on")
        .unwrap_or(false)
}

fn write_state(on: bool) -> Result<(), String> {
    let path = state_path()?;
    if on {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        fs::write(&path, "on\n").map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    } else {
        // Absence == off. Ignore "already gone".
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", path.display())),
        }
    }
    Ok(())
}

// --- helpers ---------------------------------------------------------------

fn notify(summary: &str, body: &str) {
    // Best-effort desktop toast; never fail the command over it.
    let _ = Command::new("notify-send")
        .args(["-a", "Tezca", "-i", "input-gaming", summary, body])
        .status();
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
