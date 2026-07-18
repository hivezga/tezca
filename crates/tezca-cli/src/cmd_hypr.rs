//! `tezca hypr` — live Hyprland option tuning that persists.
//!
//! The tezca-settings "Desktop" page drives this: `set` applies each option via
//! `hyprctl keyword` (instant, no reload) AND records it in the managed block of
//! conf.d/local.conf so it survives reloads and relogin. `get` reads the current
//! value; `reset` drops managed keyword overrides and reloads to restore the
//! shipped config. Monitor lines (owned by `tezca display`) are left alone.

use crate::{hypr, managed, term};

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        Some("get") => cmd_get(&args[1..]),
        Some("set") => cmd_set(&args[1..]),
        Some("reset") => cmd_reset(&args[1..]),
        Some("list") => cmd_list(),
        None | Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown hypr subcommand: {other}\n  try: get <opt> · set <opt> <val>… · reset [opt] · list"
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

/// `tezca hypr get <option>` → prints the current normalized scalar (for the GUI
/// to populate a control). Exits non-zero with nothing on stdout if unreadable.
fn cmd_get(args: &[&str]) -> Result<(), String> {
    let kw = args.first().ok_or("usage: tezca hypr get <option>")?;
    match hypr::getoption(kw) {
        Some(v) => {
            println!("{v}");
            Ok(())
        }
        None => Err(format!("could not read option '{kw}'")),
    }
}

/// `tezca hypr set <opt> <val> [<opt> <val>…]` — apply live + persist each pair.
fn cmd_set(args: &[&str]) -> Result<(), String> {
    if args.is_empty() || args.len() % 2 != 0 {
        return Err("usage: tezca hypr set <option> <value> [<option> <value>…]".into());
    }
    if !hypr::in_session() {
        return Err("not in a Hyprland session (nothing to apply)".into());
    }
    for pair in args.chunks(2) {
        let (kw, val) = (pair[0], pair[1]);
        hypr::keyword(kw, val).map_err(|e| format!("hyprctl keyword {kw}: {e}"))?;
        managed::set(kw, &format!("{kw} = {val}"))?;
    }
    Ok(())
}

/// `tezca hypr reset [<option>]` — drop one (or all non-monitor) managed keyword
/// overrides, then reload so the shipped config value takes over again.
fn cmd_reset(args: &[&str]) -> Result<(), String> {
    match args.first().copied() {
        Some(kw) => managed::remove(kw)?,
        // No arg: clear every managed keyword, but keep monitor:* (owned by
        // `tezca display`, reset separately).
        None => managed::remove_where(|k| !k.starts_with("monitor:"))?,
    }
    if hypr::in_session() {
        hypr::reload()?;
    }
    println!("  {} reset", term::green("✓"));
    Ok(())
}

/// `tezca hypr list` — the options currently persisted in the managed block.
fn cmd_list() -> Result<(), String> {
    let keys = managed::keys();
    let kw: Vec<_> = keys.iter().filter(|k| !k.starts_with("monitor:")).collect();
    println!("{}", term::header("tezca hypr — persisted overrides"));
    println!();
    if kw.is_empty() {
        println!("  {}", term::dim("(none — using the shipped config)"));
    }
    for k in kw {
        let live = hypr::getoption(k).unwrap_or_else(|| "?".into());
        println!("  {}  {}", term::cyan(k), term::dim(&format!("= {live}")));
    }
    println!();
    Ok(())
}

fn print_help() {
    println!("{}", term::header("tezca hypr"));
    println!("{}", term::dim("  live Hyprland option tuning that persists across reloads"));
    println!();
    println!("  {}   read an option's current value", term::cyan("get <option>"));
    println!("  {}  apply + persist one or more options", term::cyan("set <option> <value>…"));
    println!("  {}       drop overrides (all, or one) and reload", term::cyan("reset [option]"));
    println!("  {}             show persisted overrides", term::cyan("list"));
    println!();
    println!("{}", term::dim("  e.g. tezca hypr set decoration:rounding 14 general:gaps_out 16"));
}
