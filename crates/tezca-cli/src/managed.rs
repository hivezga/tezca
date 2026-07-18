//! The `tezca-settings` managed override block in conf.d/local.conf.
//!
//! `tezca hypr` and `tezca display` persist live tweaks here so they survive
//! `hyprctl reload` and relogin. The block is delimited by two marker comments
//! and holds plain Hyprland config lines: flat `category:name = value` keywords
//! (verified accepted by the 0.55 config parser) and `monitor = …` lines. Every
//! entry carries a stable KEY so re-setting an option replaces its line instead
//! of appending a duplicate. Everything OUTSIDE the block is preserved verbatim,
//! so hand-written machine tweaks above it are never touched.

use crate::repo;
use std::fs;
use std::path::PathBuf;

const BEGIN: &str = "# >>> tezca-settings managed (auto-generated — edit via `tezca hypr`/settings) >>>";
const END: &str = "# <<< tezca-settings managed <<<";

/// Live path to conf.d/local.conf (through the ~/.config/hypr symlink → repo).
fn path() -> Result<PathBuf, String> {
    Ok(repo::config_home()?
        .join("hypr")
        .join("conf.d")
        .join("local.conf"))
}

/// The stable identity of a managed line — what we dedup on.
///   `decoration:rounding = 12`   → "decoration:rounding"
///   `monitor = DP-1,3440x1440@165,0x0,1` → "monitor:DP-1"
fn line_key(l: &str) -> Option<String> {
    let t = l.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let (lhs, rhs) = t.split_once('=')?;
    let lhs = lhs.trim();
    if lhs == "monitor" {
        let name = rhs.trim().split(',').next()?.trim();
        if name.is_empty() {
            return None;
        }
        Some(format!("monitor:{name}"))
    } else {
        Some(lhs.to_string())
    }
}

/// Upsert one managed line, keyed by `key`. Creates the block if absent.
pub fn set(key: &str, line: &str) -> Result<(), String> {
    let mut lines = read_block()?;
    if let Some(slot) = lines.iter_mut().find(|l| line_key(l).as_deref() == Some(key)) {
        *slot = line.to_string();
    } else {
        lines.push(line.to_string());
    }
    write_block(&lines)
}

/// Remove the managed line with the given key (no-op if absent).
pub fn remove(key: &str) -> Result<(), String> {
    let mut lines = read_block()?;
    let before = lines.len();
    lines.retain(|l| line_key(l).as_deref() != Some(key));
    if lines.len() == before {
        return Ok(());
    }
    write_block(&lines)
}

/// Remove every managed line whose key satisfies `pred`. Used by resets
/// (e.g. drop all `decoration:*`/`general:*` keywords but keep `monitor:*`).
pub fn remove_where(pred: impl Fn(&str) -> bool) -> Result<(), String> {
    let mut lines = read_block()?;
    lines.retain(|l| match line_key(l) {
        Some(k) => !pred(&k),
        None => true,
    });
    write_block(&lines)
}

/// The keys currently persisted in the managed block.
pub fn keys() -> Vec<String> {
    read_block()
        .unwrap_or_default()
        .iter()
        .filter_map(|l| line_key(l))
        .collect()
}

// ---------------------------------------------------------------------------

/// The non-empty, non-marker lines currently inside the managed block.
fn read_block() -> Result<Vec<String>, String> {
    let p = path()?;
    let text = fs::read_to_string(&p).unwrap_or_default();
    let mut out = Vec::new();
    let mut inside = false;
    for l in text.lines() {
        match l.trim() {
            BEGIN => inside = true,
            END => inside = false,
            t if inside && !t.is_empty() && !t.starts_with('#') => out.push(t.to_string()),
            _ => {}
        }
    }
    Ok(out)
}

/// Rewrite the file: preserve everything outside the block, replace the block
/// with `lines` (dropping it entirely when empty).
fn write_block(lines: &[String]) -> Result<(), String> {
    let p = path()?;
    let text = fs::read_to_string(&p).unwrap_or_default();

    // Keep everything not inside the old block.
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for l in text.lines() {
        match l.trim() {
            BEGIN => inside = true,
            END => inside = false,
            _ if !inside => kept.push(l),
            _ => {}
        }
    }
    // Trim trailing blank lines so we don't accumulate whitespace each write.
    while matches!(kept.last(), Some(s) if s.trim().is_empty()) {
        kept.pop();
    }

    let mut body = kept.join("\n");
    if !lines.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(BEGIN);
        body.push('\n');
        for l in lines {
            body.push_str(l);
            body.push('\n');
        }
        body.push_str(END);
    }
    body.push('\n');
    fs::write(&p, body).map_err(|e| format!("cannot write {}: {e}", p.display()))
}
