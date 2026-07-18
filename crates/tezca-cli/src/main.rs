//! `tezca` — the Project:Tezca control CLI.
//!
//! Std-only, dependency-free (see DESIGN.md §8). Subcommands:
//!   link    symlink config/* into ~/.config (Phase 0/1)
//!   doctor  verify NVIDIA env, modeset, monitors, deps (Phase 1)
//!   theme   wallpaper-driven theming (Phase 3)
//!   dock    control the magnifying tezca-dock (Phase 5)
//!   game    gaming profile toggle + launch wrapper (Phase 6)
//!   install bootstrap guidance (delegates to install.sh)

mod cmd_display;
mod cmd_dock;
mod cmd_doctor;
mod cmd_game;
mod cmd_hypr;
mod cmd_keybind;
mod cmd_link;
mod cmd_settings;
mod cmd_theme;
mod cmd_wallpaper;
mod hypr;
mod managed;
mod repo;
mod term;

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest = args.iter().map(String::as_str);

    match rest.next() {
        None | Some("-h") | Some("--help") | Some("help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("tezca {VERSION}");
            ExitCode::SUCCESS
        }
        Some("link") => {
            let flags: Vec<&str> = rest.collect();
            let opts = cmd_link::Opts {
                dry_run: flags.iter().any(|f| *f == "--dry-run" || *f == "-n"),
                force: flags.iter().any(|f| *f == "--force" || *f == "-f"),
            };
            let dry = opts.dry_run;
            match cmd_link::run(opts) {
                Ok(()) => {
                    // Seed ~/.config/tezca/current/ so components never import a
                    // missing palette on first login. No-op if already themed.
                    if !dry {
                        if let Err(e) = cmd_theme::ensure_default(false) {
                            eprintln!("{} {e}", term::yellow("warning:"));
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{} {e}", term::red("error:"));
                    ExitCode::FAILURE
                }
            }
        }
        Some("doctor") => ExitCode::from(cmd_doctor::run() as u8),
        Some("theme") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_theme::run(&subargs) as u8)
        }
        Some("dock") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_dock::run(&subargs) as u8)
        }
        Some("display") | Some("monitor") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_display::run(&subargs) as u8)
        }
        Some("wallpaper") | Some("wall") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_wallpaper::run(&subargs) as u8)
        }
        Some("hypr") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_hypr::run(&subargs) as u8)
        }
        Some("keybind") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_keybind::run(&subargs) as u8)
        }
        Some("game") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_game::run(&subargs) as u8)
        }
        Some("settings") => {
            let subargs: Vec<&str> = rest.collect();
            ExitCode::from(cmd_settings::run(&subargs) as u8)
        }
        Some("install") => {
            println!("{}", term::header("tezca install"));
            println!();
            println!("  Bootstrap is handled by {} at the repo root:", term::bold("install.sh"));
            println!("    {}", term::cyan("./install.sh"));
            println!();
            println!("  It installs packages via paru, builds this binary, and runs");
            println!("  {} to symlink your config into place.", term::bold("tezca link"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("{} unknown command: {}", term::red("error:"), other);
            eprintln!("run {} for usage", term::bold("tezca --help"));
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("{}", term::header(&format!("tezca {VERSION}")));
    println!("{}", term::dim("  control surface for the Project:Tezca desktop"));
    println!();
    println!("{}", term::bold("USAGE"));
    println!("  tezca <command> [options]");
    println!();
    println!("{}", term::bold("COMMANDS"));
    let rows = [
        ("link", "symlink config/* into ~/.config (backs up existing)"),
        ("doctor", "verify NVIDIA env, modeset, monitors, and deps"),
        ("theme", "wallpaper-driven theming (list/set/wallpaper/reload)"),
        ("dock", "control the magnifying dock (start/stop/restart/config/set)"),
        ("display", "monitors: modes/scale + per-monitor brightness"),
        ("wallpaper", "per-monitor wallpaper overrides (set/clear/apply)"),
        ("hypr", "live+persisted Hyprland option tuning (get/set/reset)"),
        ("keybind", "list + rebind keybindings (rebind/restore)"),
        ("game", "gaming profile: on/off/toggle/status/run"),
        ("settings", "open the GTK control center (tezca-settings)"),
        ("install", "bootstrap guidance (see install.sh)"),
    ];
    for (c, d) in rows {
        println!("  {:<9} {}", term::cyan(c), d);
    }
    println!();
    println!("{}", term::bold("THEME"));
    println!("  {}            list curated themes + the active one", term::cyan("tezca theme list"));
    println!("  {}       apply a curated palette (e.g. obsidian)", term::cyan("tezca theme set <name>"));
    println!("  {}  extract a palette from any image (matugen)", term::cyan("tezca theme wallpaper <img>"));
    println!("  {}          re-apply the active theme + reload", term::cyan("tezca theme reload"));
    println!();
    println!("{}", term::bold("GAME"));
    println!("  {}          low-latency profile: blur/shadow/anim off", term::cyan("tezca game on"));
    println!("  {}         restore desktop eye-candy (hyprctl reload)", term::cyan("tezca game off"));
    println!("  {}      flip the profile (bound to SUPER+G)", term::cyan("tezca game toggle"));
    println!("  {}  launch under gamemode + MangoHud", term::cyan("tezca game run -- <cmd>"));
    println!();
    println!("{}", term::bold("LINK OPTIONS"));
    println!("  {}  preview actions without changing anything", term::cyan("-n, --dry-run"));
    println!("  {}    replace existing targets instead of backing up", term::cyan("-f, --force"));
    println!();
    println!("{}", term::bold("GLOBAL"));
    println!("  {}   print version", term::cyan("-V, --version"));
    println!("  {}      show this help", term::cyan("-h, --help"));
}
