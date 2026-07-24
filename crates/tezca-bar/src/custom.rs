//! Community / user "custom" exec modules — a bar widget driven by a shell
//! command, in the spirit of Waybar's `custom/*`.
//!
//! Each module is a drop-in manifest file at
//! `~/.config/tezca-bar/modules/<name>.toml`, parsed with the same loose
//! `key = value` reader the rest of the bar uses. One module per file, so there
//! is no nested-table syntax and therefore no TOML-crate dependency — and a
//! module is a single file you can share. A module is placed by adding
//! `custom:<name>` to a `layout_*` region (see `config.rs`).
//!
//! The `exec` command runs on an interval; its stdout drives the widget. Output
//! is either plain text (the first non-empty line) or a JSON object
//! `{"text","tooltip","class"}` (a Waybar-compatible subset). `on_click` /
//! `on_right_click` run a shell command when the widget is clicked.
//!
//! Trust model: a custom module runs a command *you* placed in your own config
//! directory, with your privileges — exactly like a script referenced from a
//! Waybar config. Tezca adds no sandbox and makes no network request of its own
//! for these; what a module does is whatever its `exec` does.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_INTERVAL: u32 = 10;

/// A discovered custom-module manifest.
#[derive(Clone, Debug)]
pub struct CustomModule {
    pub name: String,
    pub exec: String,
    pub interval: u32,
    /// Display name for the Settings module list (falls back to a prettified name).
    pub label: String,
    /// Optional static leading glyph/text.
    pub icon: Option<String>,
    /// Static tooltip, used when the script emits none.
    pub tooltip: Option<String>,
    pub on_click: Option<String>,
    pub on_right_click: Option<String>,
}

/// One poll result for a module.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub name: String,
    pub text: String,
    pub tooltip: Option<String>,
    /// Extra CSS class(es) the script asked for (`.custom.<class>` in CSS).
    pub class: Option<String>,
}

/// `~/.config/tezca-bar/modules`.
pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tezca-bar").join("modules"))
}

/// Discover every custom-module manifest, in stable (name-sorted) order.
pub fn load() -> Vec<CustomModule> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: BTreeMap<String, CustomModule> = BTreeMap::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if let Some(m) = parse_manifest(stem, &text) {
            out.insert(m.name.clone(), m);
        }
    }
    out.into_values().collect()
}

/// Parse a manifest's flat `key = value` body. Returns None without an `exec` —
/// a module with nothing to run is meaningless.
fn parse_manifest(name: &str, text: &str) -> Option<CustomModule> {
    let mut exec = None;
    let mut interval = DEFAULT_INTERVAL;
    let mut label = None;
    let mut icon = None;
    let mut tooltip = None;
    let mut on_click = None;
    let mut on_right_click = None;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let k = k.trim();
        // Unwrap a value that is *wholly* quoted, but leave interior quotes
        // alone — a command like `echo "hi"` must survive intact. Commands also
        // legitimately contain '#', so (unlike the scalar reader) we never strip
        // trailing comments here.
        let v = unquote(v);
        match k {
            "exec" | "command" => exec = Some(v),
            "interval" => {
                if let Ok(n) = v.parse::<u32>() {
                    interval = n.max(1);
                }
            }
            "label" | "name" => label = Some(v),
            "icon" => icon = Some(v),
            "tooltip" => tooltip = Some(v),
            "on_click" | "on-click" => on_click = Some(v),
            "on_right_click" | "on-right-click" => on_right_click = Some(v),
            _ => {}
        }
    }
    let exec = exec.filter(|s| !s.is_empty())?;
    let nonempty = |o: Option<String>| o.filter(|s| !s.is_empty());
    Some(CustomModule {
        name: name.to_string(),
        exec,
        interval,
        label: nonempty(label).unwrap_or_else(|| pretty_name(name)),
        icon: nonempty(icon),
        tooltip: nonempty(tooltip),
        on_click: nonempty(on_click),
        on_right_click: nonempty(on_right_click),
    })
}

