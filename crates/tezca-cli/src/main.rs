//! `tezca` — the Project:Tezca control CLI.
//!
//! Std-only, dependency-free (see DESIGN.md §8). Subcommands:
//!   link    symlink config/* into ~/.config (Phase 0/1)
//!   doctor  verify NVIDIA env, modeset, monitors, deps (Phase 1)
//!   theme   wallpaper-driven theming (Phase 3 — stub)
//!   game    gaming profile toggle (Phase 6 — stub)
//!   install bootstrap guidance (delegates to install.sh)

mod cmd_doctor;
mod cmd_link;
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
            report(cmd_link::run(opts))
        }
        Some("doctor") => ExitCode::from(cmd_doctor::run() as u8),
        Some("theme") => stub("theme", "Phase 3 (theme engine)", &[
            "tezca theme list",
            "tezca theme set <name>",
            "tezca theme wallpaper <img>",
            "tezca theme reload",
        ]),
        Some("game") => stub("game", "Phase 6 (gaming profile)", &[
            "tezca game on",
            "tezca game off",
        ]),
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

fn report(r: Result<(), String>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {e}", term::red("error:"));
            ExitCode::FAILURE
        }
    }
}

fn stub(name: &str, phase: &str, usage: &[&str]) -> ExitCode {
    println!("{}", term::header(&format!("tezca {name}")));
    println!();
    println!("  {} — {}", term::yellow("not yet implemented"), term::dim(phase));
    println!();
    println!("  planned usage:");
    for u in usage {
        println!("    {}", term::dim(u));
    }
    ExitCode::SUCCESS
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
        ("theme", "wallpaper-driven theming        (Phase 3 — stub)"),
        ("game", "toggle the gaming profile        (Phase 6 — stub)"),
        ("install", "bootstrap guidance (see install.sh)"),
    ];
    for (c, d) in rows {
        println!("  {:<9} {}", term::cyan(c), d);
    }
    println!();
    println!("{}", term::bold("LINK OPTIONS"));
    println!("  {}  preview actions without changing anything", term::cyan("-n, --dry-run"));
    println!("  {}    replace existing targets instead of backing up", term::cyan("-f, --force"));
    println!();
    println!("{}", term::bold("GLOBAL"));
    println!("  {}   print version", term::cyan("-V, --version"));
    println!("  {}      show this help", term::cyan("-h, --help"));
}
