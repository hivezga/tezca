//! Camera-in-use detection — the privacy indicator in the bar's right cluster.
//!
//! A webcam is a `/dev/video*` character device; an application that is
//! capturing holds an open file descriptor to it. So "is the camera live?" is
//! answered without any daemon or D-Bus by walking `/proc/<pid>/fd/*` and
//! looking for a symlink that resolves to `/dev/video*`. We only ever see the
//! user's own processes (the camera app runs as the user), which is exactly the
//! set we want to report — no root, no shell-out, pure `/proc` reads like the
//! rest of `sysinfo`.
//!
//! Nodes are deduplicated by owning PID, so a device that exposes both a capture
//! node and a metadata node (the Logitech C920 shows up as `video0`+`video1`)
//! is reported once per app, not twice.

use std::collections::BTreeSet;
use std::fs;

/// Snapshot of camera usage at one poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraUse {
    /// True when at least one process holds a `/dev/video*` device open.
    pub active: bool,
    /// Friendly names of the apps holding the camera, sorted + de-duplicated.
    pub apps: Vec<String>,
}

impl CameraUse {
    /// A human sentence for the tooltip, e.g. "Camera in use by Brave, Zoom".
    pub fn tooltip(&self) -> String {
        if !self.active {
            return "Camera idle".to_string();
        }
        if self.apps.is_empty() {
            // Held open, but we couldn't name the holder (its /proc entry raced
            // away or wasn't readable) — still say the camera is live.
            return "Camera in use".to_string();
        }
        format!("Camera in use by {}", self.apps.join(", "))
    }
}

/// Scan `/proc` for any process holding a `/dev/video*` device open.
///
/// Best-effort throughout: a PID that exits mid-scan, or an `fd` dir we can't
/// read, is simply skipped — never an error. Returns the distinct owning app
/// names (from `/proc/<pid>/comm`) so the same app opening several video nodes
/// counts once.
pub fn poll() -> CameraUse {
    let Ok(entries) = fs::read_dir("/proc") else {
        return CameraUse::default();
    };

    let mut apps: BTreeSet<String> = BTreeSet::new();
    let mut active = false;

    for ent in entries.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        // Only numeric PID dirs.
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }

        if !holds_video(&ent.path()) {
            continue;
        }
        active = true;
        if let Some(app) = app_name(name) {
            apps.insert(app);
        }
    }

    CameraUse { active, apps: apps.into_iter().collect() }
}

/// Does this `/proc/<pid>` have any fd pointing at a `/dev/video*` device?
fn holds_video(proc_pid: &std::path::Path) -> bool {
    let Ok(fds) = fs::read_dir(proc_pid.join("fd")) else {
        return false; // not ours / gone / unreadable
    };
    for fd in fds.flatten() {
        if let Ok(target) = fs::read_link(fd.path()) {
            if target.to_str().is_some_and(|t| t.starts_with("/dev/video")) {
                return true;
            }
        }
    }
    false
}

/// A display name for a PID: its `comm` (the executable's short name), title-cased
/// so "brave" reads as "Brave". `comm` is truncated to 15 bytes by the kernel,
/// which is fine for a tooltip.
fn app_name(pid: &str) -> Option<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let comm = comm.trim();
    if comm.is_empty() {
        return None;
    }
    Some(title_case(comm))
}

/// "brave" → "Brave", "obs" → "Obs", "WebCam" → "WebCam" (only touches the first
/// letter, leaves internal capitals alone).
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
    fn title_case_capitalises_first_only() {
        assert_eq!(title_case("brave"), "Brave");
        assert_eq!(title_case("obs"), "Obs");
        assert_eq!(title_case("WebCam"), "WebCam");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn tooltip_wording() {
        let idle = CameraUse::default();
        assert_eq!(idle.tooltip(), "Camera idle");

        let live = CameraUse { active: true, apps: vec!["Brave".into(), "Zoom".into()] };
        assert_eq!(live.tooltip(), "Camera in use by Brave, Zoom");

        let anon = CameraUse { active: true, apps: vec![] };
        assert_eq!(anon.tooltip(), "Camera in use");
    }

    #[test]
    fn poll_does_not_panic() {
        // Whatever the machine state, this must return cleanly (idle on CI).
        let _ = poll();
    }
}
