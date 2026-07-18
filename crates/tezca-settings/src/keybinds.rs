//! Keybind data for the Keybinds page — loaded from `tezca keybind list
//! --machine`, so the CLI owns the single authoritative parse of keybinds.conf
//! (line numbers included, needed for rebinding). Machine format:
//!   `S\t<title>`                         a section header
//!   `B\t<line>\t<mods>\t<key>\t<desc>`    one documented bind

use crate::backend;

#[derive(Clone)]
pub struct Bind {
    pub line: usize,
    pub mods: String, // normalized, SUPER (not $mod)
    pub key: String,
    pub desc: String,
}

pub struct Section {
    pub title: String,
    pub binds: Vec<Bind>,
}

impl Bind {
    /// "SUPER + SHIFT + W" for display.
    pub fn combo(&self) -> String {
        let mut parts: Vec<&str> = self.mods.split_whitespace().collect();
        if !self.key.is_empty() {
            parts.push(&self.key);
        }
        parts.join(" + ")
    }
}

pub fn load() -> Vec<Section> {
    let Some(out) = backend::tezca_out(&["keybind", "list", "--machine"]) else {
        return Vec::new();
    };
    let mut sections: Vec<Section> = Vec::new();
    for line in out.lines() {
        let mut f = line.split('\t');
        match f.next() {
            Some("S") => {
                let title = f.next().unwrap_or("").to_string();
                sections.push(Section { title, binds: Vec::new() });
            }
            Some("B") => {
                let line_no: usize = f.next().unwrap_or("0").parse().unwrap_or(0);
                let mods = f.next().unwrap_or("").to_string();
                let key = f.next().unwrap_or("").to_string();
                let desc = f.next().unwrap_or("").to_string();
                if sections.is_empty() {
                    sections.push(Section { title: "General".into(), binds: Vec::new() });
                }
                sections
                    .last_mut()
                    .unwrap()
                    .binds
                    .push(Bind { line: line_no, mods, key, desc });
            }
            _ => {}
        }
    }
    sections.retain(|s| !s.binds.is_empty());
    sections
}
