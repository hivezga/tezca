//! Notifications — swaync bell state, polled via `swaync-client`.
//!
//! Reaches parity with the Waybar `custom/notification` module: unread count for
//! the pulsing badge, DND state, and the same click actions (toggle the control
//! centre; right-click toggles do-not-disturb). Polled on a slow interval — the
//! count only drives a small badge, so a couple of seconds' latency is invisible.

use std::process::Command;

pub struct BellState {
    pub unread: u32,
}

pub fn state() -> BellState {
    BellState { unread: count() }
}

/// Unread notification count (`swaync-client -c`), 0 if swaync isn't running.
fn count() -> u32 {
    swaync(&["-c"]).and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// Toggle the notification control centre panel.
pub fn toggle_panel() {
    let _ = Command::new("swaync-client").args(["-t", "-sw"]).status();
}

/// Toggle do-not-disturb.
pub fn toggle_dnd() {
    let _ = Command::new("swaync-client").args(["-d", "-sw"]).status();
}

fn swaync(args: &[&str]) -> Option<String> {
    let out = Command::new("swaync-client").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
