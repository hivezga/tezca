//! `tezca keybind` — read + rebind Hyprland keybindings.
//!
//! The tezca-settings "Keybinds" page drives this. `list --machine` emits every
//! documented bind with its 1-based line number in the shipped `keybinds.lua`;
//! `rebind` changes a bind's modifier+key and `set-action` changes what it does
//! (dispatcher + args, e.g. which app it launches).
//!
//! ## An override layer, not an in-place rewrite
//!
//! These used to rewrite the matching line of `conf.d/keybinds.lua`. But
//! `~/.config/hypr` is a symlink into the repo, so every rebind edited a
//! **git-tracked file**: the working tree stayed dirty, a downstream clone could
//! not `git pull` without conflicting on the one file whose contents are pure
//! muscle memory, and the shipped map drifted invisibly from upstream.
//!
//! So the shipped map is now read-only, and changes go to a generated override
//! layer at `~/.config/tezca/keybinds.lua`, which `hyprland.lua` loads *after*
//! it. Each override releases the shipped combo with `hl.unbind`, then binds the
//! replacement. That means the base map can't be corrupted, upstream changes to it
//! keep flowing, every local change is visible in one small file, and `reset` is
//! "delete the overrides" — an operation that cannot fail halfway.
//!
//! Every `hl.unbind` is emitted *before* every `hl.bind`. Interleaved, one
//! entry's unbind would cancel another entry's replacement whenever a combo is
//! moved from one bind to another. `hl.unbind` also matches the registered combo
//! string exactly and case-sensitively, so overrides always spell it out in full
//! (`"SUPER + Q"`) rather than reusing the shipped map's `mod` variable.
//!
//! Three safety rails carry over, plus one new one:
//!   1. an `--expect` guard: the CLI refuses to touch a bind unless it still
//!      carries the combo the GUI showed (compared against the *effective* combo,
//!      overrides included);
//!   2. conflict detection: refuses (exit 2) if the target combo is already used
//!      by another bind, unless `--force`;
//!   3. a snapshot of the override layer before every write, so `restore` undoes
//!      the last change;
//!   4. input validation (`crate::validate`) — a combo containing a comma or a
//!      newline used to be written straight through, which could inject a whole
//!      extra directive. See that module for why `--expect` cannot catch it.
//!
//! Each override records the base combo it replaced (`was=`). If an upstream
//! change makes line N a *different* bind, the override is reported as stale and
//! skipped, rather than silently applied to the wrong key.
//!
//! One bind in the shipped map — ALT+Tab — is a multi-line `function() … end`
//! body rather than a single dispatcher. It is listed (its combo is on one line)
//! but cannot be rebound, because this line-oriented reader cannot reproduce the
//! body into the override layer.
//!
//! ## What the Settings page needs on top of that
//!
//! Three commands exist for the GUI rather than for the command line:
//!
//!   * `list --machine` reports, per bind, whether it is *editable* at all (the
//!     two shapes above are not) and — for an `exec_cmd` bind — the command
//!     inside the Lua literal, so the page can offer "which app does this key
//!     launch" as a plain text field instead of a Lua expression;
//!   * `set-action --exec <command>` is that field's write path: the quoting into
//!     a Lua literal happens here, next to [`lua_string`], and not in JavaScript;
//!   * `capture on` / `off` suspends every global keybinding for as long as the
//!     GUI's "press the new shortcut" box is open. Without it that box is unable
//!     to read the very combos worth rebinding: Hyprland takes a bound combo
//!     before the focused window sees it, so pressing SUPER+B would launch the
//!     browser rather than register as the keys pressed. See [`cmd_capture`].
//!
//! `reset --line N` is the fourth, and is equally the page's: it drops one
//! bind's override, which is how a single key goes back to the shipped default
//! without `reset` taking every other customisation with it.
//!
//! ## Upgrading from the hyprlang override layer
//!
//! Before the Lua cutover this layer was `~/.config/tezca/keybinds.conf`, holding
//! hyprlang `unbind`/`bind` pairs. [`migrate_legacy`] carries one into
//! `keybinds.lua` on first run. It cannot be a copy: entries are keyed by a line
//! number in a shipped map that was itself rewritten (the two disagree — `CTRL, Q`
//! is line 16 of `keybinds.conf` and line 23 of `keybinds.lua`), and dispatchers
//! changed shape (`exec, foo` → `hl.dsp.exec_cmd("foo")`). So entries are placed
//! by the combo they recorded in `was=`, and anything that cannot be re-expressed
//! faithfully is reported rather than guessed at.

use crate::{atomic, hypr, repo, term, validate};
use std::fs;
use std::path::{Path, PathBuf};

/// Header for the generated override file. The whole file is generated, so this
/// points hand edits at `conf.d/local.lua` instead.
const OVERRIDE_HEADER: &str = "\
-- ~/.config/tezca/keybinds.lua — generated by `tezca keybind`. Do not hand-edit.
--
-- Loaded by hyprland.lua AFTER conf.d/keybinds.lua, so these win. Each entry
-- releases the shipped combo with `hl.unbind` and binds the replacement; the
-- `was=` note records which shipped bind it replaced, so a later upstream change
-- to keybinds.lua is reported as stale instead of silently rebinding the wrong
-- key.
--
-- Combos are written as literal strings even where the shipped map builds them
-- from the `mod` variable: `hl.unbind` matches on the exact registered string
-- (and is case-sensitive), and `mod .. \" + Q\"` registers precisely \"SUPER + Q\".
--
-- `tezca keybind restore` undoes the last change; `tezca keybind reset` drops all
-- of them and returns to the shipped map. For extra binds of your own that Tezca
-- should never touch, use conf.d/local.lua.
";

/// The shipped bind map. Read-only — nothing in this module writes to it.
fn base_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("hypr").join("conf.d").join("keybinds.lua"))
}

/// The generated override layer.
fn override_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("keybinds.lua"))
}

/// Snapshot of the override layer taken before every write, for `restore`.
fn backup_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("neither $XDG_CACHE_HOME nor $HOME is set")?;
    Ok(base.join("tezca").join("backups").join("keybind-overrides.prev"))
}

/// Create the override file if absent, so Hyprland's `source` never errors on a
/// fresh install. Returns whether it created anything. Called by `tezca link`.
pub fn seed() -> Result<bool, String> {
    let p = override_path()?;
    let created = if p.exists() {
        false
    } else {
        atomic::write(&p, OVERRIDE_HEADER)?;
        true
    };
    // `tezca link` is the upgrade path, so this is where a pre-Lua override file
    // gets carried over. Runs after the seed, since the migration keys off
    // whether the Lua layer holds any *entries*, not whether the file exists.
    migrate_legacy()?;
    Ok(created)
}

