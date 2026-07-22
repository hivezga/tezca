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

/// GPU utilization fraction in [0,1], or None when no source is available.
///
/// Tries the generic DRM sysfs `gpu_busy_percent` first (AMD/Intel expose it),
/// then `nvidia-smi` (the target rig's RTX 4070 Ti on nvidia-open). None → the
/// bar hides the GPU metric, exactly like battery/brightness.
pub fn gpu() -> Option<f64> {
    if let Some(f) = sysfs_gpu_busy() {
        return Some(f);
    }
    nvidia_gpu()
}

/// First `card<N>` DRM device exposing `device/gpu_busy_percent` (0–100).
fn sysfs_gpu_busy() -> Option<f64> {
    let rd = std::fs::read_dir("/sys/class/drm").ok()?;
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_str().unwrap_or("");
        // Match cardN (a whole GPU), not cardN-DP-1 connectors or renderD* nodes.
        let is_card = name.len() > 4
            && name.starts_with("card")
            && name[4..].chars().all(|c| c.is_ascii_digit());
        if !is_card {
            continue;
        }
        let p = e.path().join("device/gpu_busy_percent");
        if let Some(v) = read_trim(&p).and_then(|s| s.parse::<f64>().ok()) {
            return Some((v / 100.0).clamp(0.0, 1.0));
        }
    }
    None
}

/// NVIDIA utilization via `nvidia-smi` (first GPU).
fn nvidia_gpu() -> Option<f64> {
    let out = Command::new("nvidia-smi")
        .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let v: f64 = s.lines().next()?.trim().parse().ok()?;
    Some((v / 100.0).clamp(0.0, 1.0))
}

// ── Hardware detail (metric popovers) ───────────────────────────────────────
//
// The right-cluster CPU/MEM/GPU groups expand into a glass popover on click.
// These readers gather the extra telemetry that doesn't fit on the bar: temps
// (hwmon), clocks, load, memory breakdown, and GPU power/VRAM. Everything is
// best-effort — any field the hardware doesn't expose stays `None` and its row
// is simply omitted.

/// First `tempN_input` (°C) on the hwmon chip named `chip`, preferring an input
/// whose `tempN_label` contains one of `pref` (else the first temp on the chip).
fn hwmon_temp(chip: &str, pref: &[&str]) -> Option<f64> {
    let rd = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if read_trim(&p.join("name")).as_deref() != Some(chip) {
            continue;
        }
        let read_c = |i: u32| {
            read_trim(&p.join(format!("temp{i}_input")))
                .and_then(|s| s.parse::<f64>().ok())
                .map(|m| m / 1000.0)
        };
        for want in pref {
            for i in 1..=16 {
                let label = read_trim(&p.join(format!("temp{i}_label"))).unwrap_or_default();
                if label.contains(want) {
                    if let Some(t) = read_c(i) {
                        return Some(t);
                    }
                }
            }
        }
        for i in 1..=16 {
            if let Some(t) = read_c(i) {
                return Some(t);
            }
        }
    }
    None
}

/// CPU package temperature in °C, from whichever driver the platform exposes.
pub fn cpu_temp() -> Option<f64> {
    for (chip, pref) in [
        ("k10temp", &["Tctl", "Tdie"][..]),
        ("zenpower", &["Tdie"][..]),
        ("coretemp", &["Package"][..]),
        ("cpu_thermal", &[][..]),
        ("acpitz", &[][..]),
    ] {
        if let Some(t) = hwmon_temp(chip, pref) {
            return Some(t);
        }
    }
    None
}

