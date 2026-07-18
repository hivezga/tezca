//! Parse conf.d/keybinds.conf into sections for the Keybinds page. Reads the
//! linked config so it always mirrors the live map — the same source the
//! SUPER+/ cheat-sheet script reads. Only binds carrying a trailing `# comment`
//! are shown (undocumented grids like the workspace numbers are skipped).

use std::path::PathBuf;

pub struct Section {
    pub title: String,
    pub binds: Vec<(String, String)>, // (combo, description)
}

pub fn load() -> Vec<Section> {
    let Some(path) = path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse(&text)
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("hypr").join("conf.d").join("keybinds.conf"))
}

fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<Section> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if let Some(title) = section_title(line) {
            push(&mut sections, current.take());
            current = Some(Section { title, binds: Vec::new() });
            continue;
        }
        if let Some((combo, desc)) = bind_line(line) {
            current
                .get_or_insert_with(|| Section {
                    title: "General".to_string(),
                    binds: Vec::new(),
                })
                .binds
                .push((combo, desc));
        }
    }
    push(&mut sections, current.take());
    sections
}

fn push(sections: &mut Vec<Section>, s: Option<Section>) {
    if let Some(s) = s {
        if !s.binds.is_empty() {
            sections.push(s);
        }
    }
}

/// "# ===== Window management =====" or "# ----- Focus -----" → the inner title.
fn section_title(line: &str) -> Option<String> {
    let c = line.strip_prefix('#')?.trim();
    let first = c.as_bytes().first().copied()?;
    if first != b'=' && first != b'-' {
        return None;
    }
    let inner = c.trim_matches(|ch| ch == '=' || ch == '-' || ch == ' ');
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

/// `bind[elm]* = MODS, KEY, dispatcher, …   # description` → (combo, description).
fn bind_line(line: &str) -> Option<(String, String)> {
    if !line.starts_with("bind") {
        return None;
    }
    let after = &line[line.find('=')?  + 1..];
    let (body, desc) = after.split_once('#')?;
    let desc = desc.trim().to_string();
    if desc.is_empty() {
        return None;
    }
    let mut it = body.split(',');
    let mods = it.next().unwrap_or("").trim();
    let key = it.next().unwrap_or("").trim();
    Some((format_combo(mods, key), desc))
}

fn format_combo(mods: &str, key: &str) -> String {
    let mods = mods.replace("$mod", "SUPER");
    let mut parts: Vec<String> = mods.split_whitespace().map(str::to_string).collect();
    if !key.is_empty() {
        parts.push(key.to_string());
    }
    parts.join(" + ")
}
