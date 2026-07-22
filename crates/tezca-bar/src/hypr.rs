//! Hyprland IPC — workspaces, focused window, and submap.
//!
//! State snapshots go through `hyprctl -j` (robust, versioned by Hyprland);
//! live updates come from the event stream on `.socket2.sock`, read on a
//! background thread that pings an async-channel so the GTK main loop refreshes.
//! Mirrors the approach in `crates/tezca-dock/src/hypr.rs`, extended with
//! workspace + active-window + submap state the bar needs.

use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;

/// One Hyprland workspace, trimmed to what the bar draws.
#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    #[serde(default)]
    pub id: i32,
    #[serde(default)]
    pub monitor: String,
    #[serde(default)]
    pub windows: i32,
}

/// One monitor, for mapping active workspace → output and the focused flag.
#[derive(Debug, Clone, Deserialize)]
pub struct Monitor {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "activeWorkspace", default)]
    pub active_workspace: WsRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WsRef {
    #[serde(default)]
    pub id: i32,
}

/// The focused window's app class + title.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Active {
    #[serde(default)]
    pub class: String,
}

/// A full pull of the live state the bar renders.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub monitors: Vec<Monitor>,
    pub active: Active,
}

/// A change worth reacting to, delivered from the event-socket reader.
#[derive(Debug, Clone)]
pub enum Event {
    /// Refresh the workspace / active-window model.
    Refresh,
    /// The submap changed; empty string means back to the default submap.
    Submap(String),
}

/// Pull the current workspaces, monitors, and focused window in one shot.
pub fn snapshot() -> Snapshot {
    Snapshot {
        workspaces: query("workspaces"),
        monitors: query("monitors"),
        active: query_one("activewindow").unwrap_or_default(),
    }
}

/// The active workspace id on a given output (falls back to the first monitor).
pub fn active_ws_for(monitors: &[Monitor], output: &str) -> i32 {
    monitors
        .iter()
        .find(|m| m.name == output)
        .or_else(|| monitors.first())
        .map(|m| m.active_workspace.id)
        .unwrap_or(1)
}

/// Switch to workspace `id`.
pub fn goto_workspace(id: i32) {
    let _ = Command::new("hyprctl")
        .args(["dispatch", "workspace", &id.to_string()])
        .status();
}

fn query<T: for<'de> Deserialize<'de>>(what: &str) -> Vec<T> {
    let Ok(out) = Command::new("hyprctl").args(["-j", what]).output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    serde_json::from_slice(&out.stdout).unwrap_or_default()
}

fn query_one<T: for<'de> Deserialize<'de>>(what: &str) -> Option<T> {
    let out = Command::new("hyprctl").args(["-j", what]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Spawn a reader on the Hyprland event socket, translating raw events into
/// [`Event`]s down `tx`. Returns immediately; the thread lives for the process.
pub fn subscribe(tx: async_channel::Sender<Event>) {
    let Some(path) = socket2_path() else {
        eprintln!("tezca-bar: no Hyprland event socket — live updates disabled");
        return;
    };
    std::thread::spawn(move || {
        let Ok(stream) = UnixStream::connect(&path) else {
            eprintln!("tezca-bar: cannot connect {}", path.display());
            return;
        };
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let (name, data) = line.split_once(">>").unwrap_or((line.as_str(), ""));
            let ev = match name {
                // Submap carries its name (empty on exit).
                "submap" => Some(Event::Submap(data.to_string())),
                // Anything that changes the workspace set or focus → refresh.
                "workspace" | "workspacev2" | "createworkspace" | "createworkspacev2"
                | "destroyworkspace" | "destroyworkspacev2" | "focusedmon" | "openwindow"
                | "closewindow" | "movewindow" | "movewindowv2" | "activewindow"
                | "activewindowv2" | "windowtitle" | "windowtitlev2" | "fullscreen"
                | "urgent" => Some(Event::Refresh),
                _ => None,
            };
            if let Some(ev) = ev {
                if tx.send_blocking(ev).is_err() {
                    break; // receiver gone → main loop shut down
                }
            }
        }
    });
}

fn socket2_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let his = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(PathBuf::from(runtime).join("hypr").join(his).join(".socket2.sock"))
}
