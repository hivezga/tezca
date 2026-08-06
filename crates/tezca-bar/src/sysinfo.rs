//! System metrics — CPU, memory, network, audio, battery, brightness, gamemode.
//!
//! All std/shell-out, matching the repo's idioms (the CLI shells to hyprctl /
//! wpctl / nmcli; the dock reads /proc): CPU & memory come straight from /proc,
//! network from nmcli + /proc/net, audio from wpctl, battery/brightness from
//! sysfs, gamemode from the state file `tezca game` writes. Anything
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
        let nums: Vec<u64> =
            line.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
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
            let f = split_terse(line);
            if f.first().map(String::as_str) == Some("yes") {
                let signal = f.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let ssid = f.get(2).cloned().unwrap_or_default();
                let ip = primary_ip("wifi").unwrap_or_default();
                return Net::Wifi { signal, ssid, ip };
            }
        }
    }
    // Then a connected wired device.
    if let Some(out) = nmcli(&["-t", "-f", "TYPE,STATE", "device", "status"]) {
        for line in out.lines() {
            let f = split_terse(line);
            let connected = f.get(1).map(|s| s.starts_with("connected")).unwrap_or(false);
            if f.first().map(String::as_str) == Some("ethernet") && connected {
                let ip = primary_ip("ethernet").unwrap_or_default();
                return Net::Ethernet { ip };
            }
        }
    }
    Net::Disconnected
}

/// Split one `nmcli -t` record into fields, undoing its escaping.
///
/// nmcli separates fields with `:` and escapes a `:` or `\` *inside* a value as
/// `\:` / `\\`. This used to split on a bare `:` and rejoin the tail, which meant
/// an SSID containing a colon rendered in the bar with a stray backslash — and
/// any field after such a value was read out of the wrong column. The CLI's
/// `cmd_net` has the same routine; the two crates share no library.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            fields.last_mut().expect("always non-empty").push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            fields.push(String::new());
        } else {
            fields.last_mut().expect("always non-empty").push(ch);
        }
    }
    if escaped {
        fields.last_mut().expect("always non-empty").push('\\');
    }
    fields
}

/// First IPv4 address of the first connected device of `kind` (wifi|ethernet).
fn primary_ip(kind: &str) -> Option<String> {
    let out = nmcli(&["-t", "-f", "DEVICE,TYPE,STATE", "device", "status"])?;
    let dev = out.lines().find_map(|l| {
        let f = split_terse(l);
        (f.len() >= 3 && f[1] == kind && f[2].starts_with("connected")).then(|| f[0].clone())
    })?;
    let show = nmcli(&["-t", "-f", "IP4.ADDRESS", "device", "show", &dev])?;
    show.lines().find_map(|l| {
        let f = split_terse(l);
        f.get(1).map(|v| v.split('/').next().unwrap_or(v).to_string())
    })
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
    /// Seconds until empty (discharging) or full (charging), when the driver
    /// gives us enough to work it out. `None` at rest, or on a battery that
    /// reports no rate — the bar then shows the percentage alone rather than a
    /// made-up estimate.
    pub secs_remaining: Option<u64>,
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
        let percent = read_trim(&p.join("capacity")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let status = read_trim(&p.join("status")).unwrap_or_default();
        let charging = status == "Charging" || status == "Full";
        return Some(Battery { percent, charging, secs_remaining: battery_secs(&p, charging) });
    }
    None
}

/// The figures the battery popover shows beyond the percentage.
#[derive(Default)]
pub struct BatteryDetail {
    pub model: String,
    pub status: String,
    /// Present draw, watts. Positive whether charging or discharging.
    pub power_w: Option<f64>,
    /// Full charge now vs. as designed, as a percentage — battery health.
    pub health_pct: Option<f64>,
    /// Wh now full / Wh when new.
    pub capacity_wh: Option<(f64, f64)>,
    pub cycles: Option<u64>,
    pub temp_c: Option<f64>,
}

