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
//!
//! Resource limits, though, are enforced. A module that hangs or floods stdout is
//! a mistake, not a threat, and it used to take the widget down silently: the poll
//! used a blocking `Command::output()`, so a wedged script parked its thread
//! forever (the module froze on its last value with no indication) and unbounded
//! stdout was read straight into memory. Each poll now has a timeout
//! (`timeout =` in the manifest, default [`DEFAULT_TIMEOUT`]s) and an output cap
//! ([`MAX_OUTPUT`]), and a module that fails shows a visible error marker instead
//! of disappearing.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_INTERVAL: u32 = 10;

/// How long a module's `exec` may run before it is killed, in seconds. Overridable
/// per module with `timeout = N`.
const DEFAULT_TIMEOUT: u32 = 10;

/// Most stdout we will read from one run, in bytes. A widget shows a line of text;
/// anything approaching this is a runaway script, and reading it all would grow the
/// bar's memory without bound.
const MAX_OUTPUT: usize = 64 * 1024;

/// After a failed poll, wait this many times the interval before trying again
/// (capped at [`MAX_ERROR_BACKOFF`]). A script that hangs costs a whole timeout
/// per attempt, so retrying it at full speed is pure waste.
const ERROR_BACKOFF: u32 = 3;
const MAX_ERROR_BACKOFF: u32 = 300;

/// A discovered custom-module manifest.
#[derive(Clone, Debug)]
pub struct CustomModule {
    pub name: String,
    pub exec: String,
    pub interval: u32,
    /// Seconds before a run is killed. See [`DEFAULT_TIMEOUT`].
    pub timeout: u32,
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
    /// Why this poll produced nothing usable, if it failed. Rendered as a visible
    /// marker with an `error` class rather than hiding the module, so a broken
    /// script is obvious instead of looking like one you never configured.
    pub error: Option<String>,
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
    let mut timeout = DEFAULT_TIMEOUT;
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
            "timeout" => {
                if let Ok(n) = v.parse::<u32>() {
                    timeout = n.clamp(1, 120);
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
        timeout,
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

/// What one run of a module's command produced.
struct Capture {
    text: String,
    /// True when we stopped reading at [`MAX_OUTPUT`]. The exit status is then
    /// meaningless — closing the pipe is what killed the child.
    truncated: bool,
}

/// Run one module's `exec` once and parse its stdout.
///
/// A failure is reported, not swallowed: the old version filtered on
/// `status.success()` and fell back to an empty string, so a broken script made the
/// widget silently disappear and looked identical to one that was never configured.
pub fn run_once(m: &CustomModule) -> Output {
    match capture(m) {
        Ok(cap) => {
            let mut out = parse_output(&m.name, &cap.text);
            if out.tooltip.is_none() {
                out.tooltip = m.tooltip.clone();
            }
            if cap.truncated {
                out.error = Some(format!("output truncated at {} KiB", MAX_OUTPUT / 1024));
            }
            out
        }
        Err(e) => Output {
            name: m.name.clone(),
            text: String::new(),
            tooltip: m.tooltip.clone(),
            class: None,
            error: Some(e),
        },
    }
}

/// Run the command with a timeout and an output cap.
///
/// stdout is read on a helper thread, not this one. That is the whole point: a
/// script that writes nothing and never exits would block a read on this thread
/// forever, and the timeout would never be reached. With the read on its own
/// thread, `recv_timeout` bounds the wait and we can kill the child.
///
/// One residual case is not fixable without non-blocking IO, which would mean a
/// dependency: if the command spawns a grandchild that inherits stdout and outlives
/// it, the pipe never reaches EOF, so the reader thread stays parked until that
/// grandchild exits. The timeout still returns on schedule and the module still
/// reports the failure; only the helper thread lingers. The error backoff in
/// [`spawn`] keeps a module like that from accumulating one per interval.
fn capture(m: &CustomModule) -> Result<Capture, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&m.exec)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start: {e}"))?;

    let stdout = child.stdout.take().ok_or("could not capture stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("tezca-custom-read-{}", m.name))
        .spawn(move || {
            let mut buf = Vec::new();
            // `Read::take` is the cap: it stops the read at MAX_OUTPUT rather than
            // growing the buffer to whatever the script feels like emitting.
            let _ = stdout.take(MAX_OUTPUT as u64).read_to_end(&mut buf);
            let _ = tx.send(buf);
        })
        .map_err(|e| format!("could not start reader: {e}"))?;

    let timeout = Duration::from_secs(m.timeout.max(1) as u64);
    let Ok(buf) = rx.recv_timeout(timeout) else {
        let _ = child.kill();
        let _ = child.wait(); // reap, so a timing-out module leaves no zombies
        return Err(format!("timed out after {}s", timeout.as_secs()));
    };

    let truncated = buf.len() >= MAX_OUTPUT;
    let status = child.wait().map_err(|e| format!("could not wait: {e}"))?;
    // When we hit the cap, dropping the pipe is what ended the child, so its exit
    // status says nothing about whether the script worked. Report the truncation and
    // use what we read.
    if !truncated && !status.success() {
        return Err(match status.code() {
            Some(c) => format!("exited {c}"),
            None => "killed by a signal".to_string(),
        });
    }
    Ok(Capture { text: String::from_utf8_lossy(&buf).into_owned(), truncated })
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
                error: None,
            };
        }
    }
    let text = trimmed.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    Output { name: name.to_string(), text, tooltip: None, class: None, error: None }
}

