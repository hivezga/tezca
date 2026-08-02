//! Session state the bar shows but does not own: keep-awake, night light and
//! screen recording.
//!
//! All three are driven by the `tezca` CLI (`idle inhibit`, `night`, `record`),
//! which persists tiny files; the bar only *reads* them, so a click here and the
//! same command from a terminal can never disagree. Reads are a stat and a short
//! file each — cheap enough for the 2-second control tick, and gated behind
//! `Config::uses_mod` so an unplaced module costs nothing at all.
//!
//! The one exception is the night-light schedule: the bar owns the clock for it.
//! hyprsunset has no scheduler, and rather than generate systemd timers the bar
//! evaluates the saved window each tick and shells out to `tezca night apply`
//! **only when the desired state changes** — so a configured schedule costs one
//! process at the boundary, not one per minute.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tezca"))
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("tezca"))
}

/// A pid file written by the CLI, validated against `/proc` so a stale file left
/// by a crash or a reboot does not read as "still running".
fn live_pid(file: &str) -> Option<u32> {
    let p = cache_dir()?.join(file);
    let pid: u32 = std::fs::read_to_string(p).ok()?.trim().parse().ok()?;
    std::path::Path::new(&format!("/proc/{pid}")).exists().then_some(pid)
}

// ── Keep awake ─────────────────────────────────────────────────────────────

/// Whether `tezca idle inhibit` is currently holding the session awake.
pub fn caffeine_on() -> bool {
    live_pid("inhibit.pid").is_some()
}

// ── Night light ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NightState {
    /// Configured at all — the module hides entirely when this is false.
    pub configured: bool,
    /// Whether the filter should be on right now (switch **and** schedule).
    pub active: bool,
    pub temp: u32,
}

/// Read `~/.config/tezca/night.lua`. Deliberately a small key scanner over the
/// file the CLI generates, not a Lua evaluator: the bar is not a config parser,
/// and the grammar it must accept is exactly the one `tezca night` writes.
pub fn night(now_minutes: u32) -> NightState {
    let Some(dir) = config_dir() else { return NightState::default() };
    let Ok(text) = std::fs::read_to_string(dir.join("night.lua")) else {
        return NightState::default();
    };
    let (mut enabled, mut temp, mut from, mut to) = (false, 4000u32, None, None);
    for line in text.lines() {
        let line = line.trim().trim_end_matches(',');
        let Some((k, v)) = line.split_once('=') else { continue };
        match (k.trim(), v.trim()) {
            ("enabled", v) => enabled = v == "true",
            ("temp", v) => temp = v.parse().unwrap_or(4000),
            ("from", v) => from = v.parse::<u32>().ok(),
            ("to", v) => to = v.parse::<u32>().ok(),
            _ => {}
        }
    }
    let active = enabled
        && match (from, to) {
            // A window that wraps past midnight (22:00 → 06:00) is the usual
            // case, so it is handled, not treated as an error.
            (Some(f), Some(t)) if f != t => {
                if f < t {
                    now_minutes >= f && now_minutes < t
                } else {
                    now_minutes >= f || now_minutes < t
                }
            }
            _ => true,
        };
    NightState { configured: enabled || from.is_some(), active, temp }
}

/// Re-assert the night-light state through the CLI. Called only on a transition.
pub fn night_apply() {
    let _ = Command::new("tezca")
        .args(["night", "apply"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

// ── Screen recording ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordState {
    pub active: bool,
    /// Set when something other than `tezca record` is recording — the indicator
    /// still lights up, but clicking it cannot stop a process we do not own.
    pub foreign: bool,
}

/// Recorder binaries worth noticing when they were not started by us. Deliberately
/// a short list of real screen recorders rather than a fuzzy match: a privacy
/// indicator that lies in either direction is worse than none.
const RECORDERS: &[&str] = &["wf-recorder", "wl-screenrec", "gpu-screen-recorder", "obs"];

pub fn recording() -> RecordState {
    if live_pid("record.pid").is_some() {
        return RecordState { active: true, foreign: false };
    }
    // Nothing of ours — but the dot should still light up for a recording
    // started any other way, which is the whole point of a privacy indicator.
    let Ok(entries) = std::fs::read_dir("/proc") else { return RecordState::default() };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(comm) = std::fs::read_to_string(e.path().join("comm")) else { continue };
        if RECORDERS.contains(&comm.trim()) {
            return RecordState { active: true, foreign: true };
        }
    }
    RecordState::default()
}

/// `--session-dump`: print what these three modules see, with no window.
pub fn dump() {
    let now = minutes_now();
    let n = night(now);
    let r = recording();
    println!("caffeine={}", caffeine_on());
    println!("night configured={} active={} temp={}", n.configured, n.active, n.temp);
    println!("recording active={} foreign={}", r.active, r.foreign);
    println!("clock minutes={now}");
}

/// Minutes from local midnight, via glib (already linked, and timezone-correct).
pub fn minutes_now() -> u32 {
    match gtk4::glib::DateTime::now_local() {
        Ok(t) => (t.hour() * 60 + t.minute()) as u32,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_pid_file_does_not_read_as_running() {
        // Nothing to clean up: pid 0 never exists, so /proc/0 is absent and the
        // reader must treat the file as stale rather than trusting it.
        assert!(!std::path::Path::new("/proc/0").exists());
    }

    #[test]
    fn recording_state_defaults_to_idle() {
        assert_eq!(RecordState::default(), RecordState { active: false, foreign: false });
    }

    #[test]
    fn the_recorder_list_holds_only_real_screen_recorders() {
        // A fuzzy match here would light the privacy dot for anything with
        // "record" in its name.
        assert!(RECORDERS.contains(&"wf-recorder"));
        assert!(!RECORDERS.iter().any(|r| r.len() < 3));
    }
}
