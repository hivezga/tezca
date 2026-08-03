//! Shelling out to `tezca` + reading its state files. The panel does no real
//! work itself — every action is a `tezca` / hyprctl / script call, the same
//! thing the keybinds do, so the GUI and keyboard paths stay identical.

use serde::Serialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

/// Absolute path to the `tezca` binary — prefer ~/.local/bin (where install.sh
/// puts it; not always on a GUI process's PATH), else fall back to PATH lookup.
pub fn tezca_bin() -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/bin/tezca");
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    "tezca".to_string()
}

// ---------------------------------------------------------------------------
// CLI echo
// ---------------------------------------------------------------------------

/// What became of the command the echo footer is showing.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EchoState {
    /// Spawned detached — we deliberately never learn the outcome, so the
    /// footer must not claim one.
    Sent,
    /// Running off the caller's thread. A second echo follows with the verdict.
    Running,
    Applied,
    Failed,
}

/// One line for the CLI-echo footer.
#[derive(Clone, Serialize)]
pub struct Echo {
    /// The command as a user could retype it, e.g. `tezca bar set height 40`.
    pub line: String,
    pub state: EchoState,
    /// The CLI's own error text, when it failed.
    pub detail: String,
}

/// The window every mutating command reports to.
///
/// A process-wide handle rather than the GTK build's thread-local sink: Tauri
/// runs commands on a pool, so the thread that shells out is rarely the thread
/// that would have installed a sink. `Emitter::emit` is itself thread-safe and
/// marshals to the webview, which is the whole reason the sink existed.
static ECHO_SINK: OnceLock<AppHandle> = OnceLock::new();

/// The event name the footer listens on.
pub const ECHO_EVENT: &str = "tezca://echo";

/// Route every *mutating* command this module runs to the front end — the
/// footer that shows you which `tezca` invocation your click just made.
///
/// Only the action paths report. [`tezca_out`] and [`output`] are pure reads and
/// would otherwise bury the change you actually made under a stream of
/// `display list --machine`.
pub fn set_echo_sink(app: AppHandle) {
    let _ = ECHO_SINK.set(app);
}

fn echo(cmd: &str, args: &[&str], state: EchoState, detail: &str) {
    let Some(app) = ECHO_SINK.get() else { return };
    let _ = app.emit(
        ECHO_EVENT,
        Echo { line: command_line(cmd, args), state, detail: detail.to_string() },
    );
}

/// Commands whose echo is suppressed for the duration of a call.
///
/// The readers a page runs on open are mutating in no sense, but a few of them
/// go through [`capture`] because we need their exit code. Without this the
/// footer would show `display list --machine` every time you changed tab,
/// burying the change you actually made.
static QUIET: Mutex<usize> = Mutex::new(0);

fn quiet() -> bool {
    QUIET.lock().map(|g| *g > 0).unwrap_or(false)
}

/// Run `f` with echo suppressed. Used by the bulk readers.
pub fn without_echo<T>(f: impl FnOnce() -> T) -> T {
    if let Ok(mut g) = QUIET.lock() {
        *g += 1;
    }
    let out = f();
    if let Ok(mut g) = QUIET.lock() {
        *g = g.saturating_sub(1);
    }
    out
}

fn echo_result(cmd: &str, args: &[&str], r: &CmdResult) {
    if quiet() {
        return;
    }
    if r.ok() {
        echo(cmd, args, EchoState::Applied, "");
    } else {
        echo(cmd, args, EchoState::Failed, &r.message());
    }
}

/// `/home/u/.local/bin/tezca display keep` → `tezca display keep`.
fn command_line(cmd: &str, args: &[&str]) -> String {
    let mut s = cmd.rsplit('/').next().unwrap_or(cmd).to_string();
    for a in args {
        s.push(' ');
        // Keep a value containing spaces readable as the one argument it is —
        // `clock_format %a %d %b` would otherwise look like three.
        if a.contains(' ') {
            s.push('"');
            s.push_str(a);
            s.push('"');
        } else {
            s.push_str(a);
        }
    }
    s
}

/// Run `tezca <args>` and capture trimmed stdout (theme names, …).
pub fn tezca_out(args: &[&str]) -> Option<String> {
    output(&tezca_bin(), args)
}

/// Spawn an arbitrary command detached (hyprctl, scripts, wlogout, hyprlock, …).
pub fn spawn(cmd: &str, args: &[&str]) {
    let r = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    // A detached child tells us only whether it *started*. "Sent" is the
    // strongest honest claim; the footer says exactly that.
    match r {
        Ok(_) => echo(cmd, args, EchoState::Sent, ""),
        Err(e) => echo(cmd, args, EchoState::Failed, &e.to_string()),
    }
}

