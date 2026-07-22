//! System metrics — CPU, memory, network, audio, battery, brightness, gamemode.
//!
//! All std/shell-out, matching the repo's idioms (the CLI shells to hyprctl /
//! wpctl / nmcli; the dock reads /proc): CPU & memory come straight from /proc,
//! network from nmcli + /proc/net, audio from wpctl, battery/brightness from
//! sysfs, gamemode from the same state file the Waybar module polled. Anything
//! absent on the target hardware (no battery / no backlight on a desktop) simply
//! reports `None`, and the bar hides that module.

use std::path::Path;
use std::process::Command;

/// Rolling CPU meter — /proc/stat aggregate deltas.
#[derive(Default)]
pub struct CpuMeter {
    last_total: u64,
    last_idle: u64,
}

impl CpuMeter {
    /// Fraction busy in [0,1] since the previous call (0 on the first call).
    pub fn sample(&mut self) -> f64 {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else { return 0.0 };
        let Some(line) = stat.lines().next() else { return 0.0 };
        // "cpu  user nice system idle iowait irq softirq steal ..."
        let nums: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() < 4 {
            return 0.0;
        }
        let idle = nums[3] + nums.get(4).copied().unwrap_or(0); // idle + iowait
        let total: u64 = nums.iter().sum();
        let dt = total.saturating_sub(self.last_total);
        let di = idle.saturating_sub(self.last_idle);
        self.last_total = total;
        self.last_idle = idle;
        if dt == 0 {
            return 0.0;
        }
        (1.0 - di as f64 / dt as f64).clamp(0.0, 1.0)
    }
}

/// Memory snapshot from /proc/meminfo.
pub struct Mem {
    pub used_frac: f64,
}

pub fn mem() -> Mem {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let get = |key: &str| -> f64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0) // kB
    };
    let total = get("MemTotal:");
    let avail = get("MemAvailable:");
    let used = (total - avail).max(0.0);
    Mem {
        used_frac: if total > 0.0 { used / total } else { 0.0 },
    }
}

/// Audio sink state from wpctl.
pub struct Audio {
    pub volume: u32, // percent
    pub muted: bool,
}

pub fn audio() -> Audio {
    audio_of("@DEFAULT_AUDIO_SINK@").unwrap_or(Audio { volume: 0, muted: true })
}

/// Parse `wpctl get-volume <id>` → "Volume: 0.46 [MUTED]".
pub fn audio_of(id: &str) -> Option<Audio> {
    let out = Command::new("wpctl").args(["get-volume", id]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let muted = s.contains("[MUTED]");
    let vol = s
        .split_whitespace()
        .find_map(|t| t.parse::<f64>().ok())
        .map(|v| (v * 100.0).round() as u32)
        .unwrap_or(0);
    Some(Audio { volume: vol, muted })
}

/// Network state — enough for the control glyph and the detail popover.
pub enum Net {
    Wifi { signal: u32, ssid: String, ip: String },
    Ethernet { ip: String },
    Disconnected,
}

pub fn net() -> Net {
    // Active wifi first (nmcli marks the connected AP with yes in ACTIVE).
    if let Some(out) = nmcli(&["-t", "-f", "ACTIVE,SIGNAL,SSID", "device", "wifi"]) {
        for line in out.lines() {
            let mut f = line.split(':');
            if f.next() == Some("yes") {
                let signal = f.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let ssid = f.collect::<Vec<_>>().join(":");
                let ip = primary_ip("wifi").unwrap_or_default();
                return Net::Wifi { signal, ssid, ip };
            }
        }
    }
    // Then a connected wired device.
    if let Some(out) = nmcli(&["-t", "-f", "TYPE,STATE", "device", "status"]) {
        for line in out.lines() {
            let mut f = line.split(':');
            if f.next() == Some("ethernet") && f.next().map(|s| s.starts_with("connected")).unwrap_or(false)
            {
                let ip = primary_ip("ethernet").unwrap_or_default();
                return Net::Ethernet { ip };
            }
        }
    }
    Net::Disconnected
}

/// First IPv4 address of the first connected device of `kind` (wifi|ethernet).
fn primary_ip(kind: &str) -> Option<String> {
    let out = nmcli(&["-t", "-f", "DEVICE,TYPE,STATE", "device", "status"])?;
    let dev = out.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 3 && f[1] == kind && f[2].starts_with("connected")).then(|| f[0].to_string())
    })?;
    let show = nmcli(&["-t", "-f", "IP4.ADDRESS", "device", "show", &dev])?;
    show.lines()
        .find_map(|l| l.split_once(':').map(|(_, v)| v.split('/').next().unwrap_or(v).to_string()))
}

