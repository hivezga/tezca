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

/// What the popover shows on top of [`poll`] — the paired-but-idle devices and
/// which adapter is doing the talking.
///
/// Deliberately *not* part of `poll`: this costs an extra `bluetoothctl` call
/// per paired device, which is fine once when a popover opens and far too much
/// on the module's five-second tick.
#[derive(Clone, Debug, Default)]
pub struct BtDetail {
    /// Paired devices that are not currently connected, with BlueZ's icon hint
    /// (`phone`, `audio-headset`, …) as the trailing note.
    pub paired: Vec<(String, String)>,
    /// Adapter alias and what it supports, e.g. `("tezca", "5.3")`.
    pub adapter: Option<(String, String)>,
}

pub fn detail() -> BtDetail {
    let show = run(&["show"]).unwrap_or_default();
    let adapter = adapter_of(&show);

    let connected: Vec<String> = run(&["devices", "Connected"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| Some(l.trim().strip_prefix("Device ")?.split_once(' ')?.0.to_string()))
        .collect();

    let paired = run(&["devices", "Paired"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (mac, name) = l.trim().strip_prefix("Device ")?.split_once(' ')?;
            if connected.iter().any(|c| c == mac) {
                return None;
            }
            let info = run(&["info", mac]).unwrap_or_default();
            Some((name.trim().to_string(), icon_of(&info)))
        })
        .collect();

    BtDetail { paired, adapter }
}

/// The adapter's alias and the Bluetooth revision it speaks.
///
/// Keyed off the `Controller` line so an absent adapter is `None` rather than a
/// row of blanks. The address is deliberately *not* shown — it is a stable
/// identifier for this machine, and the popover is a status readout, not an
/// inventory.
fn adapter_of(show: &str) -> Option<(String, String)> {
    show.lines().find_map(|l| l.trim().strip_prefix("Controller "))?;
    let field = |key: &str| {
        show.lines()
            .filter_map(|l| l.trim().split_once(": "))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.trim().to_string())
    };
    let alias = field("Alias").unwrap_or_else(|| "Adapter".to_string());
    let version = field("Version").and_then(|v| core_version(&v)).unwrap_or_default();
    Some((alias, version))
}

/// `0x0c (12)` → `5.3`.
///
/// BlueZ reports the raw HCI version number; the Bluetooth Core spec assigns
/// each one a revision, and the revision is the number anyone recognises.
/// Anything past the table is a spec newer than this code — reported as the
/// bare HCI number rather than guessed at.
fn core_version(field: &str) -> Option<String> {
    let open = field.find('(')?;
    let close = field[open..].find(')')? + open;
    let hci: u32 = field[open + 1..close].trim().parse().ok()?;
    Some(
        match hci {
            6 => "4.0",
            7 => "4.1",
            8 => "4.2",
            9 => "5.0",
            10 => "5.1",
            11 => "5.2",
            12 => "5.3",
            13 => "5.4",
            14 => "6.0",
            _ => return Some(format!("HCI {hci}")),
        }
        .to_string(),
    )
}

/// BlueZ's `Icon:` hint — the closest thing to a device *kind* it exposes.
fn icon_of(info: &str) -> String {
    info.lines()
        .filter_map(|l| l.trim().split_once(": "))
        .find(|(k, _)| *k == "Icon")
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_else(|| "paired".to_string())
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
    fn the_adapter_row_reads_the_alias_and_the_spec_revision() {
        // Shape taken from this machine's `bluetoothctl show`.
        let show = "Controller 08:71:90:80:D9:CC (public)\n\tManufacturer: 0x0002 (2)\n\t\
                    Version: 0x0a (10)\n\tName: tower\n\tAlias: Tezca\n\tPowered: yes\n";
        assert_eq!(adapter_of(show), Some(("Tezca".into(), "5.1".into())));
        assert_eq!(adapter_of("no controller here"), None);
    }

    #[test]
    fn an_hci_version_past_the_table_is_reported_rather_than_guessed() {
        assert_eq!(core_version("0x0c (12)").as_deref(), Some("5.3"));
        assert_eq!(core_version("0x63 (99)").as_deref(), Some("HCI 99"));
        assert_eq!(core_version("0x0a"), None);
    }

    #[test]
    fn a_paired_device_falls_back_to_a_generic_kind_when_bluez_offers_no_icon() {
        assert_eq!(icon_of("\tIcon: phone\n\tPaired: yes"), "phone");
        assert_eq!(icon_of("\tPaired: yes"), "paired");
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
