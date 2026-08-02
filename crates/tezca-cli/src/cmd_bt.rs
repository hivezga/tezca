//! `tezca bt` — Bluetooth, over BlueZ's `bluetoothctl`.
//!
//!   status [--machine]        adapter state + connected count
//!   power on|off|toggle       the adapter
//!   scan [--seconds N]        discover nearby devices (blocking)
//!   list [--machine]          paired / known devices
//!   connect|disconnect <id>   `id` is a MAC or an exact device name
//!   pair|trust|remove <id>
//!
//! ## Why `bluetoothctl` and not BlueZ's D-Bus API
//!
//! The D-Bus API is nicer — property changes push, so nothing has to poll. But
//! this crate is std-only by design (DESIGN.md §8) and a D-Bus client is not
//! something to hand-roll over a raw socket. The bar already carries `zbus` for
//! the system tray, so a future push-based reader can live there; the CLI stays
//! text-parsing, like every other data source in the project.
//!
//! Only non-interactive invocations are used. `bluetoothctl` is really a REPL,
//! and driving a REPL by writing to its stdin is how these integrations
//! traditionally become flaky; every subcommand below returns on its own.

use crate::{term, util, validate};
use std::process::{Command, Stdio};

/// How long `scan` listens by default. Long enough for a headset to answer,
/// short enough that a GUI can wait for it with a spinner.
const DEFAULT_SCAN_SECS: u32 = 10;

pub fn run(args: &[&str]) -> i32 {
    if !util::has("bluetoothctl") {
        eprintln!(
            "{} bluetoothctl not found — install BlueZ (`paru -S bluez bluez-utils`)",
            term::red("error:")
        );
        return 1;
    }
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(args.get(1..).unwrap_or(&[])),
        Some("power") => cmd_power(&args[1..]),
        Some("scan") => cmd_scan(&args[1..]),
        Some("list") | Some("devices") => cmd_list(&args[1..]),
        Some("connect") => cmd_device_action("connect", &args[1..]),
        Some("disconnect") => cmd_device_action("disconnect", &args[1..]),
        Some("pair") => cmd_device_action("pair", &args[1..]),
        Some("trust") => cmd_device_action("trust", &args[1..]),
        Some("remove") | Some("forget") => cmd_device_action("remove", &args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown bt subcommand: {other}\n  try: status · power on|off · scan · list · \
             connect <id> · disconnect <id> · pair <id> · remove <id>"
        )),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{} {e}", term::red("error:"));
            1
        }
    }
}

fn bt(args: &[&str]) -> Result<String, String> {
    let out = Command::new("bluetoothctl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run bluetoothctl: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { stdout.trim().to_string() } else { err });
    }
    Ok(stdout)
}

fn bt_opt(args: &[&str]) -> Option<String> {
    bt(args).ok()
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Adapter {
    present: bool,
    name: String,
    mac: String,
    powered: bool,
    discoverable: bool,
}

fn adapter() -> Adapter {
    let mut a = Adapter::default();
    // "Controller 08:71:90:80:D9:CC Hivezga-pc [default]"
    if let Some(list) = bt_opt(&["list"]) {
        if let Some(line) = list.lines().find(|l| l.trim_start().starts_with("Controller")) {
            let mut it = line.split_whitespace().skip(1);
            a.mac = it.next().unwrap_or_default().to_string();
            let rest: Vec<&str> = it.collect();
            a.name = rest.join(" ").replace("[default]", "").trim().to_string();
            a.present = !a.mac.is_empty();
        }
    }
    if a.present {
        if let Some(info) = bt_opt(&["show"]) {
            a.powered = yes_field(&info, "Powered");
            a.discoverable = yes_field(&info, "Discoverable");
        }
    }
    a
}

/// `\tPowered: yes` → true. Matches on the key, so a device named "Powered"
/// cannot be mistaken for the field.
fn yes_field(text: &str, key: &str) -> bool {
    text.lines()
        .filter_map(|l| l.trim().split_once(": "))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.trim() == "yes")
        .unwrap_or(false)
}