/// Everything else the first battery reports. `None` on a desktop.
///
/// Split from [`battery`] because this is only read while the popover is open:
/// the bar itself needs two fields, and the other six are a dozen small sysfs
/// reads that would otherwise happen every two seconds forever.
pub fn battery_detail() -> Option<BatteryDetail> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if read_trim(&p.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        let num = |f: &str| read_trim(&p.join(f)).and_then(|s| s.parse::<f64>().ok());
        // µWh → Wh, µW → W. Charge-reporting batteries give µAh instead, which
        // is not convertible to Wh without a voltage, so those simply have no
        // capacity figure rather than a wrong one.
        let full_now = num("energy_full").map(|v| v / 1e6);
        let full_design = num("energy_full_design").map(|v| v / 1e6);
        return Some(BatteryDetail {
            model: read_trim(&p.join("model_name")).unwrap_or_default(),
            status: read_trim(&p.join("status")).unwrap_or_default(),
            power_w: num("power_now").map(|v| v / 1e6),
            health_pct: match (full_now, full_design) {
                (Some(n), Some(d)) if d > 0.0 => Some(n / d * 100.0),
                _ => None,
            },
            capacity_wh: full_now.zip(full_design),
            cycles: read_trim(&p.join("cycle_count")).and_then(|s| s.parse().ok()),
            // Reported in tenths of a degree.
            temp_c: num("temp").map(|v| v / 10.0),
        });
    }
    None
}

/// Seconds to empty/full for the battery at `p`.
///
/// Drivers disagree about which pair of files they expose: some report energy
/// (µWh) against power (µW), others charge (µAh) against current (µA). Either
/// pair divides to hours because the µ- prefixes cancel, so try energy first and
/// fall back to charge rather than picking one and calling the other broken.
/// A zero rate means "not moving" — no estimate exists, so we return `None`
/// instead of dividing by zero into infinity.
fn battery_secs(p: &Path, charging: bool) -> Option<u64> {
    let num = |f: &str| read_trim(&p.join(f)).and_then(|s| s.parse::<f64>().ok());
    let (now, full, rate) = match (num("energy_now"), num("power_now")) {
        (Some(n), Some(r)) => (n, num("energy_full"), r),
        _ => (num("charge_now")?, num("charge_full"), num("current_now")?),
    };
    if rate <= 0.0 {
        return None;
    }
    // Charging counts up to full; discharging counts down to nothing.
    let delta = if charging { (full? - now).max(0.0) } else { now };
    Some((delta / rate * 3600.0) as u64)
}

