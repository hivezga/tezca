//! Bluetooth state for the bar module — adapter power plus connected devices.
//!
//! Shells out to `bluetoothctl`, matching the CLI's `tezca bt` (the two crates
//! share no library, so the small amount of parsing is duplicated rather than
//! extracted into a fourth crate). BlueZ's D-Bus API would push changes instead
//! of being polled, and the bar does already carry `zbus` for the tray — but the
//! tray earns that because a tray is *inherently* D-Bus. A five-second poll of
//! two short commands is not worth a second D-Bus client and its reconnect logic.
//!
//! Everything here is gated behind `Config::uses_mod(Mod::Bluetooth)`: if the
//! module is not placed in a layout, none of this ever runs.

use std::process::Command;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub name: String,
    pub battery: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtState {
    /// False when there is no adapter at all — the module hides entirely.
    pub present: bool,
    pub powered: bool,
    pub connected: Vec<Device>,
}

impl BtState {
    pub fn tooltip(&self) -> String {
        if !self.present {
            return "No Bluetooth adapter".to_string();
        }
        if !self.powered {
            return "Bluetooth off".to_string();
        }
        if self.connected.is_empty() {
            return "Bluetooth on — nothing connected".to_string();
        }
        let devices: Vec<String> = self
            .connected
            .iter()
            .map(|d| match d.battery {
                Some(b) => format!("{} ({b}%)", d.name),
                None => d.name.clone(),
            })
            .collect();
        format!("Bluetooth — {}", devices.join(", "))
    }

    /// What the glyph should show next to it: the battery of the first connected
    /// device that reports one. Headsets are the reason this module exists.
    pub fn badge(&self) -> Option<String> {
        self.badge_pct().map(|b| format!("{b}%"))
    }

    /// The same number unformatted, for the bar — which renders percentages
    /// through its own padded formatter so their width never changes.
    pub fn badge_pct(&self) -> Option<u32> {
        self.connected.iter().find_map(|d| d.battery)
    }

    /// A short name for whichever device the badge belongs to, e.g.
    /// `MX Master 3S` → `MX`.
    ///
    /// Bluetooth names are advertising copy — "WH-1000XM5", "MX Master 3S",
    /// "Jabra Elite 75t". The first word is what a person actually calls the
    /// thing, and it is the only part that fits beside a percentage.
    pub fn badge_name(&self) -> Option<String> {
        let d = self.connected.iter().find(|d| d.battery.is_some())?;
        let first = d.name.split_whitespace().next().unwrap_or(&d.name);
        (!first.is_empty()).then(|| first.chars().take(10).collect())
    }
}

pub fn poll() -> BtState {
    let Some(list) = run(&["list"]) else {
        return BtState::default();
    };
    let present = list.lines().any(|l| l.trim_start().starts_with("Controller"));
    if !present {
        return BtState::default();
    }
    let show = run(&["show"]).unwrap_or_default();
    let powered = yes_field(&show, "Powered");
    if !powered {
        return BtState { present: true, powered: false, connected: Vec::new() };
    }
    BtState { present: true, powered: true, connected: connected_devices() }
}

fn connected_devices() -> Vec<Device> {
    // `devices Connected` asks BlueZ for exactly the set we want, so the common
    // case (nothing connected) costs one command and no per-device `info` calls.
    let Some(list) = run(&["devices", "Connected"]) else { return Vec::new() };
    list.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ')?;
            let info = run(&["info", mac]).unwrap_or_default();
            Some(Device { name: name.trim().to_string(), battery: battery_percentage(&info) })
        })
        .collect()
}

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("bluetoothctl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn yes_field(text: &str, key: &str) -> bool {
    text.lines()
        .filter_map(|l| l.trim().split_once(": "))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.trim() == "yes")
        .unwrap_or(false)
}

/// `Battery Percentage: 0x5a (90)` → 90 — the decimal in parentheses, not the
/// hex prefix (which would report 90% as 5%).
fn battery_percentage(info: &str) -> Option<u32> {
    let line = info.lines().map(str::trim).find(|l| l.starts_with("Battery Percentage:"))?;
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    line[open + 1..close].trim().parse().ok()
}

/// `--bt-dump`: print what the module would show, with no window. Mirrors
/// `--camera-dump` / `--mic-dump`, which is how those two were verified without
/// launching a second bar onto the user's live desktop.
pub fn dump() {
    let s = poll();
    println!("{}", s.tooltip());
    if let Some(b) = s.badge() {
        println!("badge={b}");
    }
    println!("present={} powered={}", s.present, s.powered);
    for d in &s.connected {
        println!("device={} battery={:?}", d.name, d.battery);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_decimal_battery_not_the_hex_prefix() {
        assert_eq!(battery_percentage("\tBattery Percentage: 0x5a (90)"), Some(90));
        assert_eq!(battery_percentage("\tBattery Percentage: 0x64 (100)"), Some(100));
        assert_eq!(battery_percentage("\tConnected: yes"), None);
    }

    #[test]
    fn tooltip_names_every_connected_device_with_its_battery() {
        let s = BtState {
            present: true,
            powered: true,
            connected: vec![
                Device { name: "WH-1000XM4".into(), battery: Some(90) },
                Device { name: "MX Master".into(), battery: None },
            ],
        };
        assert_eq!(s.tooltip(), "Bluetooth — WH-1000XM4 (90%), MX Master");
        assert_eq!(s.badge().as_deref(), Some("90%"));
    }

    #[test]
    fn an_absent_adapter_is_a_distinct_state_from_a_powered_off_one() {
        let none = BtState::default();
        assert_eq!(none.tooltip(), "No Bluetooth adapter");
        assert_eq!(none.badge(), None);
        let off = BtState { present: true, powered: false, connected: vec![] };
        assert_eq!(off.tooltip(), "Bluetooth off");
    }
}