/// Capture trimmed stdout of an arbitrary command (None on failure).
pub fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A path under ~/.config/tezca/…
fn config_tezca(rel: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tezca").join(rel))
}

/// Current wallpaper path from `current/wallpaper`.
pub fn current_wallpaper() -> Option<PathBuf> {
    let s = std::fs::read_to_string(config_tezca("current/wallpaper")?).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

/// One curated theme as `tezca theme info` reports it — enough to draw its card
/// without the panel having to know where the repo's `themes/` directory lives.
#[derive(Clone, Default, Serialize)]
pub struct ThemeInfo {
    pub name: String,
    pub label: String,
    pub description: String,
    pub active: bool,
    pub base: String,
    pub accent: String,
    pub gold: String,
}

pub fn themes() -> Vec<ThemeInfo> {
    let Some(out) = tezca_out(&["theme", "info"]) else { return Vec::new() };
    records(&out)
        .iter()
        .map(|r| ThemeInfo {
            name: rec(r, "name"),
            label: rec(r, "label"),
            description: rec(r, "description"),
            active: rec_bool(r, "active"),
            base: rec(r, "base"),
            accent: rec(r, "accent"),
            gold: rec(r, "gold"),
        })
        .filter(|t| !t.name.is_empty())
        .collect()
}

/// The stored wallpaper fit mode (`fill` · `fit` · `stretch` · `center`).
pub fn wallpaper_fit() -> String {
    tezca_out(&["wallpaper", "fit"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "fill".into())
}

/// Every image the picker can offer, in the CLI's order.
pub fn wallpaper_library() -> Vec<PathBuf> {
    tezca_out(&["wallpaper", "library"])
        .map(|out| {
            out.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
        })
        .unwrap_or_default()
}

/// Whether game mode is on — `game.state` contains "on" when active.
pub fn game_on() -> bool {
    config_tezca("game.state")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim() == "on")
        .unwrap_or(false)
}

/// True when `bin` is runnable, by walking `$PATH` directly.
///
/// Deliberately not `sh -c "command -v $bin"`: that spawns a shell per probe and
/// interpolates its argument into a shell string. (The `tezca` CLI has the same
/// helper in its own `util` module — the two crates share no library, and a
/// six-line function is not worth restructuring the workspace for.)
pub fn has(bin: &str) -> bool {
    if bin.contains('/') {
        return is_executable(&PathBuf::from(bin));
    }
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .any(|d| is_executable(&d.join(bin)))
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Run one of the hypr/scripts/*.sh helpers by name, detached.
pub fn run_script(name: &str, args: &[&str]) {
    let Some(home) = std::env::var_os("HOME") else { return };
    let path = PathBuf::from(home).join(".config/hypr/scripts").join(name);
    if let Some(p) = path.to_str() {
        spawn(p, args);
    }
}

// ---------------------------------------------------------------------------
// Structured helpers for the Displays / Dock / Desktop / Keybinds pages
// ---------------------------------------------------------------------------

/// Result of a `tezca` invocation we need to branch on (e.g. rebind conflicts).
#[derive(Clone, Default, Serialize)]
pub struct CmdResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CmdResult {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// The most useful line to show a user when this failed: the CLI's own error
    /// text, falling back to stdout (hyprctl-style tools answer on stdout) and
    /// finally to a generic message, so an error surface is never blank.
    pub fn message(&self) -> String {
        for s in [&self.stderr, &self.stdout] {
            let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            if !line.is_empty() {
                // The CLI prefixes its failures with a coloured "error:"; the
                // surface already says as much visually.
                return strip_error_prefix(line).to_string();
            }
        }
        format!("command failed (exit {})", self.code)
    }
}

/// Drop a leading `error:` label, with or without its ANSI colour wrapper.
fn strip_error_prefix(line: &str) -> &str {
    let mut s = line.trim_start();
    // ESC [ … m
    while let Some(rest) = s.strip_prefix('\u{1b}') {
        match rest.find('m') {
            Some(i) => s = &rest[i + 1..],
            None => break,
        }
    }
    s.strip_prefix("error:").unwrap_or(s).trim_start()
}

/// Run `tezca <args>` synchronously, returning its exit code + output.
pub fn tezca_result(args: &[&str]) -> CmdResult {
    capture(&tezca_bin(), args)
}

/// Run any command synchronously, capturing everything we might want to report.
pub fn capture(cmd: &str, args: &[&str]) -> CmdResult {
    let r = match Command::new(cmd).args(args).output() {
        Ok(o) => CmdResult {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => CmdResult { code: -1, stdout: String::new(), stderr: e.to_string() },
    };
    // No-op on the `gio` pool, where no sink is installed — `run_async` echoes
    // for those from the main thread instead.
    echo_result(cmd, args, &r);
    r
}

/// Announce a command that is about to run, before it runs.
///
/// The slow tools the connectivity pages drive — `nmcli device wifi list
/// --rescan yes`, `bluetoothctl scan`, `ddcutil` — take long enough that a
/// footer which only spoke on completion would look broken. Callers echo
/// `Running` here, then let [`capture`] echo the verdict.
///
/// In the GTK build this was `run_async`, which had to hop threads by hand
/// because the sink touched widgets. Tauri already runs commands off the UI
/// thread, so the hop is gone and only the announcement remains.
pub fn echo_running(cmd: &str, args: &[&str]) {
    echo(cmd, args, EchoState::Running, "");
}

/// Run `tezca <args>` with `input` on its stdin.
///
/// This is how a Wi-Fi password gets from the dialog to NetworkManager: down a
/// pipe. Passing it as an argument instead would publish it in `/proc/<pid>/cmdline`,
/// which every process on the machine can read — `ps` would print the
/// pre-shared key of the network the user just joined.
pub fn tezca_stdin(args: &[&str], input: &str) -> CmdResult {
    let cmd = tezca_bin();
    echo_running(&cmd, args);
    // `input` is the secret; it goes down the pipe and is never echoed.
    let r = capture_stdin(&cmd, args, input);
    echo_result(&cmd, args, &r);
    r
}

fn capture_stdin(cmd: &str, args: &[&str], input: &str) -> CmdResult {
    use std::io::Write;
    let child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return CmdResult { code: -1, stdout: String::new(), stderr: e.to_string() },
    };
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(input.as_bytes());
        // Dropping the handle closes the pipe, which is what tells the child to
        // stop waiting for more input.
    }
    match child.wait_with_output() {
        Ok(o) => CmdResult {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => CmdResult { code: -1, stdout: String::new(), stderr: e.to_string() },
    }
}

/// Split a `--machine` listing into records.
///
/// The CLI's machine format is a flat stream of `key=value` lines, with a line
/// starting `@` opening a new record — the same shape `display list --machine`
/// has always used. Returns each record as its own key/value list.
pub fn records(out: &str) -> Vec<Vec<(String, String)>> {
    let mut recs: Vec<Vec<(String, String)>> = Vec::new();
    for line in out.lines() {
        if line.starts_with('@') {
            recs.push(Vec::new());
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if let Some(last) = recs.last_mut() {
            last.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    recs
}

/// One field out of a record, empty when absent.
pub fn rec(r: &[(String, String)], key: &str) -> String {
    r.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default()
}

/// One boolean field out of a record.
pub fn rec_bool(r: &[(String, String)], key: &str) -> bool {
    rec(r, key) == "true"
}

/// One connected monitor, from `tezca display list --machine`.
///
/// Mirrors every field the CLI prints. `vrr` and `bitdepth` are the *effective*
/// values read off the compositor — `hyprctl` reports VRR as a bool and bit depth
/// only as a pixel format — so they are evidence of what is live, not of what was
/// configured. The configured value comes from `tezca display config`, which
/// reads the override store; a control seeded from the compositor would flip back
/// to "off" every time VRR happened not to be engaged.
#[derive(Clone, Default, Serialize)]
pub struct Monitor {
    pub name: String,
    pub desc: String,
    pub res: String,
    pub rate: String,
    pub pos: String,
    pub scale: String,
    pub transform: String,
    pub vrr: String,
    pub bitdepth: String,
    pub mirror: String,
    pub disabled: bool,
    pub modes: Vec<String>, // "3440x1440@165.00"
}

pub fn monitors() -> Vec<Monitor> {
    let Some(out) = tezca_out(&["display", "list", "--machine", "--all"]) else {
        return Vec::new();
    };
    parse_monitors(&out)
}

fn parse_monitors(out: &str) -> Vec<Monitor> {
    let mut mons: Vec<Monitor> = Vec::new();
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@monitor ") {
            mons.push(Monitor { name: name.trim().to_string(), ..Default::default() });
            continue;
        }
        let Some(m) = mons.last_mut() else { continue };
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "desc" => m.desc = v.to_string(),
            "res" => m.res = v.to_string(),
            "rate" => m.rate = v.to_string(),
            "pos" => m.pos = v.to_string(),
            "scale" => m.scale = v.to_string(),
            "transform" => m.transform = v.to_string(),
            "vrr" => m.vrr = v.to_string(),
            "bitdepth" => m.bitdepth = v.to_string(),
            "mirror" => m.mirror = v.to_string(),
            "disabled" => m.disabled = v == "true",
            "modes" => m.modes = v.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    mons
}

/// The persisted per-monitor overrides (`tezca display config`) as
/// `monitor:<NAME>.<field>` → value. This is the source of truth for controls
/// whose configured value cannot be read back off the compositor (VRR mode, bit
/// depth); absent means "never set", i.e. inherit the shipped config.
pub fn display_config() -> Vec<(String, String)> {
    config_pairs(&["display", "config"])
}

/// Look one `display_config` key up: `override_for(&cfg, "DP-1", "vrr")`.
pub fn override_for(cfg: &[(String, String)], monitor: &str, field: &str) -> Option<String> {
    let key = format!("monitor:{monitor}.{field}");
    cfg.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()).filter(|v| !v.is_empty())
}

/// Per-monitor wallpaper targets: (name, is_override, path).
pub fn wallpaper_targets() -> Vec<(String, bool, String)> {
    let Some(out) = tezca_out(&["wallpaper", "list"]) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let name = f.next()?.trim().to_string();
            let source = f.next()?.trim();
            let path = f.next().unwrap_or("").trim().to_string();
            Some((name, source == "override", path))
        })
        .collect()
}

/// DDC/CI brightness (0-100) for a monitor, or None if not DDC-capable.
pub fn brightness(name: &str) -> Option<i32> {
    tezca_out(&["display", "brightness", name])?.trim().parse().ok()
}

/// The effective dock config as key→value strings (`tezca dock config`).
pub fn dock_config() -> Vec<(String, String)> {
    config_pairs(&["dock", "config"])
}

/// The effective bar config as key→value strings (`tezca bar config`).
pub fn bar_config() -> Vec<(String, String)> {
    config_pairs(&["bar", "config"])
}

/// Discovered custom bar modules: (layout id `custom:<name>`, display label).
/// Scans `~/.config/tezca-bar/modules/*.toml` — the same drop-in directory the
/// bar reads — so a manifest dropped in there shows up in the Modules editor.
pub fn custom_bar_modules() -> Vec<(String, String)> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    let Some(dir) = base.map(|b| b.join("tezca-bar").join("modules")) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        // A manifest with no `exec` is a no-op module; skip it here too.
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let has_exec = text.lines().any(|l| {
            let l = l.trim();
            l.split_once('=').map(|(k, _)| matches!(k.trim(), "exec" | "command")).unwrap_or(false)
        });
        if !has_exec {
            continue;
        }
        let label = text
            .lines()
            .find_map(|l| {
                let (k, v) = l.trim().split_once('=')?;
                matches!(k.trim(), "label" | "name")
                    .then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| title_case(stem));
        out.push((format!("custom:{stem}"), label));
    }
    out.sort();
    out
}

/// `weather-city` → `Weather city`.
fn title_case(s: &str) -> String {
    let s = s.replace(['-', '_'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s.clone(),
    }
}

/// Shared `key = value` parse for the `tezca <x> config` commands.
fn config_pairs(args: &[&str]) -> Vec<(String, String)> {
    let Some(out) = tezca_out(args) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// The current value of a Hyprland option (`tezca hypr get`).
///
/// `[[EMPTY]]` is filtered here as well as in the CLI: this is what populates
/// text entries, and a sentinel reaching one of those is written back verbatim by
/// the next Apply. Belt and braces for the case where an older `tezca` binary is
/// on PATH than the settings binary — which is exactly what a partial install
/// looks like.
pub fn hypr_get(opt: &str) -> Option<String> {
    tezca_out(&["hypr", "get", opt]).filter(|v| v != "[[EMPTY]]")
}

// ---------------------------------------------------------------------------
// Session identity — the sidebar footer card
// ---------------------------------------------------------------------------

/// What this machine is, in one card: `("quetzalcoatl", "Hyprland 0.51.1 · 3 displays")`.
///
/// Every field is best-effort and degrades to something true rather than to a
/// placeholder: no compositor answer means the version is simply left out, not
/// guessed at.
pub fn session_summary() -> (String, String) {
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "this machine".to_string());

    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = hyprland_version() {
        parts.push(format!("Hyprland {v}"));
    }
    let n = monitors().len();
    if n > 0 {
        parts.push(format!("{n} display{}", if n == 1 { "" } else { "s" }));
    }
    (host, parts.join(" · "))
}

/// The compositor's version string, e.g. `0.51.1`.
///
/// `hyprctl version` prints a multi-line banner whose exact shape has changed
/// across releases; the one stable thing in it is a `vMAJOR.MINOR…` tag, so we
/// look for that rather than for a fixed line or field position.
fn hyprland_version() -> Option<String> {
    let out = output("hyprctl", &["version"])?;
    out.split(|c: char| c.is_whitespace() || c == ',')
        .find_map(|tok| {
            let v = tok.trim_start_matches('v');
            (v != tok && v.starts_with(|c: char| c.is_ascii_digit()))
                .then(|| v.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.').to_string())
        })
        .filter(|v| !v.is_empty())
}