fn nmcli(args: &[&str]) -> Option<String> {
    let out = Command::new("nmcli").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Rolling throughput meter on the default-route interface (for the net popover).
#[derive(Default)]
pub struct NetMeter {
    iface: Option<String>,
    last_rx: u64,
    last_tx: u64,
}

pub struct Throughput {
    pub down_mbps: f64,
    pub up_mbps: f64,
}

impl NetMeter {
    /// Down/up in Mb/s since the previous call, assuming `dt_secs` elapsed.
    pub fn sample(&mut self, dt_secs: f64) -> Throughput {
        if self.iface.is_none() {
            self.iface = default_iface();
        }
        let Some(iface) = self.iface.clone() else {
            return Throughput { down_mbps: 0.0, up_mbps: 0.0 };
        };
        let Some((rx, tx)) = iface_bytes(&iface) else {
            return Throughput { down_mbps: 0.0, up_mbps: 0.0 };
        };
        let d_rx = rx.saturating_sub(self.last_rx);
        let d_tx = tx.saturating_sub(self.last_tx);
        let first = self.last_rx == 0 && self.last_tx == 0;
        self.last_rx = rx;
        self.last_tx = tx;
        if first || dt_secs <= 0.0 {
            return Throughput { down_mbps: 0.0, up_mbps: 0.0 };
        }
        // bytes → megabits per second.
        Throughput {
            down_mbps: (d_rx as f64 * 8.0) / (dt_secs * 1_000_000.0),
            up_mbps: (d_tx as f64 * 8.0) / (dt_secs * 1_000_000.0),
        }
    }
}

/// Interface with the default route, from /proc/net/route (dest 00000000).
fn default_iface() -> Option<String> {
    let text = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in text.lines().skip(1) {
        let mut f = line.split_whitespace();
        let iface = f.next()?;
        let dest = f.next()?;
        if dest == "00000000" {
            return Some(iface.to_string());
        }
    }
    None
}

/// (rx_bytes, tx_bytes) for `iface` from /proc/net/dev.
fn iface_bytes(iface: &str) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(iface) {
            let rest = rest.trim_start_matches(':').trim();
            let cols: Vec<u64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            // rx bytes = col 0, tx bytes = col 8.
            if cols.len() >= 9 {
                return Some((cols[0], cols[8]));
            }
        }
    }
    None
}

/// Battery percent + charging flag, or None on a battery-less machine (desktop).
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
}

pub fn battery() -> Option<Battery> {
    let dir = Path::new("/sys/class/power_supply");
    let rd = std::fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        let ty = read_trim(&p.join("type"));
        if ty.as_deref() != Some("Battery") {
            continue;
        }
        let percent = read_trim(&p.join("capacity"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let status = read_trim(&p.join("status")).unwrap_or_default();
        return Some(Battery { percent, charging: status == "Charging" || status == "Full" });
    }
    None
}

/// Backlight brightness percent, or None (desktop monitors use DDC, not sysfs).
pub fn brightness() -> Option<u32> {
    let dir = Path::new("/sys/class/backlight");
    let rd = std::fs::read_dir(dir).ok()?;
    let e = rd.flatten().next()?;
    let p = e.path();
    let cur: f64 = read_trim(&p.join("brightness"))?.parse().ok()?;
    let max: f64 = read_trim(&p.join("max_brightness"))?.parse().ok()?;
    if max <= 0.0 {
        return None;
    }
    Some(((cur / max) * 100.0).round() as u32)
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Whether gaming mode is on (same state file the Waybar module polled).
pub fn gamemode_on() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    let p = Path::new(&home).join(".config/tezca/game.state");
    read_trim(&p).map(|s| s.contains("on")).unwrap_or(false)
}
