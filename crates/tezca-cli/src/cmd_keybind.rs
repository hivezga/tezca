//! `tezca keybind` — read + rebind Hyprland keybindings.
//!
//! The tezca-settings "Keybinds" page drives this. `list --machine` emits every
//! documented bind with its 1-based line number in keybinds.conf; `rebind`
//! rewrites one line's modifier+key in place and `set-action` rewrites what it
//! does (dispatcher + args, e.g. which app it launches), each with three safety
//! rails:
//!   1. an --expect guard: the CLI refuses to touch the line unless it still
//!      carries the combo the GUI showed (so a stale window can't clobber the
//!      wrong bind);
//!   2. conflict detection: refuses (exit 2) if the target combo is already used
//!      by another bind, unless --force;
//!   3. a backup: the previous keybinds.conf is copied to ~/.cache/tezca before
//!      every write, and `restore` puts it back.
//! After a successful write it runs `hyprctl reload` so the change is live.

use crate::{hypr, repo, term};
use std::fs;
use std::path::PathBuf;

fn conf_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?
        .join("hypr")
        .join("conf.d")
        .join("keybinds.conf"))
}

fn backup_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("neither $XDG_CACHE_HOME nor $HOME is set")?;
    Ok(base.join("tezca").join("backups").join("keybinds.conf.prev"))
}

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("list") => cmd_list(args.get(1..).unwrap_or(&[])),
        Some("rebind") => return cmd_rebind(&args[1..]),
        Some("set-action") => cmd_set_action(&args[1..]),
        Some("restore") => cmd_restore(),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown keybind subcommand: {other}\n  try: list · rebind --line N --mods … --key … · restore"
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

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// A parsed bind line (1-based `line`).
struct Bind {
    line: usize,
    flags: String,  // "bind", "binde", "bindm", …
    mods: String,   // normalized, $mod → SUPER
    key: String,
    desc: String,   // trailing `# comment`, or "" if undocumented
    action: String, // dispatcher + args, e.g. "exec, uwsm app -- brave"
}

/// Parse `bind[flags] = MODS, KEY, dispatcher…  # desc` — None if not a bind.
fn parse_bind(no: usize, raw: &str) -> Option<Bind> {
    let line = raw.trim();
    if !line.starts_with("bind") {
        return None;
    }
    let eq = line.find('=')?;
    let flags = line[..eq].trim().to_string();
    if !flags.chars().all(|c| c.is_ascii_lowercase()) {
        return None; // e.g. a "bindings" comment — not a real bind keyword
    }
    let body = &line[eq + 1..];
    let (before, desc) = match body.split_once('#') {
        Some((b, d)) => (b, d.trim().to_string()),
        None => (body, String::new()),
    };
    let mut it = before.splitn(3, ',');
    let mods = it.next().unwrap_or("").trim().replace("$mod", "SUPER");
    let key = it.next().unwrap_or("").trim().to_string();
    let action = it.next().unwrap_or("").trim().to_string();
    Some(Bind { line: no, flags, mods, key, desc, action })
}