fn cmd_status(args: &[&str]) -> Result<(), String> {
    let a = adapter();
    let devices = devices(true)?;
    let connected: Vec<&Device> = devices.iter().filter(|d| d.connected).collect();

    if args.iter().any(|x| *x == "--machine" || *x == "-m") {
        println!("present={}", a.present);
        println!("name={}", a.name);
        println!("mac={}", a.mac);
        println!("powered={}", a.powered);
        println!("discoverable={}", a.discoverable);
        println!("connected={}", connected.len());
        return Ok(());
    }
    println!("{}", term::header("tezca bt"));
    println!();
    if !a.present {
        println!("  {}", term::dim("no Bluetooth adapter"));
        println!();
        return Ok(());
    }
    println!(
        "  {} {}  {}",
        if a.powered { term::green("●") } else { term::dim("○") },
        term::bold(&a.name),
        term::dim(&format!("{} · {}", a.mac, if a.powered { "on" } else { "off" }))
    );
    for d in connected {
        let bat = d.battery.map(|b| format!(" · {b}%")).unwrap_or_default();
        println!("  {} {}{}", term::cyan("↔"), term::bold(&d.name), term::dim(&bat));
    }
    println!();
    Ok(())
}

fn cmd_power(args: &[&str]) -> Result<(), String> {
    let a = adapter();
    if !a.present {
        return Err("no Bluetooth adapter found".into());
    }
    let want = match args.first().copied() {
        Some("on") => true,
        Some("off") => false,
        Some("toggle") | None => !a.powered,
        Some(other) => return Err(format!("expected on, off or toggle — got {other:?}")),
    };
    bt(&["power", if want { "on" } else { "off" }])?;
    println!("  {} bluetooth {}", term::green("✓"), if want { "on" } else { "off" });
    Ok(())
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

struct Device {
    mac: String,
    name: String,
    paired: bool,
    trusted: bool,
    connected: bool,
    battery: Option<u32>,
    icon: String,
}

/// `paired_only = false` also includes devices merely seen in a recent scan.
fn devices(paired_only: bool) -> Result<Vec<Device>, String> {
    let list = if paired_only {
        bt_opt(&["devices", "Paired"]).unwrap_or_default()
    } else {
        bt_opt(&["devices"]).unwrap_or_default()
    };

    let mut out = Vec::new();
    for line in list.lines() {
        // "Device AA:BB:CC:DD:EE:FF WH-1000XM4"
        let Some(rest) = line.trim().strip_prefix("Device ") else { continue };
        let mut it = rest.splitn(2, ' ');
        let mac = it.next().unwrap_or_default().trim().to_string();
        if validate::mac(&mac).is_err() {
            continue;
        }
        let name = it.next().unwrap_or("").trim().to_string();
        let info = bt_opt(&["info", &mac]).unwrap_or_default();
        out.push(Device {
            paired: yes_field(&info, "Paired"),
            trusted: yes_field(&info, "Trusted"),
            connected: yes_field(&info, "Connected"),
            battery: battery_percentage(&info),
            icon: info
                .lines()
                .filter_map(|l| l.trim().split_once(": "))
                .find(|(k, _)| *k == "Icon")
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default(),
            name: if name.is_empty() { mac.clone() } else { name },
            mac,
        });
    }
    out.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
    Ok(out)
}

/// `Battery Percentage: 0x5a (90)` → 90.
///
/// The decimal in parentheses is the one to read; the hex prefix is the raw
/// attribute value and parsing that instead silently reports 90 as 5.
fn battery_percentage(info: &str) -> Option<u32> {
    let line = info.lines().map(str::trim).find(|l| l.starts_with("Battery Percentage:"))?;
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    line[open + 1..close].trim().parse().ok()
}

fn cmd_list(args: &[&str]) -> Result<(), String> {
    let all = args.contains(&"--all");
    let ds = devices(!all)?;
    if args.iter().any(|a| *a == "--machine" || *a == "-m") {
        for d in &ds {
            println!("@device");
            println!("mac={}", d.mac);
            println!("name={}", d.name);
            println!("paired={}", d.paired);
            println!("trusted={}", d.trusted);
            println!("connected={}", d.connected);
            println!("battery={}", d.battery.map(|b| b.to_string()).unwrap_or_default());
            println!("icon={}", d.icon);
        }
        return Ok(());
    }
    println!("{}", term::header("tezca bt"));
    println!();
    if ds.is_empty() {
        // Pointing at `scan` is only useful advice when a scan is not what just
        // ran; after one it reads as though the command did nothing.
        println!(
            "  {}",
            term::dim(if all {
                "no devices found — check the device is powered on and in pairing mode"
            } else {
                "no paired devices — `tezca bt scan` to find some"
            })
        );
    }
    for d in &ds {
        let dot = if d.connected { term::green("●") } else { term::dim("○") };
        let mut tags: Vec<String> = Vec::new();
        if let Some(b) = d.battery {
            tags.push(format!("{b}%"));
        }
        if !d.icon.is_empty() {
            tags.push(d.icon.clone());
        }
        if !d.paired {
            tags.push("unpaired".into());
        }
        println!(
            "  {} {:<26} {}",
            dot,
            term::bold(&d.name),
            term::dim(&format!("{}  {}", d.mac, tags.join(" · ")))
        );
    }
    println!();
    Ok(())
}

/// Resolve a MAC or an exact device name to a MAC.
///
/// An ambiguous name is an error rather than a guess: "connect the other one of
/// your two identical headsets" is not a decision to make on the user's behalf.
fn resolve(id: &str) -> Result<String, String> {
    if validate::mac(id).is_ok() {
        return Ok(id.to_uppercase());
    }
    let ds = devices(false)?;
    let matches: Vec<&Device> = ds.iter().filter(|d| d.name.eq_ignore_ascii_case(id)).collect();
    match matches.len() {
        1 => Ok(matches[0].mac.clone()),
        0 => Err(format!(
            "no Bluetooth device called '{id}' — run `tezca bt list` (or `scan` first)"
        )),
        n => Err(format!("'{id}' matches {n} devices — use the address instead")),
    }
}

fn cmd_device_action(action: &str, args: &[&str]) -> Result<(), String> {
    let id =
        args.first().copied().ok_or_else(|| format!("usage: tezca bt {action} <address|name>"))?;
    let mac = resolve(id)?;
    let out = bt(&[action, &mac])?;
    // bluetoothctl reports failures on stdout with a zero exit status, in the
    // shape "Failed to connect: org.bluez.Error.NotAvailable".
    if let Some(line) = out.lines().map(str::trim).find(|l| l.starts_with("Failed")) {
        return Err(line.to_string());
    }
    println!("  {} {action} {}", term::green("✓"), term::bold(id));
    Ok(())
}

fn cmd_scan(args: &[&str]) -> Result<(), String> {
    let mut secs = DEFAULT_SCAN_SECS;
    let mut it = args.iter().copied();
    while let Some(a) = it.next() {
        if a == "--seconds" || a == "-s" {
            secs =
                it.next().and_then(|v| v.parse().ok()).ok_or("--seconds needs a whole number")?;
        }
    }
    let secs = secs.clamp(1, 60);

    let a = adapter();
    if !a.present {
        return Err("no Bluetooth adapter found".into());
    }
    if !a.powered {
        bt(&["power", "on"])?;
    }

    // `--timeout` makes this return on its own. Without it bluetoothctl scans
    // until killed, which is exactly the kind of unbounded child a GUI must never
    // spawn.
    let status = Command::new("bluetoothctl")
        .args(["--timeout", &secs.to_string(), "scan", "on"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("failed to run bluetoothctl: {e}"))?;
    if !status.success() {
        return Err("scan failed".into());
    }

    cmd_list(&scan_list_args(args))
}

/// The flags a post-scan listing runs with.
///
/// `--all`, never the caller's own flags. `cmd_scan`'s `args` are its
/// `--seconds N`, so passing them straight through meant `--all` was never set
/// and a scan listed the *paired* devices it had not just discovered —
/// answering "no paired devices" immediately after finding one. Only `--machine`
/// is worth carrying across, so a GUI caller still gets the parseable form.
fn scan_list_args(args: &[&str]) -> Vec<&'static str> {
    let mut out = vec!["--all"];
    if args.iter().any(|a| *a == "--machine" || *a == "-m") {
        out.push("--machine");
    }
    out
}

fn print_help() {
    println!("{}", term::header("tezca bt"));
    println!();
    for (c, d) in [
        ("status", "adapter state + connected devices"),
        ("power on|off|toggle", "the adapter"),
        ("scan [--seconds N]", "discover nearby devices, then list them"),
        ("list [--all]", "paired devices (--all includes discovered)"),
        ("connect <id>", "connect by address or exact name"),
        ("disconnect <id>", "disconnect"),
        ("pair <id>", "pair (simple pairing only — see below)"),
        ("trust <id>", "allow auto-reconnect"),
        ("remove <id>", "forget the device"),
    ] {
        println!("  {:<22} {}", term::cyan(c), term::dim(d));
    }
    println!();
    println!("{}", term::dim("  Pairing that needs a displayed passkey is not handled here —"));
    println!("{}", term::dim("  run `bluetoothctl` interactively for those."));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scan_lists_what_it_discovered_not_only_what_was_already_paired() {
        // The scan's own flags must not decide the listing: `--seconds 12` was
        // being forwarded verbatim, so `--all` was never set and a successful
        // scan reported "no paired devices" having just found a device.
        assert_eq!(scan_list_args(&["--seconds", "12"]), vec!["--all"]);
        assert_eq!(scan_list_args(&[]), vec!["--all"]);
        // A GUI caller still gets the parseable form.
        assert_eq!(scan_list_args(&["--machine"]), vec!["--all", "--machine"]);
        assert_eq!(scan_list_args(&["-s", "5", "-m"]), vec!["--all", "--machine"]);
    }

    const INFO: &str = "Device AA:BB:CC:DD:EE:FF (public)
\tName: WH-1000XM4
\tAlias: WH-1000XM4
\tPaired: yes
\tTrusted: no
\tBlocked: no
\tConnected: yes
\tIcon: audio-headset
\tBattery Percentage: 0x5a (90)
";

    #[test]
    fn reads_the_decimal_battery_not_the_hex_prefix() {
        // "0x5a (90)" — parsing the hex token instead reports 5, which looks
        // plausible enough on a meter that nobody would notice.
        assert_eq!(battery_percentage(INFO), Some(90));
        assert_eq!(battery_percentage("\tBattery Percentage: 0x64 (100)"), Some(100));
        assert_eq!(battery_percentage("\tConnected: yes"), None);
    }

    #[test]
    fn reads_yes_no_fields_by_key() {
        assert!(yes_field(INFO, "Paired"));
        assert!(yes_field(INFO, "Connected"));
        assert!(!yes_field(INFO, "Trusted"));
        assert!(!yes_field(INFO, "Blocked"));
        // A key that is not there is simply false, not a panic.
        assert!(!yes_field(INFO, "Powered"));
    }

    #[test]
    fn a_device_name_containing_a_space_survives_the_split() {
        let line = "Device 08:71:90:80:D9:CC My Living Room Speaker";
        let rest = line.trim().strip_prefix("Device ").unwrap();
        let mut it = rest.splitn(2, ' ');
        assert_eq!(it.next().unwrap(), "08:71:90:80:D9:CC");
        assert_eq!(it.next().unwrap(), "My Living Room Speaker");
    }
}