/// `13560` → `3h 46m`, `900` → `15m`. Empty for a nonsensical span.
pub fn duration_short(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    // Past a day the estimate is noise — a laptop that claims 40h is telling you
    // its rate sample is bad, not that it will last two days.
    if h >= 24 {
        return String::new();
    }
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
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

/// Per-core busy fractions, from the `cpuN` lines of /proc/stat.
///
/// Separate from [`CpuMeter`] because it keeps one previous sample *per core*
/// and is only sampled while a popover is open — a 16-core machine would
/// otherwise pay for 16 deltas a second to render a grid nobody is looking at.
#[derive(Default)]
pub struct CoreMeter {
    last: Vec<(u64, u64)>, // (total, idle) per core
}

impl CoreMeter {
    /// Busy fraction per core since the previous call. Empty on the first call
    /// — there is no delta yet, and showing zeros would read as "idle".
    pub fn sample(&mut self) -> Vec<f64> {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else { return Vec::new() };
        let now: Vec<(u64, u64)> = stat
            .lines()
            .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
            .filter_map(|l| {
                let n: Vec<u64> =
                    l.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
                (n.len() >= 4).then(|| (n.iter().sum(), n[3] + n.get(4).copied().unwrap_or(0)))
            })
            .collect();

        // A core count that changed (hotplug, or the first sample) invalidates
        // every stored delta at once.
        let out = if self.last.len() == now.len() {
            now.iter()
                .zip(&self.last)
                .map(|((t, i), (pt, pi))| {
                    let dt = t.saturating_sub(*pt);
                    let di = i.saturating_sub(*pi);
                    if dt == 0 {
                        0.0
                    } else {
                        (1.0 - di as f64 / dt as f64).clamp(0.0, 1.0)
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        self.last = now;
        out
    }
}

/// One row of the "top processes" list.
pub struct Proc {
    pub name: String,
    pub pid: u32,
    /// Resident set size, kB.
    pub rss_kb: u64,
    /// Total CPU jiffies used, for the caller to delta against a previous read.
    pub cpu_jiffies: u64,
}

/// Every process we can read, with its name, RSS and cumulative CPU time.
///
/// Deliberately not shelling out to `ps`: this runs while a popover is open,
/// and a subprocess per open is both slower and one more thing that can be
/// missing. Unreadable entries (a process that exited mid-scan, or one owned by
/// another user) are skipped rather than reported as zero.
pub fn processes() -> Vec<Proc> {
    let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let p = e.path();
        // `stat`'s comm field is parenthesised and may itself contain spaces or
        // parentheses, so the fields after it are found from the LAST ')'.
        let Ok(stat) = std::fs::read_to_string(p.join("stat")) else { continue };
        let Some(close) = stat.rfind(')') else { continue };
        let comm = stat[..close].split_once('(').map(|(_, c)| c).unwrap_or("").to_string();
        let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        // After the state field: utime is index 11, stime 12 (1-based field 14/15).
        let num = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let rss_pages = num(21);
        out.push(Proc {
            name: comm,
            pid,
            rss_kb: rss_pages * page_size_kb(),
            cpu_jiffies: num(11) + num(12),
        });
    }
    out
}

/// Page size in kB, worked out once by asking the kernel about *us*.
///
/// `/proc/<pid>/stat` reports RSS in pages while `/proc/<pid>/status` reports
/// it in kB, so dividing our own two figures gives the page size with no libc
/// binding and no `unsafe` — and no hardcoded 4, which would silently scale
/// every memory figure wrong on a 16K-page kernel. Falls back to 4 only when
/// one of the two reads is unavailable.
fn page_size_kb() -> u64 {
    use std::sync::OnceLock;
    static SIZE: OnceLock<u64> = OnceLock::new();
    *SIZE.get_or_init(|| {
        let pages = std::fs::read_to_string("/proc/self/stat")
            .ok()
            .and_then(|s| {
                let close = s.rfind(')')?;
                s[close + 1..].split_whitespace().nth(21)?.parse::<u64>().ok()
            })
            .filter(|p| *p > 0);
        let kb = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))?
                .split_whitespace()
                .nth(1)?
                .parse::<u64>()
                .ok()
        });
        match (pages, kb) {
            (Some(p), Some(k)) if k > 0 => snap_page_size(k as f64 / p as f64),
            _ => 4,
        }
    })
}

/// Snap a measured pages-to-kB ratio onto a real page size.
///
/// The two `/proc` files are read a moment apart and this process is allocating
/// in between, so the raw quotient drifts — on a 4K kernel it measures anywhere
/// from about 3.5 to 5.5. Page sizes are powers of two by definition, so the
/// answer is the nearest candidate rather than the rounded quotient. (Rounding
/// alone silently reported 5 kB here, which would have made every memory figure
/// in the bar 25% too large.)
fn snap_page_size(ratio: f64) -> u64 {
    const CANDIDATES: [u64; 5] = [4, 8, 16, 32, 64];
    *CANDIDATES
        .iter()
        .min_by(|a, b| {
            let d = |v: u64| (ratio - v as f64).abs();
            d(**a).total_cmp(&d(**b))
        })
        .expect("CANDIDATES is never empty")
}

/// One application stream in the mixer, from `wpctl status`.
pub struct Stream {
    pub name: String,
    pub volume: u32,
    pub muted: bool,
}

/// The per-application playback streams PipeWire currently has.
///
/// `wpctl status` is the only interface here that does not need a subscription;
/// its output is a tree with a "Streams:" section under Audio. Parsed
/// defensively — a format change should cost the list, not the popover.
pub fn streams() -> Vec<Stream> {
    wpctl_section("Streams:")
        .into_iter()
        .filter_map(|d| {
            let a = audio_of(&d.id.to_string())?;
            Some(Stream { name: d.name, volume: a.volume, muted: a.muted })
        })
        .collect()
}

/// One routable PipeWire node — a sink or a source — from `wpctl status`.
pub struct Device {
    pub id: u32,
    pub name: String,
    /// Carries the `*` marker: this is what `@DEFAULT_AUDIO_SINK@` resolves to.
    pub default: bool,
}

/// The playback devices PipeWire advertises, in `wpctl status` order.
pub fn sinks() -> Vec<Device> {
    wpctl_section("Sinks:")
}