/// Mean current core clock in MHz across all `cpufreq` policies.
fn cpu_freq_mhz() -> Option<f64> {
    let rd = std::fs::read_dir("/sys/devices/system/cpu").ok()?;
    let (mut sum, mut n) = (0.0, 0u32);
    for e in rd.flatten() {
        let p = e.path().join("cpufreq/scaling_cur_freq");
        if let Some(khz) = read_trim(&p).and_then(|s| s.parse::<f64>().ok()) {
            sum += khz;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f64 / 1000.0)
}

/// The 1 / 5 / 15-minute load averages from /proc/loadavg.
fn loadavg() -> (f64, f64, f64) {
    let t = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut it = t.split_whitespace().filter_map(|s| s.parse::<f64>().ok());
    (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
}

/// Expanded CPU telemetry for the metric popover.
pub struct CpuDetail {
    pub model: String,
    pub temp_c: Option<f64>,
    pub freq_mhz: Option<f64>,
    pub threads: usize,
    pub load: (f64, f64, f64),
}

pub fn cpu_detail() -> CpuDetail {
    let model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| "CPU".to_string());
    CpuDetail {
        model,
        temp_c: cpu_temp(),
        freq_mhz: cpu_freq_mhz(),
        threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        load: loadavg(),
    }
}

/// Expanded memory telemetry (all fields in kB, matching /proc/meminfo).
pub struct MemDetail {
    pub total_kb: f64,
    pub used_kb: f64,
    pub available_kb: f64,
    pub cached_kb: f64,
    pub buffers_kb: f64,
    pub swap_total_kb: f64,
    pub swap_used_kb: f64,
    pub dimm_temp_c: Option<f64>,
}

pub fn mem_detail() -> MemDetail {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let get = |key: &str| -> f64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let total = get("MemTotal:");
    let available = get("MemAvailable:");
    let swap_total = get("SwapTotal:");
    let swap_free = get("SwapFree:");
    MemDetail {
        total_kb: total,
        used_kb: (total - available).max(0.0),
        available_kb: available,
        cached_kb: get("Cached:"),
        buffers_kb: get("Buffers:"),
        swap_total_kb: swap_total,
        swap_used_kb: (swap_total - swap_free).max(0.0),
        // jc42 SPD sensors sit on the DIMMs; take the hottest module.
        dimm_temp_c: hwmon_temp("jc42", &[]),
    }
}

/// Expanded GPU telemetry for the metric popover (fields absent → `None`).
pub struct GpuDetail {
    pub name: String,
    pub temp_c: Option<f64>,
    pub power_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub mem_used_mb: Option<f64>,
    pub mem_total_mb: Option<f64>,
    pub core_clock_mhz: Option<f64>,
    pub mem_clock_mhz: Option<f64>,
    pub fan_pct: Option<f64>,
    pub util_pct: Option<f64>,
}

pub fn gpu_detail() -> Option<GpuDetail> {
    sysfs_gpu_detail().or_else(nvidia_detail)
}

/// NVIDIA telemetry from a single batched `nvidia-smi` query.
fn nvidia_detail() -> Option<GpuDetail> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,temperature.gpu,power.draw,power.limit,memory.used,\
             memory.total,clocks.gr,clocks.mem,fan.speed,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let f: Vec<String> = s.lines().next()?.split(',').map(|x| x.trim().to_string()).collect();
    if f.len() < 10 {
        return None;
    }
    // "[N/A]" and blanks parse to None, which is exactly what we want.
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
    Some(GpuDetail {
        name: f[0].clone(),
        temp_c: num(1),
        power_w: num(2),
        power_limit_w: num(3),
        mem_used_mb: num(4),
        mem_total_mb: num(5),
        core_clock_mhz: num(6),
        mem_clock_mhz: num(7),
        fan_pct: num(8),
        util_pct: num(9),
    })
}

/// Best-effort AMD/Intel telemetry from sysfs (temp + utilization + power).
fn sysfs_gpu_detail() -> Option<GpuDetail> {
    let temp = hwmon_temp("amdgpu", &["edge", "junction"]).or_else(|| hwmon_temp("i915", &[]));
    let util = sysfs_gpu_busy().map(|f| f * 100.0);
    let power = hwmon_power_w("amdgpu");
    if temp.is_none() && util.is_none() {
        return None;
    }
    Some(GpuDetail {
        name: "GPU".to_string(),
        temp_c: temp,
        power_w: power,
        power_limit_w: None,
        mem_used_mb: None,
        mem_total_mb: None,
        core_clock_mhz: None,
        mem_clock_mhz: None,
        fan_pct: None,
        util_pct: util,
    })
}

/// `power1_average` (µW → W) on the hwmon chip named `chip`.
fn hwmon_power_w(chip: &str) -> Option<f64> {
    let rd = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if read_trim(&p.join("name")).as_deref() != Some(chip) {
            continue;
        }
        if let Some(uw) = read_trim(&p.join("power1_average")).and_then(|s| s.parse::<f64>().ok()) {
            return Some(uw / 1_000_000.0);
        }
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