/// "# ==== Title ====" / "# ---- Title ----" → the inner Title.
fn section_title(line: &str) -> Option<String> {
    let c = line.trim().strip_prefix('#')?.trim();
    let first = c.as_bytes().first().copied()?;
    if first != b'=' && first != b'-' {
        return None;
    }
    let inner = c.trim_matches(|ch| ch == '=' || ch == '-' || ch == ' ');
    (!inner.is_empty()).then(|| inner.to_string())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// `tezca keybind list [--machine]` — documented binds with line numbers.
/// Machine format:  `S\t<title>`  for a section,
/// `B\t<line>\t<mods>\t<key>\t<desc>\t<action>`  for a bind.
fn cmd_list(args: &[&str]) -> Result<(), String> {
    let text = fs::read_to_string(conf_path()?).map_err(|e| format!("cannot read keybinds.conf: {e}"))?;
    let machine = args.iter().any(|a| *a == "--machine" || *a == "-m");

    for (i, raw) in text.lines().enumerate() {
        if let Some(title) = section_title(raw) {
            if machine {
                println!("S\t{title}");
            } else {
                println!("\n{}", term::bold(&title));
            }
            continue;
        }
        if let Some(b) = parse_bind(i + 1, raw) {
            if b.desc.is_empty() {
                continue; // only surface documented binds
            }
            let combo = format_combo(&b.mods, &b.key);
            if machine {
                println!("B\t{}\t{}\t{}\t{}\t{}", b.line, b.mods, b.key, b.desc, b.action);
            } else {
                println!("  {:<24} {}", combo, term::dim(&b.desc));
            }
        }
    }
    Ok(())
}

fn format_combo(mods: &str, key: &str) -> String {
    let mut parts: Vec<&str> = mods.split_whitespace().collect();
    if !key.is_empty() {
        parts.push(key);
    }
    parts.join(" + ")
}

// ---------------------------------------------------------------------------
// rebind
// ---------------------------------------------------------------------------

/// Returns a process exit code directly (2 = conflict, for the GUI to detect).
fn cmd_rebind(args: &[&str]) -> i32 {
    match rebind(args) {
        Ok(()) => 0,
        Err(RebindErr::Conflict(msg)) => {
            eprintln!("conflict: {msg}");
            2
        }
        Err(RebindErr::Other(e)) => {
            eprintln!("{} {e}", term::red("error:"));
            1
        }
    }
}

enum RebindErr {
    Conflict(String),
    Other(String),
}
impl From<String> for RebindErr {
    fn from(s: String) -> Self {
        RebindErr::Other(s)
    }
}

fn rebind(args: &[&str]) -> Result<(), RebindErr> {
    let mut line: Option<usize> = None;
    let mut mods = String::new();
    let mut key = String::new();
    let mut expect_mods: Option<String> = None;
    let mut expect_key: Option<String> = None;
    let mut force = false;

    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--line" => line = it.next().and_then(|v| v.parse().ok()),
            "--mods" => mods = it.next().unwrap_or("").to_string(),
            "--key" => key = it.next().unwrap_or("").to_string(),
            "--expect-mods" => expect_mods = it.next().map(str::to_string),
            "--expect-key" => expect_key = it.next().map(str::to_string),
            "--force" => force = true,
            other => return Err(format!("unknown flag: {other}").into()),
        }
    }
    let line = line.ok_or_else(|| "rebind needs --line N".to_string())?;
    if key.trim().is_empty() {
        return Err("rebind needs --key".to_string().into());
    }

    let path = conf_path()?;
    let text = fs::read_to_string(&path).map_err(|e| format!("cannot read keybinds.conf: {e}"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    let idx = line
        .checked_sub(1)
        .filter(|&i| i < lines.len())
        .ok_or_else(|| format!("line {line} is out of range"))?;

    let target = parse_bind(line, &lines[idx])
        .ok_or_else(|| format!("line {line} is not a bind line"))?;

    // Guard: the line must still carry the combo the GUI showed us.
    if let (Some(em), Some(ek)) = (&expect_mods, &expect_key) {
        if !same_combo(&target.mods, &target.key, em, ek) {
            return Err(
                "keybinds.conf changed since it was read — reopen Settings and try again"
                    .to_string()
                    .into(),
            );
        }
    }

    // Conflict check against every other regular bind (skip mouse/hold binds).
    if !force {
        for (i, raw) in text.lines().enumerate() {
            if i == idx {
                continue;
            }
            if let Some(b) = parse_bind(i + 1, raw) {
                if b.flags == "bindm" {
                    continue;
                }
                if same_combo(&b.mods, &b.key, &mods, &key) {
                    let what = if b.desc.is_empty() {
                        format!("line {}", b.line)
                    } else {
                        b.desc.clone()
                    };
                    return Err(RebindErr::Conflict(format!(
                        "{} is already bound to {what}",
                        format_combo(&normalize_mods(&mods), &normalize_key(&key))
                    )));
                }
            }
        }
    }

    // Rewrite the line, preserving flags / dispatcher / args / comment.
    lines[idx] = rewrite_line(&lines[idx], &mods, &key)
        .ok_or_else(|| format!("could not rewrite line {line}"))?;

    // Backup, then write.
    write_backup(&text)?;
    let mut body = lines.join("\n");
    body.push('\n');
    fs::write(&path, body).map_err(|e| format!("cannot write keybinds.conf: {e}"))?;

    if hypr::in_session() {
        let _ = hypr::reload();
    }
    println!(
        "  {} {} → {}",
        term::green("✓"),
        term::dim(&format!("line {line}")),
        format_combo(&normalize_mods(&mods), &normalize_key(&key))
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// set-action  (change what a bind does — dispatcher + args)
// ---------------------------------------------------------------------------

/// `tezca keybind set-action --line N --action "exec, uwsm app -- firefox.desktop"
/// [--desc "Firefox"] [--expect-mods … --expect-key …]` — rewrite one line's
/// dispatcher+args (and optionally its `# comment` label), keeping its combo.
fn cmd_set_action(args: &[&str]) -> Result<(), String> {
    let mut line: Option<usize> = None;
    let mut action: Option<String> = None;
    let mut desc: Option<String> = None;
    let mut expect_mods: Option<String> = None;
    let mut expect_key: Option<String> = None;

    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--line" => line = it.next().and_then(|v| v.parse().ok()),
            "--action" => action = it.next().map(str::to_string),
            "--desc" => desc = it.next().map(str::to_string),
            "--expect-mods" => expect_mods = it.next().map(str::to_string),
            "--expect-key" => expect_key = it.next().map(str::to_string),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let line = line.ok_or("set-action needs --line N")?;
    let action = action.ok_or("set-action needs --action")?;
    let action = action.trim();
    if action.is_empty() {
        return Err("set-action needs a non-empty --action".into());
    }

    let path = conf_path()?;
    let text = fs::read_to_string(&path).map_err(|e| format!("cannot read keybinds.conf: {e}"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let idx = line
        .checked_sub(1)
        .filter(|&i| i < lines.len())
        .ok_or_else(|| format!("line {line} is out of range"))?;
    let target =
        parse_bind(line, &lines[idx]).ok_or_else(|| format!("line {line} is not a bind line"))?;

    // Guard: the line must still carry the combo the GUI showed us.
    if let (Some(em), Some(ek)) = (&expect_mods, &expect_key) {
        if !same_combo(&target.mods, &target.key, em, ek) {
            return Err(
                "keybinds.conf changed since it was read — reopen Settings and try again".into(),
            );
        }
    }

    lines[idx] = rewrite_action_line(&lines[idx], action, desc.as_deref())
        .ok_or_else(|| format!("could not rewrite line {line}"))?;

    write_backup(&text)?;
    let mut body = lines.join("\n");
    body.push('\n');
    fs::write(&path, body).map_err(|e| format!("cannot write keybinds.conf: {e}"))?;

    if hypr::in_session() {
        let _ = hypr::reload();
    }
    println!("  {} {} → {}", term::green("✓"), term::dim(&format!("line {line}")), action);
    Ok(())
}

/// Replace the dispatcher+args of a bind line, keeping flags/mods/key; when
/// `desc` is given (non-empty) it becomes the new `# comment` label, else the
/// existing comment is preserved.
fn rewrite_action_line(raw: &str, action: &str, desc: Option<&str>) -> Option<String> {
    let indent: String = raw.chars().take_while(|c| c.is_whitespace()).collect();
    let line = raw.trim();
    let eq = line.find('=')?;
    let head = line[..eq].trim_end(); // "bind" / "binde" / …
    let body = &line[eq + 1..];
    let (before, existing_desc) = match body.split_once('#') {
        Some((b, d)) => (b, Some(d.trim().to_string())),
        None => (body, None),
    };
    let mut it = before.splitn(3, ',');
    let mods = it.next()?.trim();
    let key = it.next()?.trim();
    let desc_final = desc.filter(|s| !s.is_empty()).map(String::from).or(existing_desc);
    let comment = desc_final.map(|d| format!("  # {d}")).unwrap_or_default();
    Some(format!("{indent}{head} = {mods}, {key}, {action}{comment}"))
}

fn cmd_restore() -> Result<(), String> {
    let bak = backup_path()?;
    let text = fs::read_to_string(&bak)
        .map_err(|_| "no backup to restore (rebind something first)".to_string())?;
    fs::write(conf_path()?, text).map_err(|e| format!("cannot restore: {e}"))?;
    if hypr::in_session() {
        let _ = hypr::reload();
    }
    println!("  {} restored keybinds.conf from backup", term::green("✓"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Combo helpers
// ---------------------------------------------------------------------------

/// Canonical modifier set: $mod→SUPER, uppercased, sorted, de-duplicated.
fn normalize_mods(mods: &str) -> String {
    let mut parts: Vec<String> = mods
        .replace("$mod", "SUPER")
        .split_whitespace()
        .map(|s| s.to_uppercase())
        .collect();
    parts.sort();
    parts.dedup();
    parts.join(" ")
}

/// Tidy a key for writing: single letter → uppercase, arrows → lowercase.
fn normalize_key(key: &str) -> String {
    let k = key.trim();
    if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) {
        return k.to_uppercase();
    }
    match k.to_lowercase().as_str() {
        "left" | "right" | "up" | "down" => k.to_lowercase(),
        _ => k.to_string(),
    }
}

/// Do two combos mean the same bind? (order-independent mods, case-fold key).
fn same_combo(mods_a: &str, key_a: &str, mods_b: &str, key_b: &str) -> bool {
    normalize_mods(mods_a) == normalize_mods(mods_b)
        && key_a.trim().eq_ignore_ascii_case(key_b.trim())
}

/// Replace the mods + key of a bind line, keeping flags/dispatcher/args/comment.
fn rewrite_line(raw: &str, mods: &str, key: &str) -> Option<String> {
    // Preserve leading indentation.
    let indent: String = raw.chars().take_while(|c| c.is_whitespace()).collect();
    let line = raw.trim();
    let eq = line.find('=')?;
    let head = line[..eq].trim_end(); // "bind" / "binde" / …
    let body = &line[eq + 1..];

    let mut it = body.splitn(3, ',');
    let _old_mods = it.next()?; // discarded
    let _old_key = it.next()?; // discarded
    let rest = it.next().unwrap_or("").trim_start();

    // Write $mod for SUPER to match the file's house style.
    let mods_out = normalize_mods(mods).replace("SUPER", "$mod");
    let key_out = normalize_key(key);
    Some(format!("{indent}{head} = {mods_out}, {key_out}, {rest}"))
}

fn write_backup(text: &str) -> Result<(), String> {
    let bak = backup_path()?;
    if let Some(dir) = bak.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create backup dir: {e}"))?;
    }
    fs::write(&bak, text).map_err(|e| format!("cannot write backup: {e}"))
}

fn print_help() {
    println!("{}", term::header("tezca keybind"));
    println!();
    println!("  {}                    list documented binds", term::cyan("list [--machine]"));
    println!(
        "  {}  rebind a line's combo",
        term::cyan("rebind --line N --mods \"SUPER SHIFT\" --key W")
    );
    println!(
        "  {}  change what a bind does",
        term::cyan("set-action --line N --action \"exec, uwsm app -- app.desktop\"")
    );
    println!("  {}                         undo the last change", term::cyan("restore"));
}