/// The capture devices PipeWire advertises.
pub fn sources() -> Vec<Device> {
    wpctl_section("Sources:")
}

/// Route playback to `id`.
///
/// WirePlumber's default policy re-homes any stream that has not asked for a
/// specific target, so this moves what is already playing as well as what
/// connects next. There is no `wpctl move` to fall back on if a stream has
/// pinned itself — that one keeps its device, which is what it asked for.
pub fn set_default_sink(id: u32) {
    let _ = Command::new("wpctl").args(["set-default", &id.to_string()]).status();
}

fn wpctl_section(header: &str) -> Vec<Device> {
    wpctl(&["status"]).map(|out| parse_wpctl_section(&out, header)).unwrap_or_default()
}

/// Read one `wpctl status` section as id / name / default rows.
///
/// The output is a tree, so every line carries box-drawing glyphs that have to
/// come off before the `<id>. <name>` shape is visible, and the default node is
/// marked with a `*`. Parsed defensively: an unrecognised line costs that row,
/// not the section.
fn parse_wpctl_section(out: &str, header: &str) -> Vec<Device> {
    let mut rows = Vec::new();
    let mut inside = false;
    for line in out.lines() {
        let t =
            line.trim_matches(|c: char| c.is_whitespace() || matches!(c, '│' | '├' | '└' | '─'));
        if t.starts_with(header) {
            inside = true;
            continue;
        }
        if inside && t.ends_with(':') {
            break;
        }
        if !inside {
            continue;
        }
        // Under Streams:, each client node is followed by its individual ports
        // ("82. output_FL  > ALCS1200A Analog:playback_FL"). Those are links,
        // not things you can set a volume on, and listing them in the mixer put
        // an `output_FL` row under every application.
        if t.contains('>') {
            continue;
        }
        let (default, t) = match t.strip_prefix('*') {
            Some(rest) => (true, rest.trim()),
            None => (false, t),
        };
        let Some((id, rest)) = t.split_once('.') else { continue };
        let Ok(id) = id.trim().parse::<u32>() else { continue };
        // "Analog Stereo [vol: 0.10]" — the level already has its own row.
        let name = rest.split(" [vol:").next().unwrap_or(rest).trim();
        if !name.is_empty() {
            rows.push(Device { id, name: name.to_string(), default });
        }
    }
    rows
}

/// What the audio server calls itself, and the format it is running at.
///
/// Both come from `pactl info`; either line may be missing on a server that
/// isn't PipeWire-through-pulse, in which case the row is simply omitted.
#[derive(Default)]
pub struct AudioServer {
    pub server: String,
    pub spec: String,
}

pub fn audio_server() -> AudioServer {
    let mut s = AudioServer::default();
    let Some(out) = Command::new("pactl")
        .arg("info")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    else {
        return s;
    };
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("Server Name:") {
            s.server = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Default Sample Specification:") {
            s.spec = v.trim().to_string();
        }
    }
    s
}

/// Seconds since boot, from /proc/uptime.
pub fn uptime_secs() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/uptime").ok()?;
    s.split_whitespace().next()?.parse::<f64>().ok().map(|v| v as u64)
}

