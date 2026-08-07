//! Keybind data for the Keybinds page — loaded from `tezca keybind list
//! --machine`, so the CLI owns the single authoritative parse of keybinds.lua
//! (line numbers included, needed for rebinding). Machine format:
//!   `S\t<title>`                             a section header
//!   `B\t<line>\t<mods>\t<key>\t<desc>\t<action>\t<overridden>\t<editable>\t<exec>`
//!
//! The last three fields are what makes the page an editor rather than a table:
//! whether this bind has been changed from the shipped default (so it can offer
//! to put it back), whether it can be changed at all, and — for a bind that
//! launches something — the command by itself, so the page can show a text field
//! instead of a Lua expression. They are read defensively: an older `tezca` on
//! PATH emits six fields, and the page should degrade to read-only rather than
//! claim every bind is a locked non-exec one.

use crate::backend;

#[derive(Clone)]
pub struct Bind {
    pub line: usize,
    pub mods: String, // normalized, SUPER (not $mod)
    pub key: String,
    pub desc: String,
    pub action: String, // the Lua dispatcher, e.g. `hl.dsp.exec_cmd("brave")`
    /// An override layer entry replaced this bind.
    pub overridden: bool,
    /// False for the shapes the CLI refuses to override — a hold-bind and a
    /// multi-line `function() … end` body.
    pub editable: bool,
    /// The command an `exec_cmd` bind runs; empty for any other dispatcher.
    pub exec: String,
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
                let action = f.next().unwrap_or("").to_string();
                let overridden = f.next() == Some("1");
                // Absent means an older CLI, which had no unrebindable shapes to
                // report because it had no GUI asking. Assume editable and let
                // the CLI refuse the two it cannot do, with its own message.
                let editable = !matches!(f.next(), Some("0"));
                let exec = f.next().unwrap_or("").to_string();
                if sections.is_empty() {
                    sections.push(Section { title: "General".into(), binds: Vec::new() });
                }
                sections.last_mut().unwrap().binds.push(Bind {
                    line: line_no,
                    mods,
                    key,
                    desc,
                    action,
                    overridden,
                    editable,
                    exec,
                });
            }
            _ => {}
        }
    }
    sections.retain(|s| !s.binds.is_empty());
    sections
}
