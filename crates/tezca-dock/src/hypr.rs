//! Hyprland IPC — the running-app set and focus control.
//!
//! Queries go through `hyprctl -j` (robust, versioned by Hyprland itself);
//! live updates come from the event stream on `.socket2.sock`, read on a
//! background thread that pings an async-channel so the GTK main loop rebuilds
//! the dock model. See DESIGN.md §9.

use serde::Deserialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;

/// One Hyprland client (window), trimmed to what the dock needs.
#[derive(Debug, Clone, Deserialize)]
pub struct Client {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub mapped: bool,
}

/// All mapped, real windows (drops the scratch terminal + unmapped clients).
pub fn clients() -> Vec<Client> {
    let out = Command::new("hyprctl").args(["-j", "clients"]).output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let parsed: Vec<Client> = serde_json::from_slice(&out.stdout).unwrap_or_default();
    parsed
        .into_iter()
        .filter(|c| c.mapped && !c.class.is_empty() && c.class != "tezca-scratch")
        .collect()
}

/// Address (`0x…`) of the currently focused window, if any.
pub fn active_address() -> Option<String> {
    let out = Command::new("hyprctl").args(["-j", "activewindow"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    #[derive(Deserialize)]
    struct Active {
        #[serde(default)]
        address: String,
    }
    let a: Active = serde_json::from_slice(&out.stdout).ok()?;
    (!a.address.is_empty()).then_some(a.address)
}

/// Global cursor position in layout coords, via the Hyprland request socket
/// (cheaper than spawning `hyprctl` at poll rate). Returns None if unavailable.
pub fn cursor_pos() -> Option<(i32, i32)> {
    let path = socket1_path()?;
    let mut stream = UnixStream::connect(&path).ok()?;
    stream.write_all(b"cursorpos").ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    // Response looks like "1720, 700".
    let (x, y) = buf.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Focus (and bring to the active workspace) the window at `address`.
pub fn focus(address: &str) {
    let _ = Command::new("hyprctl")
        .args(["dispatch", "focuswindow", &format!("address:{address}")])
        .status();
}

/// Spawn a reader on the Hyprland event socket. Sends `()` down `tx` whenever a
/// window-lifecycle or focus event lands, so the caller can refresh the model.
/// Returns immediately; the thread lives for the process.
pub fn subscribe(tx: async_channel::Sender<()>) {
    let Some(path) = socket2_path() else {
        eprintln!("tezca-dock: no Hyprland event socket — live updates disabled");
        return;
    };
    std::thread::spawn(move || {
        let Ok(stream) = UnixStream::connect(&path) else {
            eprintln!("tezca-dock: cannot connect {}", path.display());
            return;
        };
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            // Events are `NAME>>DATA`. We rebuild on anything that changes the
            // window set or which app is focused.
            let name = line.split(">>").next().unwrap_or("");
            let relevant = matches!(
                name,
                "openwindow"
                    | "closewindow"
                    | "movewindow"
                    | "movewindowv2"
                    | "activewindow"
                    | "activewindowv2"
                    | "windowtitle"
                    | "windowtitlev2"
                    | "changefloatingmode"
                    | "fullscreen"
                    | "workspace"
                    | "focusedmon"
            );
            if relevant && tx.send_blocking(()).is_err() {
                break; // receiver gone → main loop shut down
            }
        }
    });
}

fn socket2_path() -> Option<PathBuf> {
    hypr_dir().map(|d| d.join(".socket2.sock"))
}

/// The request socket (`.socket.sock`) — for one-shot commands like cursorpos.
fn socket1_path() -> Option<PathBuf> {
    hypr_dir().map(|d| d.join(".socket.sock"))
}

fn hypr_dir() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let his = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(PathBuf::from(runtime).join("hypr").join(his))
}
