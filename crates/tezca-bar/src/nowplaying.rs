//! Now-playing — MPRIS via `playerctl` shell-out.
//!
//! The prototype centres a media pill (art, title, artist, live equaliser). We
//! source it from `playerctl` (the ubiquitous MPRIS CLI) rather than pulling a
//! D-Bus dependency into the bar, matching the crate's shell-out philosophy.
//! Absent player → `None`, and the pill is hidden.

use std::process::Command;

pub struct NowPlaying {
    pub title: String,
    pub artist: String,
}

/// Current track, or None if no MPRIS player is present.
pub fn current() -> Option<NowPlaying> {
    // One call, unit-separated so titles/artists with spaces survive.
    let out = Command::new("playerctl")
        .args(["metadata", "--format", "{{status}}\x1f{{title}}\x1f{{artist}}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.split('\x1f');
    let _status = parts.next().unwrap_or("");
    let title = parts.next().unwrap_or("").trim().to_string();
    let artist = parts.next().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return None;
    }
    Some(NowPlaying { title, artist })
}

/// Toggle play/pause on the active player.
pub fn play_pause() {
    let _ = Command::new("playerctl").arg("play-pause").status();
}

/// Nudge the position by `secs` (negative rewinds).
pub fn seek(secs: i32) {
    let arg = if secs >= 0 {
        format!("{secs}+")
    } else {
        format!("{}-", -secs)
    };
    let _ = Command::new("playerctl").args(["position", &arg]).status();
}