/// Strip surrounding quotes only when the whole value is wrapped in a matching
/// pair — so `"weather.sh"` → `weather.sh` but `echo "hi"` is left untouched.
fn unquote(v: &str) -> String {
    let v = v.trim();
    let b = v.as_bytes();
    if b.len() >= 2 {
        let (first, last) = (b[0], b[b.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return v[1..v.len() - 1].to_string();
        }
    }
    v.to_string()
}

fn pretty_name(name: &str) -> String {
    let s = name.replace(['-', '_'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => name.to_string(),
    }
}

/// Run one module's `exec` once and parse its stdout.
pub fn run_once(m: &CustomModule) -> Output {
    let raw = Command::new("sh")
        .arg("-c")
        .arg(&m.exec)
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut out = parse_output(&m.name, &raw);
    if out.tooltip.is_none() {
        out.tooltip = m.tooltip.clone();
    }
    out
}

#[derive(Deserialize)]
struct Json {
    text: Option<String>,
    tooltip: Option<String>,
    class: Option<ClassField>,
}

/// Waybar allows `class` to be a string or an array of strings; accept both.
#[derive(Deserialize)]
#[serde(untagged)]
enum ClassField {
    One(String),
    Many(Vec<String>),
}

/// Parse a module's stdout. A leading `{` means JSON; anything else is plain
/// text (the first non-empty line). Malformed JSON falls back to plain text so a
/// buggy script shows *something* rather than silently vanishing.
fn parse_output(name: &str, raw: &str) -> Output {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        if let Ok(j) = serde_json::from_str::<Json>(trimmed) {
            let class = j
                .class
                .map(|c| match c {
                    ClassField::One(s) => s,
                    ClassField::Many(v) => v.join(" "),
                })
                .filter(|s| !s.trim().is_empty());
            return Output {
                name: name.to_string(),
                text: j.text.unwrap_or_default().trim().to_string(),
                tooltip: j.tooltip.filter(|s| !s.is_empty()),
                class,
            };
        }
    }
    let text = trimmed.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    Output { name: name.to_string(), text, tooltip: None, class: None }
}

/// One background thread per module: run on its interval and send each result.
/// A slow or wedged script only stalls its own module (mirrors `ai::spawn`'s
/// thread-per-work model; the GTK loop only ever applies a finished `Output`).
pub fn spawn(mods: Vec<CustomModule>, tx: async_channel::Sender<Output>) {
    for m in mods {
        let tx = tx.clone();
        std::thread::Builder::new()
            .name(format!("tezca-custom-{}", m.name))
            .spawn(move || loop {
                let out = run_once(&m);
                if tx.send_blocking(out).is_err() {
                    return; // bar gone
                }
                std::thread::sleep(Duration::from_secs(m.interval.max(1) as u64));
            })
            .ok();
    }
}

/// Spawn a click command detached (best-effort), like the power button does.
pub fn run_action(cmd: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Human-readable report for `tezca-bar --custom-dump`: what was discovered and
/// what each module currently prints. The debugging entry point (no GTK).
pub fn dump(mods: &[CustomModule]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let dir = dir().map(|p| p.display().to_string()).unwrap_or_default();
    if mods.is_empty() {
        let _ = writeln!(s, "No custom modules found in {dir}");
        let _ = writeln!(s, "Drop a <name>.toml manifest there (see DESIGN.md / the example) to add one.");
        return s;
    }
    let _ = writeln!(s, "{} custom module(s) in {dir}:\n", mods.len());
    for m in mods {
        let out = run_once(m);
        let _ = writeln!(s, "  custom:{}  ({}, every {}s)", m.name, m.label, m.interval);
        let _ = writeln!(s, "    exec:    {}", m.exec);
        let _ = writeln!(s, "    text:    {:?}", out.text);
        if let Some(t) = &out.tooltip {
            let _ = writeln!(s, "    tooltip: {t:?}");
        }
        if let Some(c) = &out.class {
            let _ = writeln!(s, "    class:   {c:?}");
        }
        let _ = writeln!(s);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_needs_exec() {
        assert!(parse_manifest("x", "interval = 5\nlabel = X").is_none());
        assert!(parse_manifest("x", "exec = echo hi").is_some());
    }

    #[test]
    fn manifest_parses_fields_and_clamps_interval() {
        let m = parse_manifest(
            "weather",
            "exec = \"weather.sh\"\ninterval = 0\nicon = \u{f0590}\non-click = kitty\n",
        )
        .unwrap();
        assert_eq!(m.exec, "weather.sh");
        assert_eq!(m.interval, 1); // 0 clamps to the 1s floor
        assert_eq!(m.icon.as_deref(), Some("\u{f0590}"));
        assert_eq!(m.on_click.as_deref(), Some("kitty"));
        assert_eq!(m.label, "Weather"); // prettified from the file stem
    }

    #[test]
    fn interior_quotes_in_a_command_survive_only_whole_wrapping_is_stripped() {
        // Interior quotes must not be touched, or the command breaks.
        let m = parse_manifest("hi", r#"exec = echo "  hello""#).unwrap();
        assert_eq!(m.exec, r#"echo "  hello""#);
        // A wholly single-quoted printf JSON payload keeps its inner quotes.
        let m = parse_manifest("w", "exec = 'printf \"{}\"'").unwrap();
        assert_eq!(m.exec, "printf \"{}\"");
        // A value that IS wholly wrapped gets unwrapped.
        assert_eq!(parse_manifest("x", "exec = \"a.sh\"").unwrap().exec, "a.sh");
    }

    #[test]
    fn plain_text_output_takes_first_nonempty_line() {
        let o = parse_output("m", "\n  42%  \nsecond line\n");
        assert_eq!(o.text, "42%");
        assert!(o.tooltip.is_none());
        assert!(o.class.is_none());
    }

    #[test]
    fn json_output_maps_text_tooltip_and_string_class() {
        let o = parse_output("m", r#"{"text":"  18°","tooltip":"Clear","class":"cold"}"#);
        assert_eq!(o.text, "18°");
        assert_eq!(o.tooltip.as_deref(), Some("Clear"));
        assert_eq!(o.class.as_deref(), Some("cold"));
    }

    #[test]
    fn json_class_array_joins_with_spaces() {
        let o = parse_output("m", r#"{"text":"x","class":["a","b"]}"#);
        assert_eq!(o.class.as_deref(), Some("a b"));
    }

    #[test]
    fn malformed_json_falls_back_to_plain_text() {
        let o = parse_output("m", "{not valid json");
        assert_eq!(o.text, "{not valid json");
        assert!(o.class.is_none());
    }
}