pub fn run(args: &[&str]) -> i32 {
    // Before anything reads the override layer. Reported but not fatal: a
    // malformed pre-Lua file should not cost you `list`, which needs only the
    // shipped map.
    if let Err(e) = migrate_legacy() {
        eprintln!("  {} could not migrate the pre-Lua keybind overrides: {e}", term::yellow("!"));
    }
    let r = match args.first().copied() {
        None | Some("list") => cmd_list(args.get(1..).unwrap_or(&[])),
        Some("rebind") => return cmd_rebind(&args[1..]),
        Some("set-action") => cmd_set_action(&args[1..]),
        Some("restore") => cmd_restore(),
        Some("reset") => cmd_reset(&args[1..]),
        Some("capture") => cmd_capture(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown keybind subcommand: {other}\n  try: list · rebind --line N … · \
             set-action --line N … · restore · reset [--line N] · capture on|off"
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
// Parsing the shipped map
// ---------------------------------------------------------------------------

/// A parsed bind (1-based `line` in the shipped map).
#[derive(Clone, Debug, PartialEq)]
struct Bind {
    line: usize,
    /// The Lua options table verbatim (`{ locked = true, repeating = true }`),
    /// or "" when the bind takes none. This replaces hyprlang's `bind`/`binde`/
    /// `bindl`/`bindel`/`bindm` keyword suffixes, which all became flags here.
    opts: String,
    mods: String, // normalized, always spelled SUPER (never the `mod` variable)
    key: String,
    desc: String, // trailing `-- comment`, or "" if undocumented
    /// The dispatcher expression, e.g. `hl.dsp.exec_cmd("uwsm app -- brave")`.
    /// Empty marks a bind whose call spans several lines (a `function() … end`
    /// body), which this line-oriented reader cannot capture or reproduce.
    action: String,
    /// True when an override-layer entry replaced this bind.
    overridden: bool,
}

/// Split a Lua argument list on top-level commas, ignoring those nested in
/// parentheses, braces or string literals. Returns the arguments plus the byte
/// offset just past the `)` that closed the call, so the caller can find the
/// trailing comment without rescanning.
///
/// Returns None if the list does not close on this line — a multi-line
/// `hl.bind(…, function() …` call.
///
/// Scanning is required rather than a `find("--")`: `hl.dsp.exec_cmd("uwsm app
/// -- dolphin")` puts a comment marker inside a string on most of these lines,
/// and splitting there would truncate the dispatcher and lose the description.
fn split_lua_args(s: &str) -> Option<(Vec<String>, usize)> {
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    let (mut args, mut cur) = (Vec::new(), String::new());
    for (i, c) in s.char_indices() {
        if in_str {
            cur.push(c);
            match c {
                _ if esc => esc = false,
                '\\' => esc = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '(' | '{' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | '}' | ']' if depth > 0 => {
                depth -= 1;
                cur.push(c);
            }
            // The unmatched ')' that closes `hl.bind(` itself.
            ')' => {
                args.push(cur.trim().to_string());
                return Some((args, i + c.len_utf8()));
            }
            ',' if depth == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    None
}

/// The `-- comment` at the end of a line, ignoring `--` inside string literals
/// (`hl.dsp.exec_cmd("uwsm app -- dolphin")`). Empty when there is none.
fn trailing_comment(s: &str) -> String {
    let (mut in_str, mut esc) = (false, false);
    let b: Vec<char> = s.chars().collect();
    for i in 0..b.len() {
        if in_str {
            match b[i] {
                _ if esc => esc = false,
                '\\' => esc = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        if b[i] == '"' {
            in_str = true;
        } else if b[i] == '-' && b.get(i + 1) == Some(&'-') {
            return b[i + 2..].iter().collect::<String>().trim().to_string();
        }
    }
    String::new()
}

/// Recover "SUPER SHIFT" + "left" from a combo argument, which is either a
/// literal `"CTRL + Q"` or the shipped map's `mod .. " + SHIFT + left"`.
fn parse_combo_arg(arg: &str) -> Option<(String, String)> {
    let a = arg.trim();
    let text = match a.strip_prefix("mod") {
        // `mod .. " + X"` registers the string "SUPER + X", so that is what the
        // override layer has to unbind — expand it rather than keep the variable.
        Some(rest) => {
            let lit = rest.trim().strip_prefix("..")?.trim();
            let inner = lit.strip_prefix('"')?.strip_suffix('"')?;
            format!("SUPER{inner}")
        }
        None => a.strip_prefix('"')?.strip_suffix('"')?.to_string(),
    };
    let mut parts: Vec<&str> = text.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let key = parts.pop()?.to_string();
    Some((parts.join(" "), key))
}

/// Parse `hl.bind(<combo>, <dispatcher>[, <opts>])  -- desc` — None if not a bind.
fn parse_bind(no: usize, raw: &str) -> Option<Bind> {
    let line = raw.trim();
    let rest = line.strip_prefix("hl.bind(")?;

    match split_lua_args(rest) {
        Some((args, end)) => {
            let (mods, key) = parse_combo_arg(args.first()?)?;
            let desc = rest[end..]
                .trim_start()
                .strip_prefix("--")
                .map(|d| d.trim().to_string())
                .unwrap_or_default();
            Some(Bind {
                line: no,
                opts: args.get(2).cloned().unwrap_or_default(),
                mods,
                key,
                desc,
                action: args.get(1).cloned().unwrap_or_default(),
                overridden: false,
            })
        }
        // Multi-line call: the combo is still on this line and is what `list`
        // and conflict detection need, but the body is not reproducible. The
        // description still has to be picked up, or the bind vanishes from
        // `list` entirely (which filters undocumented binds) instead of showing
        // up as present-but-not-rebindable.
        None => {
            let (mods, key) = parse_combo_arg(rest.split_once(',')?.0)?;
            Some(Bind {
                line: no,
                opts: String::new(),
                mods,
                key,
                desc: trailing_comment(rest),
                action: String::new(),
                overridden: false,
            })
        }
    }
}

/// "-- ==== Title ====" / "-- ---- Title ----" → the inner Title.
fn section_title(line: &str) -> Option<String> {
    let c = line.trim().strip_prefix("--")?.trim();
    let first = c.as_bytes().first().copied()?;
    if first != b'=' && first != b'-' {
        return None;
    }
    let inner = c.trim_matches(|ch| ch == '=' || ch == '-' || ch == ' ');
    (!inner.is_empty()).then(|| inner.to_string())
}

// ---------------------------------------------------------------------------
// The override layer
// ---------------------------------------------------------------------------

/// One entry of the override layer: what line N of the shipped map became.
#[derive(Clone, Debug, PartialEq)]
struct Ovr {
    line: usize,
    /// The shipped combo this entry replaced, recorded so a later upstream change
    /// to `keybinds.lua` is detected instead of silently misapplied.
    was_mods: String,
    was_key: String,
    opts: String,
    mods: String,
    key: String,
    action: String,
    desc: String,
}

/// `# @42 was=$mod|W` — the entry header. `|` separates the recorded combo, so a
/// comma in neither field can be ambiguous, and an empty modifier list is legal
/// (`was=|XF86AudioMute`) because plenty of media binds have no modifier.
fn parse_ovr_header(line: &str) -> Option<(usize, String, String)> {
    let rest = line.trim().strip_prefix("-- @")?;
    let (num, was) = rest.split_once(" was=")?;
    let n: usize = num.trim().parse().ok()?;
    let (mods, key) = was.split_once('|')?;
    Some((n, mods.trim().to_string(), key.trim().to_string()))
}

/// Parse the override layer out of already-read text.
fn parse_overrides(text: &str) -> Vec<Ovr> {
    let mut out: Vec<Ovr> = Vec::new();
    let mut pending: Option<(usize, String, String)> = None;
    for raw in text.lines() {
        if let Some(h) = parse_ovr_header(raw) {
            pending = Some(h);
            continue;
        }
        let Some((line, was_mods, was_key)) = pending.clone() else { continue };
        // `parse_bind`'s line argument is only a label here; the header is the key.
        if let Some(b) = parse_bind(line, raw) {
            out.push(Ovr {
                line,
                was_mods,
                was_key,
                opts: b.opts,
                mods: b.mods,
                key: b.key,
                action: b.action,
                desc: b.desc,
            });
            pending = None;
        }
    }
    out
}

/// Read the override layer. A missing file simply means "no overrides".
fn load_overrides() -> Result<Vec<Ovr>, String> {
    let p = override_path()?;
    Ok(fs::read_to_string(&p).map(|t| parse_overrides(&t)).unwrap_or_default())
}

/// Serialize the override layer.
///
/// Every `unbind` is emitted before every `bind`: interleaved, moving a combo from
/// one bind to another would have the donor's `unbind` cancel the recipient's new
/// `bind` whenever the donor sorted later.
fn render_overrides(ovrs: &[Ovr]) -> String {
    let mut s = String::from(OVERRIDE_HEADER);
    if ovrs.is_empty() {
        return s;
    }
    let mut sorted = ovrs.to_vec();
    sorted.sort_by_key(|o| o.line);

    s.push_str("\n-- --- release the shipped combos being replaced ---\n");
    for o in &sorted {
        s.push_str(&format!(
            "hl.unbind(\"{}\")\n",
            format_combo(&display_mods(&o.was_mods), &o.was_key)
        ));
    }
    s.push_str("\n-- --- replacements ---\n");
    for o in &sorted {
        let comment = if o.desc.is_empty() { String::new() } else { format!("  -- {}", o.desc) };
        let opts = if o.opts.is_empty() { String::new() } else { format!(", {}", o.opts) };
        s.push_str(&format!("-- @{} was={}|{}\n", o.line, o.was_mods, o.was_key));
        s.push_str(&format!(
            "hl.bind(\"{}\", {}{}){}\n",
            format_combo(&display_mods(&o.mods), &o.key),
            o.action,
            opts,
            comment
        ));
    }
    s
}

/// Write the override layer, snapshotting the previous contents for `restore`.
fn save_overrides(ovrs: &[Ovr]) -> Result<(), String> {
    let p = override_path()?;
    let previous = fs::read_to_string(&p).unwrap_or_else(|_| OVERRIDE_HEADER.to_string());
    atomic::write(&backup_path()?, &previous)?;
    atomic::write(&p, &render_overrides(ovrs))
}

/// Apply the override layer to the shipped map.
///
/// An entry whose recorded `was=` combo no longer matches line N is **stale** —
/// the shipped map changed underneath it — so it is skipped and reported rather
/// than applied to whatever bind now occupies that line.
fn apply_overrides(base: &str, ovrs: &[Ovr]) -> (Vec<Bind>, Vec<Ovr>) {
    let mut binds = Vec::new();
    let mut stale = Vec::new();
    for (i, raw) in base.lines().enumerate() {
        let Some(mut b) = parse_bind(i + 1, raw) else { continue };
        if let Some(o) = ovrs.iter().find(|o| o.line == b.line) {
            if same_combo(&b.mods, &b.key, &o.was_mods, &o.was_key) {
                b.mods = o.mods.clone();
                b.key = o.key.clone();
                b.action = o.action.clone();
                b.desc = if o.desc.is_empty() { b.desc } else { o.desc.clone() };
                b.overridden = true;
            } else {
                stale.push(o.clone());
            }
        }
        binds.push(b);
    }
    (binds, stale)
}

/// The effective bind map: shipped, with overrides applied.
fn effective() -> Result<(Vec<Bind>, Vec<Ovr>), String> {
    let base =
        fs::read_to_string(base_path()?).map_err(|e| format!("cannot read keybinds.lua: {e}"))?;
    let ovrs = load_overrides()?;
    Ok(apply_overrides(&base, &ovrs))
}

fn warn_stale(stale: &[Ovr]) {
    for o in stale {
        eprintln!(
            "  {} the override for line {} no longer matches the shipped map \
             (it recorded {}) — ignoring it; re-apply it from Settings",
            term::yellow("!"),
            o.line,
            format_combo(&o.was_mods, &o.was_key)
        );
    }
}

// ---------------------------------------------------------------------------
// Migrating the pre-Lua override layer
// ---------------------------------------------------------------------------

/// Appended to the pre-Lua override file once its entries have been carried over,
/// so a later `keybind reset` cannot resurrect them.
///
/// A marker rather than emptying the file, which is what `managed.rs` does to the
/// old option store. The hyprlang tree is still shipped as a rollback (rename
/// `hyprland.lua` and relog — see its header), and `hyprland.conf` still sources
/// this file, so blanking it would mean rolling back silently lost your rebinds.
/// It is a comment, so neither hyprlang nor [`parse_legacy_overrides`] sees it.
const MIGRATED_MARKER: &str = "# migrated to ~/.config/tezca/keybinds.lua by `tezca keybind`";

/// The pre-Lua override layer. Read once by [`migrate_legacy`]; never rewritten
/// beyond appending [`MIGRATED_MARKER`].
fn legacy_override_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca").join("keybinds.conf"))
}

/// The pre-Lua shipped map, still in the repo as the hyprlang rollback.
fn legacy_base_path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("hypr").join("conf.d").join("keybinds.conf"))
}

/// One entry of the pre-Lua override layer.
#[derive(Clone, Debug, PartialEq)]
struct LegacyOvr {
    /// Line in the *pre-Lua* shipped map. Used only to look that map up when
    /// deciding whether the action had been customised — never carried over as
    /// the new entry's line, because the two maps disagree about numbering
    /// (`CTRL, Q` is line 16 of keybinds.conf and line 23 of keybinds.lua).
    line: usize,
    was_mods: String,
    was_key: String,
    mods: String,
    key: String,
    /// Hyprlang dispatcher plus arguments, e.g. `exec, uwsm app -- brave`.
    action: String,
    desc: String,
}

/// `# @42 was=$mod|W` — the pre-Lua entry header (hyprlang comments start `#`).
fn parse_legacy_ovr_header(line: &str) -> Option<(usize, String, String)> {
    let rest = line.trim().strip_prefix("# @")?;
    let (num, was) = rest.split_once(" was=")?;
    let n: usize = num.trim().parse().ok()?;
    let (mods, key) = was.split_once('|')?;
    Some((n, mods.trim().replace("$mod", "SUPER"), key.trim().to_string()))
}

/// Parse a hyprlang `bind[flags] = MODS, KEY, dispatcher…  # desc` line into
/// (mods, key, action, desc). None if the line is not a bind.
///
/// Splitting the description off at the first `#` is what the pre-Lua writer and
/// reader both did, so it round-trips the files that actually exist — at the cost
/// of truncating an action containing a literal `#`. That flaw is inherited on
/// purpose: reproducing the old reader is what makes this migration faithful.
fn parse_legacy_bind(raw: &str) -> Option<(String, String, String, String)> {
    let line = raw.trim();
    if !line.starts_with("bind") {
        return None;
    }
    let eq = line.find('=')?;
    let flags = line[..eq].trim();
    if !flags.chars().all(|c| c.is_ascii_lowercase()) {
        return None; // e.g. a "bindings" prose comment, not a real bind keyword
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
    Some((mods, key, action, desc))
}

/// Parse the pre-Lua override layer out of already-read text.
fn parse_legacy_overrides(text: &str) -> Vec<LegacyOvr> {
    let mut out = Vec::new();
    let mut pending: Option<(usize, String, String)> = None;
    for raw in text.lines() {
        if let Some(h) = parse_legacy_ovr_header(raw) {
            pending = Some(h);
            continue;
        }
        let Some((line, was_mods, was_key)) = pending.clone() else { continue };
        if let Some((mods, key, action, desc)) = parse_legacy_bind(raw) {
            out.push(LegacyOvr { line, was_mods, was_key, mods, key, action, desc });
            pending = None;
        }
    }
    out
}

/// Did this entry change what the bind *does*, rather than only which keys run
/// it? Answerable only while the pre-Lua shipped map is still on disk; when it is
/// gone the answer is "assume not", which is what `rebind` — the command that
/// wrote nearly all of these — would have produced anyway.
fn legacy_action_was_customised(e: &LegacyOvr, legacy_shipped: &str) -> bool {
    let Some(line) = legacy_shipped.lines().nth(e.line.saturating_sub(1)) else { return false };
    match parse_legacy_bind(line) {
        // Only trust the comparison if that line still holds the bind the entry
        // recorded; otherwise the old map moved under it too and proves nothing.
        Some((mods, key, action, _)) => {
            same_combo(&mods, &key, &e.was_mods, &e.was_key) && action != e.action
        }
        None => false,
    }
}

/// Translate one pre-Lua entry against the current shipped map.
///
/// `Ok(None)` means the entry would only restate what the shipped map already
/// says, so it is dropped rather than written back out.
fn migrate_entry(
    e: &LegacyOvr,
    shipped: &[Bind],
    legacy_shipped: &str,
) -> Result<Option<Ovr>, String> {
    let combo = format_combo(&display_mods(&e.was_mods), &e.was_key);

    // Placed by combo, never by the stored line number: that number indexes a
    // file this build no longer reads, and the two maps do not agree on it, so
    // carrying it across would rebind whatever now happens to sit on that line.
    let mut hits = shipped.iter().filter(|b| same_combo(&b.mods, &b.key, &e.was_mods, &e.was_key));
    let target = hits
        .next()
        .ok_or_else(|| {
            format!("{combo} is no longer in the shipped map — rebind it from Settings")
        })?
        .clone();
    if hits.next().is_some() {
        return Err(format!(
            "{combo} now matches more than one shipped bind, so there is no single \
             bind to move — rebind it from Settings"
        ));
    }
    reject_hold_bind(&target)?;
    validate::keybind_mods(&e.mods)?;
    validate::keybind_key(&e.key)?;

    let action = match e.action.strip_prefix("exec,") {
        // Faithful whichever command wrote the entry: the Lua map spells every
        // exec bind exactly this way, so this reproduces a `set-action` that
        // changed which app a key launches just as well as a plain rebind.
        Some(cmd) => format!("hl.dsp.exec_cmd({})", lua_string(cmd.trim())),
        // Any other hyprlang dispatcher (`killactive`, `movefocus, l`, …) has no
        // mechanical Lua equivalent — the shapes differ per dispatcher. Taking
        // what the Lua map now binds at this combo is exactly right for a rebind;
        // it is only wrong for a `set-action` onto a non-exec dispatcher, and
        // that is the case worth reporting rather than guessing at.
        None => {
            if legacy_action_was_customised(e, legacy_shipped) {
                eprintln!(
                    "  {} {combo} had a customised action ({}) with no mechanical Lua \
                     equivalent — it now runs the shipped action again; re-apply it \
                     from Settings",
                    term::yellow("!"),
                    e.action
                );
            }
            target.action.clone()
        }
    };

    let ovr = Ovr {
        line: target.line,
        was_mods: target.mods.clone(),
        was_key: target.key.clone(),
        opts: target.opts.clone(),
        mods: display_mods(&e.mods),
        key: normalize_key(&e.key),
        action,
        desc: if e.desc.is_empty() { target.desc.clone() } else { e.desc.clone() },
    };
    Ok((!restates_shipped(&ovr, &target)).then_some(ovr))
}

/// Record that the pre-Lua file has been consumed, leaving its contents intact.
fn mark_legacy_migrated(p: &Path, text: &str) -> Result<(), String> {
    let mut out = text.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(MIGRATED_MARKER);
    out.push('\n');
    atomic::write(p, &out)
}

/// Carry the pre-Lua override layer into `keybinds.lua`.
///
/// Idempotent and cheap: normally one `stat` and one small read. Runs before
/// every command and from `tezca link`, so upgrading needs no explicit step.
///
/// Deliberately more than `managed.rs`'s block copy, because both halves of the
/// entry changed shape in the cutover: an entry is *placed* by the combo it
/// recorded rather than by its line number, and its dispatcher is re-expressed
/// where that can be done faithfully and reported where it cannot.
fn migrate_legacy() -> Result<(), String> {
    // The Lua layer already carries overrides, so it is authoritative — and this
    // is also what stops a `keybind reset` from being undone by a re-migration.
    if !load_overrides()?.is_empty() {
        return Ok(());
    }
    let legacy_p = legacy_override_path()?;
    let Ok(legacy_text) = fs::read_to_string(&legacy_p) else { return Ok(()) };
    if legacy_text.contains(MIGRATED_MARKER) {
        return Ok(());
    }
    let entries = parse_legacy_overrides(&legacy_text);
    if entries.is_empty() {
        return Ok(()); // a header-only file (never rebound) — leave it alone
    }

    let base =
        fs::read_to_string(base_path()?).map_err(|e| format!("cannot read keybinds.lua: {e}"))?;
    let shipped = apply_overrides(&base, &[]).0;
    // Absent on a clone that dropped the hyprlang rollback; only costs the
    // rebind-vs-set-action distinction, so an empty string is a fine fallback.
    let legacy_shipped = fs::read_to_string(legacy_base_path()?).unwrap_or_default();

    let mut carried = Vec::new();
    for e in &entries {
        match migrate_entry(e, &shipped, &legacy_shipped) {
            Ok(Some(o)) => carried.push(o),
            Ok(None) => {}
            Err(why) => eprintln!("  {} {why}", term::yellow("skipped:")),
        }
    }

    if !carried.is_empty() {
        save_overrides(&carried)?;
        eprintln!(
            "  {} carried {} keybinding override(s) from {} → {}",
            term::dim("migrated:"),
            carried.len(),
            legacy_p.display(),
            override_path()?.display()
        );
        if hypr::in_session() {
            let _ = hypr::reload();
        }
    }
    // Marked even when nothing survived translation: every entry was reported
    // above, and re-reporting on every command would be noise, not news.
    mark_legacy_migrated(&legacy_p, &legacy_text)
}

// ---------------------------------------------------------------------------
// Lua string literals — the exec_cmd command, read and written
// ---------------------------------------------------------------------------

/// Quote a command as a Lua string literal, following the shipped map's own
/// convention: a plain `"…"`, or a `[[…]]` long bracket when the command carries
/// a quote or a backslash — widened to `[=[…]=]` and so on if it also contains
/// the closing sequence.
fn lua_string(s: &str) -> String {
    if !s.contains('"') && !s.contains('\\') {
        return format!("\"{s}\"");
    }
    let mut eqs = String::new();
    while s.contains(&format!("]{eqs}]")) {
        eqs.push('=');
    }
    format!("[{eqs}[{s}]{eqs}]")
}

/// The inverse of [`lua_string`]: the text of a Lua string literal, in either
/// form. None when `s` is not exactly one literal — an expression, a
/// concatenation, or a literal with something after it.
///
/// Only the escapes the writer can emit are decoded. An unrecognised one is kept
/// as written rather than dropped, because this text is shown to be edited: a
/// command that came back subtly different from the file would be re-saved that
/// way.
fn lua_unstring(s: &str) -> Option<String> {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix('"') {
        let body = rest.strip_suffix('"')?;
        let mut out = String::new();
        let mut it = body.chars();
        while let Some(c) = it.next() {
            match c {
                // An unescaped quote closed the literal earlier than the last
                // character, so this is a literal plus something else.
                '"' => return None,
                '\\' => match it.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    e @ ('\\' | '"' | '\'') => out.push(e),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                },
                _ => out.push(c),
            }
        }
        return Some(out);
    }
    // `[[…]]`, `[=[…]=]`, … — no escapes inside, by definition.
    let rest = t.strip_prefix('[')?;
    let eqs = rest.chars().take_while(|c| *c == '=').count();
    let close = format!("]{}]", "=".repeat(eqs));
    let body = rest.get(eqs..)?.strip_prefix('[')?.strip_suffix(&close)?;
    (!body.contains(&close)).then(|| body.to_string())
}

/// The command an `hl.dsp.exec_cmd("…")` bind runs, for the GUI to show as an
/// editable field. None for every other dispatcher, and for the `exec_cmd(cmd,
/// rules)` two-argument form — there is no second field to put the rules in, and
/// saving would silently drop them.
///
/// Also None if the command contains a control character, which cannot survive
/// the tab-separated machine format. Nothing in the shipped map does; a
/// hand-edited `conf.d/keybinds.lua` is not owed an editable field.
fn exec_command(action: &str) -> Option<String> {
    let inner = action.trim().strip_prefix("hl.dsp.exec_cmd(")?.strip_suffix(')')?;
    lua_unstring(inner).filter(|c| !c.chars().any(char::is_control))
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// `tezca keybind list [--machine]` — documented binds with line numbers.
/// Machine format:  `S\t<title>`  for a section, and for a bind
/// `B\t<line>\t<mods>\t<key>\t<desc>\t<action>\t<overridden 0|1>\t<editable 0|1>\t<exec>`.
/// Fields are only ever appended, so a parser that reads the first five and
/// ignores the rest keeps working.
///
/// `editable` is 0 for the two shapes [`reject_hold_bind`] refuses — a hold-bind
/// and a multi-line body. The GUI needs that *before* the click, so it can show
/// the combo as text rather than offering a capture box that could only fail.
/// `exec` is the command inside an `hl.dsp.exec_cmd(…)` action and empty for
/// every other dispatcher; see [`exec_command`].
fn cmd_list(args: &[&str]) -> Result<(), String> {
    let base =
        fs::read_to_string(base_path()?).map_err(|e| format!("cannot read keybinds.lua: {e}"))?;
    let (binds, stale) = apply_overrides(&base, &load_overrides()?);
    let machine = args.iter().any(|a| *a == "--machine" || *a == "-m");
    if !machine {
        warn_stale(&stale);
    }

    for (i, raw) in base.lines().enumerate() {
        if let Some(title) = section_title(raw) {
            if machine {
                println!("S\t{title}");
            } else {
                println!("\n{}", term::bold(&title));
            }
            continue;
        }
        let Some(b) = binds.iter().find(|b| b.line == i + 1) else { continue };
        if b.desc.is_empty() {
            continue; // only surface documented binds
        }
        if machine {
            println!(
                "B\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                b.line,
                b.mods,
                b.key,
                b.desc,
                b.action,
                u8::from(b.overridden),
                u8::from(reject_hold_bind(b).is_ok()),
                exec_command(&b.action).unwrap_or_default()
            );
        } else {
            let combo = format_combo(&b.mods, &b.key);
            let mark = if b.overridden { term::cyan("*") } else { " ".to_string() };
            println!("{mark} {:<24} {}", combo, term::dim(&b.desc));
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

    // Validate before anything is read or written: both values are formatted
    // verbatim into a config line.
    validate::keybind_mods(&mods)?;
    validate::keybind_key(&key)?;

    let (binds, stale) = effective()?;
    warn_stale(&stale);
    let target = binds
        .iter()
        .find(|b| b.line == line)
        .ok_or_else(|| format!("line {line} is not a bind line"))?
        .clone();
    reject_hold_bind(&target)?;

    // Guard: the bind must still carry the combo the GUI showed us — compared
    // against the *effective* combo, since that is what was displayed.
    if let (Some(em), Some(ek)) = (&expect_mods, &expect_key) {
        if !same_combo(&target.mods, &target.key, em, ek) {
            return Err("the keybindings changed since they were read — reopen Settings \
                        and try again"
                .to_string()
                .into());
        }
    }

    // Conflict check against every other bind, as it effectively stands.
    if !force {
        if let Some(other) = conflict(&binds, line, &mods, &key) {
            let what =
                if other.desc.is_empty() { format!("line {}", other.line) } else { other.desc };
            return Err(RebindErr::Conflict(format!(
                "{} is already bound to {what}",
                format_combo(&display_mods(&mods), &normalize_key(&key))
            )));
        }
    }

    // Read the shipped combo straight from the base map: that is what the override
    // has to release, and it stays correct across repeated rebinds.
    let base_bind = shipped(line)?;
    let ovr = Ovr {
        line,
        was_mods: base_bind.mods.clone(),
        was_key: base_bind.key.clone(),
        opts: base_bind.opts.clone(),
        mods: display_mods(&mods),
        key: normalize_key(&key),
        // Carry the effective action/desc, so a previous `set-action` is not lost.
        action: target.action.clone(),
        desc: target.desc.clone(),
    };
    commit(ovr, &base_bind)?;

    println!(
        "  {} {} → {}",
        term::green("✓"),
        term::dim(&format!("line {line}")),
        format_combo(&display_mods(&mods), &normalize_key(&key))
    );
    Ok(())
}

/// A hold-bind — hyprlang's `bindm`, now the `mouse = true` flag. Hyprland's
/// `unbind` does not release one, so an override would leave both the old and
/// the new binding active.
fn is_hold_bind(b: &Bind) -> bool {
    b.opts.contains("mouse")
}

/// Refuse to override what cannot be reproduced or released.
fn reject_hold_bind(b: &Bind) -> Result<(), String> {
    if is_hold_bind(b) {
        return Err(format!(
            "line {} is a `mouse = true` hold-bind, which cannot be overridden \
             (Hyprland's `hl.unbind` does not release mouse/hold binds) — \
             edit conf.d/keybinds.lua directly",
            b.line
        ));
    }
    if b.action.is_empty() {
        return Err(format!(
            "line {} is a multi-line bind (a `function() … end` body), which \
             cannot be reproduced in the override layer — \
             edit conf.d/keybinds.lua directly",
            b.line
        ));
    }
    Ok(())
}

/// The first *other* bind that already uses this combo.
fn conflict(binds: &[Bind], skip_line: usize, mods: &str, key: &str) -> Option<Bind> {
    binds
        .iter()
        .find(|b| {
            if b.line == skip_line {
                return false;
            }
            // Hold-binds live in their own namespace, so sharing a combo with one
            // is not a conflict — that is how `$mod+Z` doubles as hold-to-move.
            if is_hold_bind(b) {
                return false;
            }
            same_combo(&b.mods, &b.key, mods, key)
        })
        .cloned()
}

/// The bind exactly as shipped, ignoring the override layer.
fn shipped(line: usize) -> Result<Bind, String> {
    let base =
        fs::read_to_string(base_path()?).map_err(|e| format!("cannot read keybinds.lua: {e}"))?;
    base.lines()
        .enumerate()
        .find_map(|(i, raw)| (i + 1 == line).then(|| parse_bind(line, raw)).flatten())
        .ok_or_else(|| format!("line {line} is not a bind line"))
}

/// Upsert an override — or drop it, when it would only restate the shipped bind.
fn commit(ovr: Ovr, base_bind: &Bind) -> Result<(), String> {
    let mut ovrs = load_overrides()?;
    ovrs.retain(|o| o.line != ovr.line);
    if !restates_shipped(&ovr, base_bind) {
        ovrs.push(ovr);
    }
    save_overrides(&ovrs)?;
    if hypr::in_session() {
        let _ = hypr::reload();
    }
    Ok(())
}

/// True when an override would say exactly what the shipped map already says, so
/// returning a key to its default leaves a clean override layer behind.
fn restates_shipped(o: &Ovr, base: &Bind) -> bool {
    same_combo(&o.mods, &o.key, &base.mods, &base.key)
        && o.action == base.action
        && o.desc == base.desc
}

// ---------------------------------------------------------------------------
// set-action  (change what a bind does — dispatcher + args)
// ---------------------------------------------------------------------------

/// `tezca keybind set-action --line N --action 'hl.dsp.exec_cmd("firefox")'
/// [--desc "Firefox"] [--expect-mods … --expect-key …]` — change one bind's
/// dispatcher (and optionally its label), keeping its combo.
///
/// `--exec <command>` is the same thing for the common case, and the form the
/// Settings page uses: it wraps the command in a Lua literal here rather than
/// asking a caller to quote one. The two are mutually exclusive.
fn cmd_set_action(args: &[&str]) -> Result<(), String> {
    let mut line: Option<usize> = None;
    let mut action: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut desc: Option<String> = None;
    let mut expect_mods: Option<String> = None;
    let mut expect_key: Option<String> = None;

    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--line" => line = it.next().and_then(|v| v.parse().ok()),
            "--action" => action = it.next().map(str::to_string),
            "--exec" => exec = it.next().map(str::to_string),
            "--desc" => desc = it.next().map(str::to_string),
            "--expect-mods" => expect_mods = it.next().map(str::to_string),
            "--expect-key" => expect_key = it.next().map(str::to_string),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let line = line.ok_or("set-action needs --line N")?;
    if action.is_some() && exec.is_some() {
        return Err("--action and --exec both say what the bind does — pass one".to_string());
    }
    let action = match (action, exec) {
        (Some(a), _) => a.trim().to_string(),
        (None, Some(cmd)) => {
            let cmd = cmd.trim();
            // The command becomes a Lua string, so quotes and backslashes are
            // escaped rather than refused — only what a literal cannot hold is
            // rejected, which is what `exec_line` checks.
            validate::exec_line(cmd)?;
            format!("hl.dsp.exec_cmd({})", lua_string(cmd))
        }
        (None, None) => return Err("set-action needs --action or --exec".to_string()),
    };

    validate::keybind_action(&action)?;
    if let Some(d) = &desc {
        validate::keybind_desc(d)?;
    }

    let (binds, stale) = effective()?;
    warn_stale(&stale);
    let target = binds
        .iter()
        .find(|b| b.line == line)
        .ok_or_else(|| format!("line {line} is not a bind line"))?
        .clone();
    reject_hold_bind(&target)?;

    if let (Some(em), Some(ek)) = (&expect_mods, &expect_key) {
        if !same_combo(&target.mods, &target.key, em, ek) {
            return Err(
                "the keybindings changed since they were read — reopen Settings and try again"
                    .into(),
            );
        }
    }

    let base_bind = shipped(line)?;
    let ovr = Ovr {
        line,
        was_mods: base_bind.mods.clone(),
        was_key: base_bind.key.clone(),
        opts: base_bind.opts.clone(),
        // Keep the combo as it effectively stands, so a previous rebind survives.
        mods: target.mods.clone(),
        key: target.key.clone(),
        action: action.clone(),
        desc: desc.filter(|s| !s.is_empty()).unwrap_or_else(|| target.desc.clone()),
    };
    commit(ovr, &base_bind)?;

    println!("  {} {} → {}", term::green("✓"), term::dim(&format!("line {line}")), action);
    Ok(())
}

fn cmd_restore() -> Result<(), String> {
    let bak = backup_path()?;
    let text = fs::read_to_string(&bak)
        .map_err(|_| "no snapshot to restore (change a keybinding first)".to_string())?;
    atomic::write(&override_path()?, &text)?;
    if hypr::in_session() {
        let _ = hypr::reload();
    }
    println!("  {} undid the last keybinding change", term::green("✓"));
    Ok(())
}

/// `tezca keybind reset [--line N]` — drop every override, or just one bind's.
///
/// The per-line form is what "restore this shortcut to its default" is in the
/// GUI. Dropping the entry rather than writing an override back to the shipped
/// combo is the whole point: the layer stays empty for keys you never changed,
/// so an upstream change to one of them still reaches you.
fn cmd_reset(args: &[&str]) -> Result<(), String> {
    let mut line: Option<usize> = None;
    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--line" => line = it.next().and_then(|v| v.parse().ok()),
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    let mut ovrs = load_overrides()?;
    let before = ovrs.len();
    match line {
        Some(n) => ovrs.retain(|o| o.line != n),
        None => ovrs.clear(),
    }
    let dropped = before - ovrs.len();
    // Still written when nothing matched: `restore` then has a snapshot that
    // says "this is where you were", and a no-op write is harmless.
    save_overrides(&ovrs)?;
    if hypr::in_session() {
        let _ = hypr::reload();
    }
    match line {
        Some(n) if dropped == 0 => {
            println!("  {} line {n} was already at its shipped binding", term::dim("·"))
        }
        Some(n) => {
            let b = shipped(n)?;
            println!(
                "  {} line {n} → {} (shipped)",
                term::green("✓"),
                format_combo(&b.mods, &b.key)
            );
        }
        None => println!(
            "  {} dropped {dropped} override(s) — back to the shipped keybindings",
            term::green("✓")
        ),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// capture — suspending the global binds while the GUI reads a shortcut
// ---------------------------------------------------------------------------

/// The submap the GUI's "press the new shortcut" box runs inside.
///
/// Hyprland consumes a bound combo before the focused window ever sees it, so a
/// capture box in a normal window can read only the combos that are *not* worth
/// rebinding: press SUPER+B to move it and the browser opens instead. A submap
/// suspends every global bind except its own, so for as long as one is active
/// the keys reach the window like any other typing.
const CAPTURE_SUBMAP: &str = "tezca-capture";

/// Defined at capture time through `hyprctl eval`, not in `conf.d/keybinds.lua`.
///
/// Two reasons. The shipped map is read line-by-line by this module, and an
/// `hl.bind` nested inside a `define_submap` body would be parsed as a top-level
/// bind — listed on the Keybinds page and offered for rebinding. And defining it
/// here means the escape hatch exists on any session, including one whose config
/// on disk predates this command.
///
/// The `_G` guard makes it idempotent: `define_submap` appends, so defining on
/// every capture would grow the bind list all session. The guard lives in the
/// same Lua state as the definition, so a `hyprctl reload` — which drops the
/// submap — drops the guard with it.
///
/// CTRL+ALT+Escape leaves the submap. That is the hatch: if the GUI dies
/// mid-capture and never runs `capture off`, one keypress restores every
/// binding instead of the session being left with no keyboard shortcuts at all.
///
/// Plain Escape is deliberately *not* it, however much it reads like the
/// obvious choice. A bind inside the submap is consumed by the compositor, so
/// binding Escape here would take Escape away from the window doing the
/// capturing — its "press Escape to cancel" would silently release the grab and
/// leave the box waiting for keys that had gone back to running their actions.
const DEFINE_CAPTURE_SUBMAP: &str = concat!(
    "if not _G.__tezca_capture_submap then _G.__tezca_capture_submap = true ",
    "hl.define_submap(\"tezca-capture\", function() ",
    "hl.bind(\"CTRL + ALT + Escape\", hl.dsp.submap(\"reset\")) end) end"
);

/// `tezca keybind capture on|off`.
///
/// `off` is deliberately willing to run at any time, including when no capture is
/// in progress: it is what the GUI calls from every path that could end one —
/// commit, cancel, the window losing focus, a watchdog — and each of those has to
/// be safe to fire twice.
fn cmd_capture(args: &[&str]) -> Result<(), String> {
    if !hypr::in_session() {
        return Err("not inside a Hyprland session, so there are no binds to suspend".to_string());
    }
    match args.first().copied() {
        Some("on") => {
            hypr::eval(DEFINE_CAPTURE_SUBMAP)?;
            hypr::dispatch(&format!("hl.dsp.submap(\"{CAPTURE_SUBMAP}\")"))?;
            println!(
                "  {} global keybindings suspended — CTRL+ALT+Escape releases them",
                term::dim("·")
            );
        }
        Some("off") => {
            hypr::dispatch("hl.dsp.submap(\"reset\")")?;
        }
        _ => return Err("capture needs on or off".to_string()),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Combo helpers
// ---------------------------------------------------------------------------

/// Canonical modifier set for *comparison*: $mod→SUPER, uppercased, sorted,
/// de-duplicated. Plain alphabetical order, so two spellings of the same combo
/// compare equal (`$mod SHIFT ALT` and `$mod ALT SHIFT` both appear in the
/// shipped map).
fn normalize_mods(mods: &str) -> String {
    let mut parts: Vec<String> =
        mods.replace("$mod", "SUPER").split_whitespace().map(|s| s.to_uppercase()).collect();
    parts.sort();
    parts.dedup();
    parts.join(" ")
}

/// Canonical modifier set for *display and storage*: the Tezca modifier first,
/// then the rest alphabetically.
///
/// Distinct from [`normalize_mods`] on purpose. Sorting plain-alphabetically is
/// what makes two spellings compare equal, but it also renders `SUPER SHIFT` as
/// `SHIFT $mod`, which reads backwards next to the shipped map's `$mod SHIFT`.
/// Keeping the two apart also makes the override layer round-trip byte-for-byte:
/// what we store is exactly what re-parsing the file gives back.
fn display_mods(mods: &str) -> String {
    let canonical = normalize_mods(mods);
    let mut parts: Vec<&str> = canonical.split_whitespace().collect();
    parts.sort_by_key(|m| (*m != "SUPER", *m));
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

fn print_help() {
    println!("{}", term::header("tezca keybind"));
    println!("{}", term::dim("  changes go to ~/.config/tezca/keybinds.lua;"));
    println!("{}", term::dim("  the shipped conf.d/keybinds.lua is never modified"));
    println!();
    println!(
        "  {}                              list effective binds (* = overridden)",
        term::cyan("list [--machine]")
    );
    println!(
        "  {}  move a bind to another combo",
        term::cyan("rebind --line N --mods \"SUPER SHIFT\" --key W")
    );
    println!(
        "  {}            change what a bind launches",
        term::cyan("set-action --line N --exec \"<cmd>\"")
    );
    println!(
        "  {}          change any other dispatcher",
        term::cyan("set-action --line N --action \"<lua>\"")
    );
    println!(
        "  {}                                       undo the last change",
        term::cyan("restore")
    );
    println!(
        "  {}                              drop one bind's override, or all of them",
        term::cyan("reset [--line N]")
    );
    println!(
        "  {}                                suspend the global binds while Settings",
        term::cyan("capture on|off")
    );
    println!("{}", term::dim("                                                reads a shortcut"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed stand-in for the shipped map, carrying the shapes that actually
    /// appear in it: literal and `mod ..` combos, bare and flagged binds, a
    /// hold-bind, a dispatcher whose argument contains both `--` and a comma,
    /// and a multi-line `function()` bind.
    const BASE: &str = r#"
-- ==== Windows ====
hl.bind("CTRL + Q",    hl.dsp.window.close())                       -- close focused window
hl.bind(mod .. " + W", hl.dsp.exec_cmd("uwsm app -- brave"))        -- Browser
hl.bind(mod .. " + C", hl.dsp.exec_cmd("code"), { repeating = true }) -- Editor
hl.bind(mod .. " + Z", hl.dsp.window.drag(), { mouse = true })      -- hold to move
-- ---- Media ----
hl.bind("XF86AudioMute", hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_SINK@ toggle"), { locked = true })  -- Mute
hl.bind(mod .. " + SHIFT + left", hl.dsp.window.resize({ x = -40, y = 0, relative = true }), { repeating = true }) -- Shrink
hl.bind("ALT + Tab", function()                                     -- cycle focus
    hl.dispatch(hl.dsp.window.cycle_next())
end)
hl.bind(mod .. " + T", hl.dsp.exec_cmd("alacritty"))                -- Terminal
hl.bind(mod .. " + U", hl.dsp.exec_cmd("undocumented-no-desc"))
"#;

    fn base_binds() -> Vec<Bind> {
        apply_overrides(BASE, &[]).0
    }

    fn at(line: usize) -> Bind {
        base_binds().into_iter().find(|b| b.line == line).unwrap()
    }

    /// An override of `line`, keeping the shipped action/desc.
    fn ovr(line: usize, mods: &str, key: &str) -> Ovr {
        let b = at(line);
        Ovr {
            line,
            was_mods: b.mods.clone(),
            was_key: b.key.clone(),
            opts: b.opts,
            mods: mods.to_string(),
            key: key.to_string(),
            action: b.action,
            desc: b.desc,
        }
    }

    #[test]
    fn parses_both_combo_spellings_and_keeps_the_dispatcher_whole() {
        // `mod .. " + W"` registers "SUPER + W", so it must normalize to SUPER —
        // that is the string `hl.unbind` will have to match.
        let brave = at(4);
        assert_eq!(brave.mods, "SUPER");
        assert_eq!(brave.key, "W");
        assert_eq!(brave.desc, "Browser");
        // The `--` inside the command must not be mistaken for the comment.
        assert_eq!(brave.action, r#"hl.dsp.exec_cmd("uwsm app -- brave")"#);
        assert_eq!(brave.opts, "");

        // A literal combo with no `mod` prefix.
        let close = at(3);
        assert_eq!((close.mods.as_str(), close.key.as_str()), ("CTRL", "Q"));

        // A bind with no modifier at all parses with an empty mods field, and
        // its flags land in `opts`.
        let mute = at(8);
        assert_eq!(mute.mods, "");
        assert_eq!(mute.key, "XF86AudioMute");
        assert_eq!(mute.opts, "{ locked = true }");
    }

    #[test]
    fn a_comma_inside_the_dispatcher_does_not_split_the_arguments() {
        // `resize({ x = -40, y = 0, relative = true })` has three commas nested
        // inside braces; splitting on any of them would truncate the action and
        // put garbage in the options slot.
        let shrink = at(9);
        assert_eq!(shrink.mods, "SUPER SHIFT");
        assert_eq!(shrink.key, "left");
        assert_eq!(shrink.action, "hl.dsp.window.resize({ x = -40, y = 0, relative = true })");
        assert_eq!(shrink.opts, "{ repeating = true }");
        assert_eq!(shrink.desc, "Shrink");
    }

    #[test]
    fn a_multiline_function_bind_is_listed_but_refused_for_rebinding() {
        let tab = at(10);
        assert_eq!((tab.mods.as_str(), tab.key.as_str()), ("ALT", "Tab"));
        assert_eq!(tab.action, "", "an unreproducible body must not be invented");
        // The description still has to survive, or `list` (which filters
        // undocumented binds) would hide the bind rather than show it as
        // present-but-not-rebindable.
        assert_eq!(tab.desc, "cycle focus");
        let e = reject_hold_bind(&tab).unwrap_err();
        assert!(e.contains("multi-line"), "{e}");
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_not_a_comment() {
        assert_eq!(trailing_comment(r#""x", hl.dsp.exec_cmd("uwsm app -- brave")"#), "");
        assert_eq!(
            trailing_comment(r#""x", hl.dsp.exec_cmd("uwsm app -- brave")) -- Browser"#),
            "Browser"
        );
        assert_eq!(trailing_comment(r#""x", hl.dsp.window.close())"#), "");
    }

    #[test]
    fn a_hold_bind_is_detected_from_its_mouse_flag() {
        let hold = at(6);
        assert!(is_hold_bind(&hold));
        assert!(reject_hold_bind(&hold).unwrap_err().contains("hold-bind"));
    }

    #[test]
    fn finds_section_titles_in_both_styles() {
        assert_eq!(section_title("-- ==== Windows ====").as_deref(), Some("Windows"));
        assert_eq!(section_title("-- ---- Media ----").as_deref(), Some("Media"));
        assert_eq!(section_title("-- a normal comment"), None);
        assert_eq!(section_title(r#"hl.bind("SUPER + W", hl.dsp.exec_cmd("x"))"#), None);
    }

    #[test]
    fn an_override_replaces_the_combo_but_keeps_the_action() {
        let (binds, stale) = apply_overrides(BASE, &[ovr(4, "SUPER SHIFT", "W")]);
        assert!(stale.is_empty());
        let b = binds.iter().find(|b| b.line == 4).unwrap();
        assert_eq!(b.mods, "SUPER SHIFT");
        assert_eq!(
            b.action, r#"hl.dsp.exec_cmd("uwsm app -- brave")"#,
            "the action must survive a rebind"
        );
        assert!(b.overridden);
        assert!(!binds.iter().filter(|b| b.line != 4).any(|b| b.overridden));
    }

    #[test]
    fn a_stale_override_is_reported_and_the_shipped_bind_stands() {
        // The shipped map changed under the override: line 4 is no longer SUPER+W.
        let mut o = ovr(4, "SUPER SHIFT", "W");
        o.was_key = "Q".to_string();
        let (binds, stale) = apply_overrides(BASE, &[o]);
        assert_eq!(stale.len(), 1, "the mismatch must be surfaced");
        let b = binds.iter().find(|b| b.line == 4).unwrap();
        assert_eq!(b.key, "W");
        assert!(!b.overridden);
    }

    #[test]
    fn every_unbind_is_emitted_before_every_bind() {
        // Move SUPER+T (line 13) onto SUPER+W, and SUPER+W (line 4) elsewhere. If
        // the unbind for line 13 came after line 4's replacement, it would cancel
        // it and the combo would end up bound to nothing.
        let out = render_overrides(&[ovr(13, "SUPER", "W"), ovr(4, "SUPER SHIFT", "B")]);
        let last_unbind = out.rfind("\nhl.unbind(").expect("unbinds present");
        let first_bind = out.find("\nhl.bind(").expect("binds present");
        assert!(last_unbind < first_bind, "all unbinds must precede all binds:\n{out}");
    }

    #[test]
    fn the_override_layer_round_trips_through_render_and_parse() {
        let a = ovr(4, "SUPER SHIFT", "B");
        // An empty modifier list has to survive the `was=|Key` encoding.
        let b = ovr(8, "", "XF86AudioMute");
        let parsed = parse_overrides(&render_overrides(&[a.clone(), b.clone()]));
        assert_eq!(parsed, vec![a, b]);
    }

    #[test]
    fn a_hold_bind_cannot_be_overridden() {
        let e = reject_hold_bind(&at(6)).unwrap_err();
        assert!(e.contains("hold-bind"), "{e}");
        assert!(reject_hold_bind(&at(4)).is_ok());
    }

    #[test]
    fn a_hold_bind_sharing_a_combo_is_not_a_conflict_but_a_real_bind_is() {
        let binds = base_binds();
        // A hold-bind is its own namespace: rebinding onto SUPER+Z is allowed.
        assert!(conflict(&binds, 4, "SUPER", "Z").is_none());
        // A plain bind that already owns the combo is a real conflict.
        let c = conflict(&binds, 4, "SUPER", "T").expect("SUPER+T is the terminal bind");
        assert_eq!(c.desc, "Terminal");
        // Rebinding a line onto its own current combo is not a conflict.
        assert!(conflict(&binds, 4, "SUPER", "W").is_none());
    }

    #[test]
    fn conflict_detection_sees_through_the_override_layer() {
        // Line 13 has been moved off SUPER+T, so SUPER+T is free for line 4 now…
        let (binds, _) = apply_overrides(BASE, &[ovr(13, "SUPER ALT", "T")]);
        assert!(conflict(&binds, 4, "SUPER", "T").is_none());
        // …and its new combo is the one that is taken.
        assert!(conflict(&binds, 4, "SUPER ALT", "T").is_some());
    }

    #[test]
    fn an_override_that_restates_the_shipped_bind_is_dropped() {
        let base = at(4);
        let restated = Ovr {
            line: 4,
            was_mods: base.mods.clone(),
            was_key: base.key.clone(),
            opts: base.opts.clone(),
            mods: display_mods("SUPER"),
            key: normalize_key("w"),
            action: base.action.clone(),
            desc: base.desc.clone(),
        };
        assert!(restates_shipped(&restated, &base), "a rebind back to SUPER+W is the default");

        let moved = ovr(4, "SUPER SHIFT", "W");
        assert!(!restates_shipped(&moved, &base));
    }

    #[test]
    fn combos_compare_independently_of_modifier_order_and_key_case() {
        assert!(same_combo("SUPER SHIFT", "W", "SHIFT SUPER", "w"));
        assert!(same_combo("$mod", "W", "SUPER", "W"));
        assert!(!same_combo("SUPER", "W", "SUPER SHIFT", "W"));
        assert!(!same_combo("SUPER", "W", "SUPER", "Q"));
    }

    #[test]
    fn writes_super_back_out_in_the_house_style() {
        // The override layer spells combos out in full: `hl.unbind` matches the
        // registered string exactly, and SUPER must lead so the rendered string
        // matches what `mod .. " + …"` produced in the shipped map.
        assert_eq!(format_combo(&display_mods("SUPER SHIFT"), "W"), "SUPER + SHIFT + W");
        assert_eq!(
            format_combo(&display_mods("SHIFT SUPER"), "W"),
            "SUPER + SHIFT + W",
            "input order must not matter"
        );
        assert_eq!(format_combo(&display_mods(""), "XF86AudioMute"), "XF86AudioMute");
        // Comparison order stays plain-alphabetical so equal combos compare equal.
        assert_eq!(normalize_mods("SUPER SHIFT"), "SHIFT SUPER");
        assert_eq!(normalize_key("w"), "W");
        assert_eq!(normalize_key("Left"), "left");
        assert_eq!(normalize_key("XF86AudioMute"), "XF86AudioMute");
    }

    // --- migrating the pre-Lua override layer ------------------------------

    /// The pre-Lua shipped map, trimmed to the same binds as `BASE` — but
    /// deliberately at *different line numbers* (an extra comment up top, and no
    /// multi-line ALT+Tab), which is the whole reason the migration cannot carry
    /// a line number across. `$mod, W` is line 5 here and line 4 in `BASE`.
    const LEGACY_BASE: &str = r#"
# ==== Windows ====
# a section note with no counterpart in the Lua map
bind  = CTRL, Q,          killactive                    # close focused window
bind  = $mod, W,          exec, uwsm app -- brave       # Browser
binde = $mod, C,          exec, code                    # Editor
bindm = $mod, Z,          movewindow                    # hold to move
# ---- Media ----
bindl = , XF86AudioMute,  exec, wpctl set-mute @DEFAULT_SINK@ toggle  # Mute
bind  = $mod, T,          exec, alacritty               # Terminal
"#;

    fn legacy_entry(header: &str, bind: &str) -> LegacyOvr {
        let text = format!("{header}\n{bind}\n");
        parse_legacy_overrides(&text).into_iter().next().expect("one entry")
    }

    fn migrated(header: &str, bind: &str) -> Result<Option<Ovr>, String> {
        migrate_entry(&legacy_entry(header, bind), &base_binds(), LEGACY_BASE)
    }

    #[test]
    fn parses_a_pre_lua_entry_and_expands_the_mod_variable() {
        let e = legacy_entry(
            "# @5 was=$mod|W",
            "bind = $mod SHIFT, B, exec, uwsm app -- brave  # Browser",
        );
        assert_eq!(e.line, 5);
        // `$mod` has to become SUPER on both halves, or the combo will not match
        // the Lua map (which spells every registered combo out in full).
        assert_eq!((e.was_mods.as_str(), e.was_key.as_str()), ("SUPER", "W"));
        assert_eq!((e.mods.as_str(), e.key.as_str()), ("SUPER SHIFT", "B"));
        assert_eq!(e.action, "exec, uwsm app -- brave");
        assert_eq!(e.desc, "Browser");
    }

    #[test]
    fn a_migrated_entry_is_placed_by_its_combo_not_its_line_number() {
        // The entry records line 5 (where $mod+W lives in the hyprlang map). In
        // the Lua map that line is the *Editor* bind — carrying the number over
        // would silently rebind the wrong key, which is the bug this guards.
        let o =
            migrated("# @5 was=$mod|W", "bind = $mod SHIFT, B, exec, uwsm app -- brave  # Browser")
                .unwrap()
                .expect("a real override");
        assert_eq!(o.line, 4, "must land on the Lua map's $mod+W, not on line 5");
        assert_eq!((o.was_mods.as_str(), o.was_key.as_str()), ("SUPER", "W"));
        assert_eq!((o.mods.as_str(), o.key.as_str()), ("SUPER SHIFT", "B"));
    }

    #[test]
    fn an_exec_dispatcher_is_re_expressed_as_a_lua_call() {
        let o =
            migrated("# @5 was=$mod|W", "bind = $mod SHIFT, B, exec, uwsm app -- brave  # Browser")
                .unwrap()
                .unwrap();
        assert_eq!(o.action, r#"hl.dsp.exec_cmd("uwsm app -- brave")"#);
    }

    #[test]
    fn an_exec_set_action_survives_even_though_the_dispatcher_changed_shape() {
        // The pre-Lua map ran `alacritty` on $mod+T; this entry had been pointed
        // at something else. `exec` translates faithfully, so the customisation
        // must come through rather than snapping back to the shipped command.
        let o = migrated("# @10 was=$mod|T", "bind = $mod, T, exec, uwsm app -- kitty  # Terminal")
            .unwrap()
            .expect("a set-action is not a no-op");
        assert_eq!(o.line, 13, "the Lua map's terminal bind");
        assert_eq!(o.action, r#"hl.dsp.exec_cmd("uwsm app -- kitty")"#);
    }

    #[test]
    fn a_non_exec_rebind_adopts_whatever_the_lua_map_now_dispatches() {
        // A plain rebind never touched the action, so the Lua map's own call is
        // the correct one — no hyprlang→Lua dispatcher table needed.
        let o =
            migrated("# @4 was=CTRL|Q", "bind = CTRL SHIFT, Q, killactive  # close focused window")
                .unwrap()
                .expect("the combo moved");
        assert_eq!(o.line, 3);
        assert_eq!(o.action, "hl.dsp.window.close()");
        assert_eq!((o.mods.as_str(), o.key.as_str()), ("CTRL SHIFT", "Q"));
    }

    #[test]
    fn a_customised_non_exec_action_is_detected_so_it_can_be_reported() {
        // Shipped line 4 is `killactive`; this entry says `fullscreen, 0`. That
        // is a set-action onto a dispatcher with no mechanical translation — the
        // one case the migration has to admit it cannot carry.
        let e = legacy_entry(
            "# @4 was=CTRL|Q",
            "bind = CTRL, Q, fullscreen, 0  # close focused window",
        );
        assert!(legacy_action_was_customised(&e, LEGACY_BASE));

        // A plain rebind must NOT be flagged, or every migration warns.
        let plain = legacy_entry(
            "# @4 was=CTRL|Q",
            "bind = CTRL SHIFT, Q, killactive  # close focused window",
        );
        assert!(!legacy_action_was_customised(&plain, LEGACY_BASE));

        // Without the old shipped map there is nothing to compare against, and
        // "assume it was a rebind" is the quiet answer.
        assert!(!legacy_action_was_customised(&e, ""));
    }

    #[test]
    fn entries_that_cannot_be_reproduced_are_refused_rather_than_guessed_at() {
        // A combo that no longer exists in the shipped map.
        let e =
            migrated("# @4 was=CTRL|Y", "bind = CTRL SHIFT, Y, killactive  # gone").unwrap_err();
        assert!(e.contains("no longer in the shipped map"), "{e}");

        // A hold-bind: `hl.unbind` cannot release one, so an override would leave
        // both bindings live.
        let e = migrated("# @7 was=$mod|Z", "bindm = $mod SHIFT, Z, movewindow  # hold to move")
            .unwrap_err();
        assert!(e.contains("hold-bind"), "{e}");

        // A hand-edited old file is still untrusted input, and it reaches the
        // generated Lua verbatim — so it goes through the same validator as the
        // live path rather than being trusted for having been on disk.
        let e =
            migrated("# @5 was=$mod|W", "bind = $mod, W;rm -rf ~, exec, x  # Browser").unwrap_err();
        assert!(e.contains("invalid character"), "{e}");
    }

    #[test]
    fn an_entry_that_restates_the_shipped_bind_is_dropped() {
        // Migrating a file whose entry now says exactly what the Lua map says
        // must leave a clean override layer, not a no-op entry.
        let o = migrated("# @5 was=$mod|W", "bind = $mod, W, exec, uwsm app -- brave  # Browser")
            .unwrap();
        assert!(o.is_none(), "a no-op override should not be carried over");
    }

    #[test]
    fn the_command_of_an_exec_bind_is_recovered_for_the_settings_field() {
        assert_eq!(exec_command(&at(4).action).as_deref(), Some("uwsm app -- brave"));
        assert_eq!(exec_command(&at(13).action).as_deref(), Some("alacritty"));
        // The `--` that trips a naive comment split is inside the literal, so it
        // has to come back whole.
        assert_eq!(
            exec_command(r#"hl.dsp.exec_cmd([[sh -c 'notify-send Tezca "x"']])"#).as_deref(),
            Some(r#"sh -c 'notify-send Tezca "x"'"#)
        );
        // Not an exec bind: there is no command to offer, and inventing one would
        // let the field overwrite a dispatcher with a shell command by accident.
        assert_eq!(exec_command(&at(3).action), None);
        assert_eq!(exec_command("hl.dsp.window.resize({ x = -40, y = 0 })"), None);
        // The two-argument form carries window rules this cannot round-trip.
        assert_eq!(exec_command(r#"hl.dsp.exec_cmd("foo", { float = true })"#), None);
        // A concatenation is an expression, not a literal.
        assert_eq!(exec_command(r#"hl.dsp.exec_cmd("a" .. "b")"#), None);
    }

    #[test]
    fn a_command_round_trips_through_the_lua_literal_it_is_written_as() {
        for cmd in [
            "walker -m files",
            "uwsm app -- brave-origin.desktop",
            r#"sh -c 'cliphist wipe && notify-send Tezca "cleared"'"#,
            r#"echo "a]]b""#,
            r"sh -c 'printf a\tb'",
            "cliphist list | walker -d -p Clipboard | cliphist decode | wl-copy",
        ] {
            let action = format!("hl.dsp.exec_cmd({})", lua_string(cmd));
            assert_eq!(exec_command(&action).as_deref(), Some(cmd), "round trip of {cmd:?}");
        }
    }

    #[test]
    fn commands_are_quoted_as_lua_strings_the_way_the_shipped_map_does() {
        assert_eq!(lua_string("walker -m files"), r#""walker -m files""#);
        // A quote in the command forces the long-bracket form — exactly what the
        // shipped map uses for the cliphist-wipe bind.
        assert_eq!(
            lua_string(r#"sh -c 'cliphist wipe && notify-send Tezca "cleared"'"#),
            r#"[[sh -c 'cliphist wipe && notify-send Tezca "cleared"']]"#
        );
        // …widened when the command itself contains the closing sequence.
        assert_eq!(lua_string(r#"echo "a]]b""#), r#"[=[echo "a]]b"]=]"#);
    }

    #[test]
    fn the_migration_marker_is_inert_to_both_readers() {
        // hyprlang ignores it as a comment, and it must not parse as an entry —
        // otherwise marking the file would corrupt what it is marking.
        assert!(MIGRATED_MARKER.starts_with('#'));
        assert!(parse_legacy_overrides(MIGRATED_MARKER).is_empty());
        assert!(parse_legacy_bind(MIGRATED_MARKER).is_none());
        assert!(parse_legacy_ovr_header(MIGRATED_MARKER).is_none());
    }

    #[test]
    fn a_header_only_pre_lua_file_carries_nothing() {
        // The shape on a machine that never rebound anything: the migration must
        // no-op rather than write an empty layer or report a migration.
        let header = "# ~/.config/tezca/keybinds.conf — generated by `tezca keybind`.\n\
                      #\n# Sourced by hyprland.conf AFTER conf.d/keybinds.conf.\n";
        assert!(parse_legacy_overrides(header).is_empty());
    }

    #[test]
    fn an_empty_override_layer_renders_to_just_the_header() {
        let out = render_overrides(&[]);
        assert_eq!(out, OVERRIDE_HEADER);
        // The header prose mentions `unbind`; what must be absent is a directive.
        assert!(!out.lines().any(|l| l.starts_with("unbind")));
        assert!(parse_overrides(&out).is_empty());
    }
}
