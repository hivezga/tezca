//! Microphone-in-use detection — the audio-capture privacy indicator, the
//! companion to the camera one (see `camera.rs`).
//!
//! Unlike a webcam (a `/dev/video*` fd an app holds directly), microphone
//! capture on a PipeWire/PulseAudio system doesn't map to a per-app device fd:
//! PipeWire owns the ALSA device and apps open *recording streams* against it.
//! So we ask the sound server instead — `pactl -f json list source-outputs`
//! lists every capture stream. We report one as "the mic is live" when it is:
//!   * not `corked` (a corked stream is paused/primed, not actually pulling
//!     audio), and
//!   * reading from a real source, NOT a `.monitor` (a monitor source is the
//!     loopback of an output — capturing *desktop audio*, e.g. a screen
//!     recording, which is not the microphone).
//!
//! Best-effort, like the rest of the shell-out readers: no `pactl`, malformed
//! JSON, or a server hiccup all read as "mic idle" rather than erroring.

use std::collections::BTreeSet;
use std::process::Command;

use serde_json::Value;

/// Snapshot of microphone usage at one poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MicUse {
    /// True when at least one live (non-corked) stream is reading a real mic.
    pub active: bool,
    /// Friendly names of the apps recording, sorted + de-duplicated.
    pub apps: Vec<String>,
}

impl MicUse {
    /// A human sentence for the tooltip, e.g. "Microphone in use by Zoom".
    pub fn tooltip(&self) -> String {
        if !self.active {
            return "Microphone idle".to_string();
        }
        if self.apps.is_empty() {
            return "Microphone in use".to_string();
        }
        format!("Microphone in use by {}", self.apps.join(", "))
    }
}

/// Ask PipeWire/PulseAudio which apps (if any) are actively recording the mic.
pub fn poll() -> MicUse {
    let monitors = monitor_source_indices();
    let outputs = match pactl_json(&["list", "source-outputs"]) {
        Some(Value::Array(a)) => a,
        _ => return MicUse::default(), // no pactl / bad json → treat as idle
    };

    let mut apps: BTreeSet<String> = BTreeSet::new();
    let mut active = false;

    for so in &outputs {
        // A paused/primed stream isn't pulling audio — ignore it.
        if so.get("corked").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        // Skip monitor captures (desktop audio / screen-recording), keeping only
        // real microphone sources.
        if let Some(src) = so.get("source").and_then(Value::as_u64) {
            if monitors.contains(&src) {
                continue;
            }
        }
        active = true;
        if let Some(app) = app_name(so) {
            apps.insert(app);
        }
    }

    MicUse { active, apps: apps.into_iter().collect() }
}

/// Indices of sources that are monitors (`…​.monitor`) — captures against these
/// are desktop audio, not the mic, so they're excluded.
fn monitor_source_indices() -> BTreeSet<u64> {
    let mut set = BTreeSet::new();
    let Some(Value::Array(sources)) = pactl_json(&["list", "sources"]) else {
        return set;
    };
    for s in &sources {
        let is_monitor = s
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.ends_with(".monitor"))
            // PipeWire also carries the sink it monitors, when set.
            || s.get("monitor_of_sink").map(|v| !v.is_null()).unwrap_or(false)
                && s.get("monitor_of_sink").and_then(Value::as_str) != Some("n/a");
        if is_monitor {
            if let Some(idx) = s.get("index").and_then(Value::as_u64) {
                set.insert(idx);
            }
        }
    }
    set
}

/// A display name for a source-output: its `application.name`, falling back to
/// the process binary or node name, title-cased ("zoom" → "Zoom").
fn app_name(so: &Value) -> Option<String> {
    let props = so.get("properties")?;
    let pick = |key: &str| props.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
    let raw = pick("application.name")
        .or_else(|| pick("application.process.binary"))
        .or_else(|| pick("node.name"))?;
    Some(title_case(raw))
}

/// Run `pactl -f json <args>` and parse stdout. None on any failure.
fn pactl_json(args: &[&str]) -> Option<Value> {
    let out = Command::new("pactl").arg("-f").arg("json").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Capitalise only the first character, leaving internal capitals alone.
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_wording() {
        assert_eq!(MicUse::default().tooltip(), "Microphone idle");
        let live = MicUse { active: true, apps: vec!["Zoom".into()] };
        assert_eq!(live.tooltip(), "Microphone in use by Zoom");
        let anon = MicUse { active: true, apps: vec![] };
        assert_eq!(anon.tooltip(), "Microphone in use");
    }

    #[test]
    fn title_case_first_char_only() {
        assert_eq!(title_case("parecord"), "Parecord");
        assert_eq!(title_case("WebRTC VoiceEngine"), "WebRTC VoiceEngine");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn poll_does_not_panic() {
        let _ = poll();
    }
}