fn wpctl(args: &[&str]) -> Option<String> {
    let out = Command::new("wpctl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
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

/// Whether gaming mode is on (the state file `tezca game` writes).
pub fn gamemode_on() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    let p = Path::new(&home).join(".config/tezca/game.state");
    read_trim(&p).map(|s| s.contains("on")).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed verbatim from this machine's `wpctl status`, box glyphs and all.
    const WPCTL: &str = "\
Audio
 ├─ Devices:
 │      45. AD104 High Definition Audio Controller [alsa]
 │
 ├─ Sinks:
 │      54. [G533 Wireless Headset Dongle] Analog Stereo [vol: 0.80]
 │  *   59. Starship/Matisse HD Audio Controller Analog Stereo [vol: 0.15]
 │
 ├─ Sources:
 │      55. [G533 Wireless Headset Dongle] Mono [vol: 1.00]
 │  *   58. C920 HD Pro Webcam Analog Stereo    [vol: 0.44]
 │
 ├─ Filters:
 │
 └─ Streams:
        81. Waydroid
             82. output_FL       > ALCS1200A Analog:playback_FL\t[active]
             84. output_FR       > ALCS1200A Analog:playback_FR\t[active]

Video
 ├─ Sinks:
 │      99. Not an audio sink
";

    #[test]
    fn a_section_stops_at_the_next_header_so_video_never_leaks_into_audio() {
        let sinks = parse_wpctl_section(WPCTL, "Sinks:");
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].id, 54);
        assert_eq!(sinks[0].name, "[G533 Wireless Headset Dongle] Analog Stereo");
        assert!(!sinks[0].default);
        assert_eq!(sinks[1].id, 59);
        // The `*` marks the default, and the volume suffix is not part of a name.
        assert_eq!(sinks[1].name, "Starship/Matisse HD Audio Controller Analog Stereo");
        assert!(sinks[1].default);
    }

    #[test]
    fn the_default_source_is_the_starred_one_not_the_first() {
        let srcs = parse_wpctl_section(WPCTL, "Sources:");
        assert_eq!(srcs.iter().find(|d| d.default).map(|d| d.id), Some(58));
        assert_eq!(srcs[1].name, "C920 HD Pro Webcam Analog Stereo");
    }

    #[test]
    fn a_streams_ports_are_not_mistaken_for_applications() {
        // The bug this covers: `output_FL`/`output_FR` rows appeared in the
        // mixer under every application that had them.
        let streams = parse_wpctl_section(WPCTL, "Streams:");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].name, "Waydroid");
    }

    #[test]
    fn nmcli_terse_fields_survive_a_colon_inside_a_value() {
        // The bug this replaced: splitting on a bare ':' left the SSID as
        // "My\:Net" (rendered with the backslash) and, for any record with more
        // fields after it, read every later column one position to the left.
        assert_eq!(split_terse(r"yes:72:My\:Net"), vec!["yes", "72", "My:Net"]);
        assert_eq!(split_terse("yes:72:Plain"), vec!["yes", "72", "Plain"]);
        assert_eq!(split_terse(r"a\\b:c"), vec![r"a\b", "c"]);
        assert_eq!(split_terse("ethernet:connected"), vec!["ethernet", "connected"]);
    }

    /// The scanner has to find the process running the test, with a sane RSS —
    /// which is also the only end-to-end check that `page_size_kb` is right.
    #[test]
    fn processes_finds_this_one_with_a_plausible_rss() {
        let me = std::process::id();
        let procs = processes();
        assert!(!procs.is_empty(), "no processes readable from /proc");
        let mine = procs.iter().find(|p| p.pid == me).expect("did not find the test process");
        // A Rust test binary is somewhere between a megabyte and a gigabyte.
        // The point is the order of magnitude: a wrong page size lands this
        // 4x or 1/4x out, and a pages-vs-kB mixup lands it 1000x out.
        assert!(
            (1_000..1_000_000).contains(&mine.rss_kb),
            "implausible RSS {} kB — page size likely wrong",
            mine.rss_kb
        );
        assert!(!mine.name.is_empty(), "process name should not be blank");
    }

    #[test]
    fn page_size_is_a_power_of_two() {
        let k = page_size_kb();
        assert!(k > 0 && k.is_power_of_two(), "page size {k} kB is not a power of two");
        // Every architecture this ships to is 4K, 16K or 64K.
        assert!((4..=64).contains(&k), "page size {k} kB is outside anything Linux uses");
    }

    #[test]
    fn core_meter_reports_one_value_per_cpu_after_a_delta() {
        let mut m = CoreMeter::default();
        // First sample has nothing to diff against and must say so with an
        // empty vec rather than a row of convincing zeroes.
        assert!(m.sample().is_empty());
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let second = m.sample();
        assert_eq!(second.len(), n, "one busy fraction per logical core");
        assert!(second.iter().all(|f| (0.0..=1.0).contains(f)));
    }

    #[test]
    fn page_size_snaps_to_a_real_one_not_the_raw_quotient() {
        // The measured ratio drifts because RSS moves between the two reads.
        // Every one of these came from a 4K kernel.
        for r in [3.5, 3.9, 4.0, 4.4, 5.0, 5.6] {
            assert_eq!(snap_page_size(r), 4, "ratio {r} should read as a 4 kB page");
        }
        assert_eq!(snap_page_size(15.2), 16);
        assert_eq!(snap_page_size(61.0), 64);
    }
}