/// One background thread per module: run on its interval and send each result.
/// A slow or wedged script only stalls its own module (mirrors `ai::spawn`'s
/// thread-per-work model; the GTK loop only ever applies a finished `Output`).
pub fn spawn(mods: Vec<CustomModule>, tx: async_channel::Sender<Output>) {
    for m in mods {
        let tx = tx.clone();
        std::thread::Builder::new()
            .name(format!("tezca-custom-{}", m.name))
            .spawn(move || {
                let mut failures: u32 = 0;
                loop {
                    let out = run_once(&m);
                    failures = if out.error.is_some() { failures.saturating_add(1) } else { 0 };
                    if tx.send_blocking(out).is_err() {
                        return; // bar gone
                    }
                    // A failing module is usually a hang, which costs a full timeout
                    // per attempt. Slow down instead of burning a timeout every
                    // interval — and, for the grandchild case in `capture`, stop
                    // accruing parked reader threads at interval speed.
                    let base = m.interval.max(1);
                    let wait = if failures == 0 {
                        base
                    } else {
                        base.saturating_mul(ERROR_BACKOFF).min(MAX_ERROR_BACKOFF)
                    };
                    std::thread::sleep(Duration::from_secs(wait as u64));
                }
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
        let _ = writeln!(
            s,
            "  custom:{}  ({}, every {}s, {}s timeout)",
            m.name, m.label, m.interval, m.timeout
        );
        let _ = writeln!(s, "    exec:    {}", m.exec);
        if let Some(e) = &out.error {
            let _ = writeln!(s, "    ERROR:   {e}");
        }
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
            "exec = \"weather.sh\"\ninterval = 0\nicon = \u{f0590}\non-click = alacritty\n",
        )
        .unwrap();
        assert_eq!(m.exec, "weather.sh");
        assert_eq!(m.interval, 1); // 0 clamps to the 1s floor
        assert_eq!(m.timeout, DEFAULT_TIMEOUT);
        assert_eq!(m.icon.as_deref(), Some("\u{f0590}"));
        assert_eq!(m.on_click.as_deref(), Some("alacritty"));
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
    fn a_timeout_is_clamped_to_a_sane_range() {
        assert_eq!(parse_manifest("x", "exec = a\ntimeout = 0").unwrap().timeout, 1);
        assert_eq!(parse_manifest("x", "exec = a\ntimeout = 5").unwrap().timeout, 5);
        assert_eq!(parse_manifest("x", "exec = a\ntimeout = 9999").unwrap().timeout, 120);
        // Unparseable values leave the default alone rather than zeroing it.
        assert_eq!(parse_manifest("x", "exec = a\ntimeout = soon").unwrap().timeout, DEFAULT_TIMEOUT);
    }

    fn module(exec: &str, timeout: u32) -> CustomModule {
        CustomModule {
            name: "t".into(),
            exec: exec.into(),
            interval: 10,
            timeout,
            label: "T".into(),
            icon: None,
            tooltip: None,
            on_click: None,
            on_right_click: None,
        }
    }

    #[test]
    fn a_successful_module_reports_its_output_and_no_error() {
        let out = run_once(&module("echo 42%", 5));
        assert_eq!(out.text, "42%");
        assert!(out.error.is_none());
    }

    #[test]
    fn a_hanging_module_times_out_instead_of_wedging_its_thread_forever() {
        // The regression this guards: `Command::output()` blocked here forever, so
        // the module froze on its last value and never polled again.
        let started = std::time::Instant::now();
        let out = run_once(&module("sleep 30", 1));
        let elapsed = started.elapsed();
        assert!(out.error.as_deref().unwrap_or("").contains("timed out"), "{:?}", out.error);
        assert!(elapsed < Duration::from_secs(10), "returned in {elapsed:?}, so it did not wait");
    }

    #[test]
    fn a_failing_module_reports_the_exit_status_rather_than_vanishing() {
        let out = run_once(&module("exit 3", 5));
        assert_eq!(out.error.as_deref(), Some("exited 3"));
        // The widget shows a marker for this instead of hiding itself.
        assert!(out.text.is_empty());
    }

    #[test]
    fn a_flood_of_output_is_capped_rather_than_read_into_memory() {
        // `yes` never stops. Previously this grew the bar's RSS without bound.
        let out = run_once(&module("yes tezca", 10));
        assert!(
            out.error.as_deref().unwrap_or("").contains("truncated"),
            "{:?}",
            out.error
        );
        // We still show the first line: partial output beats no output.
        assert_eq!(out.text, "tezca");
    }

    #[test]
    fn malformed_json_falls_back_to_plain_text() {
        let o = parse_output("m", "{not valid json");
        assert_eq!(o.text, "{not valid json");
        assert!(o.class.is_none());
    }
}
