//! `tezca net` — Wi-Fi, VPN and airplane mode, over NetworkManager.
//!
//!   status [--machine]              the active connection
//!   list [--rescan] [--machine]     visible access points
//!   connect <SSID> [--password-stdin|--ask] [--hidden]
//!   disconnect [<iface>]            drop the current Wi-Fi connection
//!   forget <SSID>                   delete the saved profile
//!   radio [on|off|toggle]           the Wi-Fi radio
//!   airplane [on|off|toggle]        every radio, Bluetooth included
//!   vpn [list|up <name>|down <name>]
//!   edit                            hand off to nm-connection-editor
//!
//! Everything shells out to `nmcli`, which is the only supported way to drive
//! NetworkManager without a D-Bus client — and this crate is deliberately
//! dependency-free (DESIGN.md §8).
//!
//! ## Secrets never reach argv
//!
//! The obvious spelling, `nmcli device wifi connect SSID password <psk>`, puts
//! the pre-shared key in the process's command line, where every other process on
//! the machine can read it out of `/proc`. `connect --password-stdin` instead
//! runs `nmcli --ask` and writes the secret to the child's stdin, so it exists
//! only in a pipe. If NetworkManager ever stops accepting a piped answer, the
//! failure is loud (a prompt that reads EOF errors out) rather than silent.
//!
//! ## Terse output is escaped, and it matters
//!
//! `nmcli -t` separates fields with `:` and escapes any `:` or `\` *inside* a
//! value as `\:` / `\\`. An SSID containing a colon is not exotic, and splitting
//! naively both mangles the name and shifts every field after it. [`split_terse`]
//! is the one place that is handled.

use crate::{term, util, validate};
use std::io::Write;
use std::process::{Command, Stdio};

pub fn run(args: &[&str]) -> i32 {
    if !util::has("nmcli") {
        eprintln!(
            "{} nmcli not found — install NetworkManager (`paru -S networkmanager`)",
            term::red("error:")
        );
        return 1;
    }
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(args.get(1..).unwrap_or(&[])),
        Some("list") | Some("scan") => cmd_list(&args[1..]),
        Some("connect") => cmd_connect(&args[1..]),
        Some("disconnect") => cmd_disconnect(&args[1..]),
        Some("forget") => cmd_forget(&args[1..]),
        Some("radio") => cmd_radio(&args[1..]),
        Some("airplane") => cmd_airplane(&args[1..]),
        Some("vpn") => cmd_vpn(&args[1..]),
        Some("edit") => cmd_edit(),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown net subcommand: {other}\n  try: status · list · connect <ssid> · \
             disconnect · forget <ssid> · radio on|off · airplane on|off · vpn · edit"
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

// ---------------------------------------------------------------------------
// nmcli plumbing
// ---------------------------------------------------------------------------

fn nmcli(args: &[&str]) -> Result<String, String> {
    let out = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run nmcli: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        return Err(if msg.is_empty() { "nmcli failed".into() } else { msg });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Best-effort read: absent hardware and disabled radios are normal states, not
/// errors, and every caller here treats "no answer" as "nothing to show".
fn nmcli_opt(args: &[&str]) -> Option<String> {
    nmcli(args).ok()
}

/// Split one `nmcli -t` record into its fields, undoing the escaping as it goes.
///
/// `\:` and `\\` are literal characters inside a value; a bare `:` is a
/// separator. Getting this wrong corrupts any SSID containing a colon *and*
/// shifts every field after it, so signal strength and security end up read out
/// of the wrong columns.
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
    // A trailing backslash is malformed; keep it rather than dropping a character.
    if escaped {
        fields.last_mut().expect("always non-empty").push('\\');
    }
    fields
}

fn field(fields: &[String], i: usize) -> String {
    fields.get(i).cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Status {
    kind: String, // wifi | ethernet | (empty)
    device: String,
    ssid: String,
    signal: String,
    ip: String,
    gateway: String,
    dns: String,
    wifi_radio: bool,
    vpn: String,
}

fn read_status() -> Status {
    let mut s = Status { wifi_radio: wifi_radio_on(), ..Default::default() };

    // DEVICE:TYPE:STATE:CONNECTION — pick the first connected real device,
    // preferring ethernet, which is what a desktop is actually using when both
    // are up.
    let devices = nmcli_opt(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"])
        .unwrap_or_default();
    let mut best: Option<(String, String)> = None;
    for line in devices.lines() {
        let f = split_terse(line);
        let (dev, kind, state) = (field(&f, 0), field(&f, 1), field(&f, 2));
        if !state.starts_with("connected") || dev == "lo" {
            continue;
        }
        match kind.as_str() {
            "ethernet" => {
                best = Some((dev, kind));
                break;
            }
            "wifi" if best.is_none() => best = Some((dev, kind)),
            _ => {}
        }
    }
    if let Some((dev, kind)) = best {
        s.device = dev;
        s.kind = kind;
    }

    if s.kind == "wifi" {
        if let Some(out) = nmcli_opt(&["-t", "-f", "IN-USE,SIGNAL,SSID", "device", "wifi"]) {
            for line in out.lines() {
                let f = split_terse(line);
                if field(&f, 0) == "*" {
                    s.signal = field(&f, 1);
                    s.ssid = field(&f, 2);
                    break;
                }
            }
        }
    }

    if !s.device.is_empty() {
        if let Some(out) = nmcli_opt(&[
            "-t",
            "-f",
            "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
            "device",
            "show",
            &s.device,
        ]) {
            for line in out.lines() {
                let f = split_terse(line);
                let (k, v) = (field(&f, 0), field(&f, 1));
                let v = v.split('/').next().unwrap_or(&v).to_string();
                if k.starts_with("IP4.ADDRESS") && s.ip.is_empty() {
                    s.ip = v;
                } else if k.starts_with("IP4.GATEWAY") && s.gateway.is_empty() {
                    s.gateway = v;
                } else if k.starts_with("IP4.DNS") && s.dns.is_empty() {
                    s.dns = v;
                }
            }
        }
    }

    // An active VPN is a separate "device" of type tun/wireguard; read it off the
    // connection list instead, which names it.
    if let Some(out) = nmcli_opt(&["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"]) {
        for line in out.lines() {
            let f = split_terse(line);
            let ty = field(&f, 1);
            if field(&f, 2) == "yes" && (ty.contains("vpn") || ty.contains("wireguard")) {
                s.vpn = field(&f, 0);
                break;
            }
        }
    }
    s
}

fn cmd_status(args: &[&str]) -> Result<(), String> {
    let s = read_status();
    if args.iter().any(|a| *a == "--machine" || *a == "-m") {
        println!("kind={}", s.kind);
        println!("device={}", s.device);
        println!("ssid={}", s.ssid);
        println!("signal={}", s.signal);
        println!("ip={}", s.ip);
        println!("gateway={}", s.gateway);
        println!("dns={}", s.dns);
        println!("wifi_radio={}", s.wifi_radio);
        println!("vpn={}", s.vpn);
        return Ok(());
    }
    println!("{}", term::header("tezca net"));
    println!();
    match s.kind.as_str() {
        "" => println!("  {} {}", term::dim("○"), term::bold("disconnected")),
        "wifi" => println!(
            "  {} {}  {}",
            term::green("●"),
            term::bold(&s.ssid),
            term::dim(&format!("{} · {}% · {}", s.device, s.signal, s.ip))
        ),
        _ => println!(
            "  {} {}  {}",
            term::green("●"),
            term::bold("wired"),
            term::dim(&format!("{} · {}", s.device, s.ip))
        ),
    }
    if !s.vpn.is_empty() {
        println!("  {} {}", term::cyan("vpn"), term::dim(&s.vpn));
    }
    println!(
        "  {} {}",
        term::dim("wifi radio"),
        if s.wifi_radio { term::green("on") } else { term::dim("off") }
    );
    println!();
    Ok(())
}

fn wifi_radio_on() -> bool {
    nmcli_opt(&["radio", "wifi"]).map(|s| s.trim() == "enabled").unwrap_or(false)
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

struct Ap {
    ssid: String,
    signal: u32,
    security: String,
    active: bool,
    saved: bool,
}

fn saved_wifi_names() -> Vec<String> {
    let Some(out) = nmcli_opt(&["-t", "-f", "NAME,TYPE", "connection", "show"]) else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|l| {
            let f = split_terse(l);
            (field(&f, 1) == "802-11-wireless").then(|| field(&f, 0))
        })
        .collect()
}

fn access_points(rescan: bool) -> Result<Vec<Ap>, String> {
    let saved = saved_wifi_names();
    // `--rescan yes` re-probes the air and takes seconds; the default reuses
    // NetworkManager's cache and returns immediately. Callers on a UI thread want
    // the cache first and a rescan in the background.
    let out = nmcli(&[
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL,SECURITY",
        "device",
        "wifi",
        "list",
        "--rescan",
        if rescan { "yes" } else { "no" },
    ])?;

    let mut aps: Vec<Ap> = Vec::new();
    for line in out.lines() {
        let f = split_terse(line);
        let ssid = field(&f, 1);
        // A hidden network reports no SSID. There is nothing to click on, and
        // `connect --hidden` is the way in, so leave it out of the list.
        if ssid.trim().is_empty() {
            continue;
        }
        let ap = Ap {
            active: field(&f, 0) == "*",
            signal: field(&f, 2).parse().unwrap_or(0),
            security: {
                let s = field(&f, 3);
                if s.trim().is_empty() { "open".to_string() } else { s }
            },
            saved: saved.contains(&ssid),
            ssid,
        };
        // The same network usually answers from several radios/BSSIDs. Keep the
        // strongest sighting rather than listing a name three times.
        match aps.iter_mut().find(|e| e.ssid == ap.ssid) {
            Some(existing) => {
                if ap.signal > existing.signal {
                    existing.signal = ap.signal;
                }
                existing.active |= ap.active;
            }
            None => aps.push(ap),
        }
    }
    aps.sort_by(|a, b| b.active.cmp(&a.active).then(b.signal.cmp(&a.signal)));
    Ok(aps)
}

fn cmd_list(args: &[&str]) -> Result<(), String> {
    let rescan = args.iter().any(|a| *a == "--rescan" || *a == "-r");
    let machine = args.iter().any(|a| *a == "--machine" || *a == "-m");
    if !wifi_radio_on() {
        if machine {
            return Ok(());
        }
        println!("  {}", term::dim("wifi radio is off — `tezca net radio on`"));
        return Ok(());
    }
    let aps = access_points(rescan)?;
    if machine {
        for a in &aps {
            println!("@ap");
            println!("ssid={}", a.ssid);
            println!("signal={}", a.signal);
            println!("security={}", a.security);
            println!("saved={}", a.saved);
            println!("active={}", a.active);
        }
        return Ok(());
    }
    println!("{}", term::header("tezca net"));
    println!();
    if aps.is_empty() {
        println!("  {}", term::dim("no networks in range"));
    }
    for a in &aps {
        let dot = if a.active { term::green("●") } else { term::dim("○") };
        let mut tags: Vec<String> = vec![format!("{}%", a.signal)];
        if a.security != "open" {
            tags.push(a.security.clone());
        }
        if a.saved {
            tags.push("saved".into());
        }
        println!("  {} {:<28} {}", dot, term::bold(&a.ssid), term::dim(&tags.join(" · ")));
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// connect / disconnect / forget
// ---------------------------------------------------------------------------

fn cmd_connect(args: &[&str]) -> Result<(), String> {
    let ssid = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .ok_or("usage: tezca net connect <SSID> [--password-stdin] [--hidden]")?;
    validate::ssid(ssid)?;
    let hidden = args.contains(&"--hidden");
    let from_stdin = args.contains(&"--password-stdin");
    let ask = args.contains(&"--ask");

    // A network we already have a profile for needs no secret at all.
    if !from_stdin && !ask && saved_wifi_names().iter().any(|n| n == ssid) {
        nmcli(&["connection", "up", "id", ssid])?;
        println!("  {} connected to {}", term::green("✓"), term::bold(ssid));
        return Ok(());
    }

    let mut cmd = Command::new("nmcli");
    if from_stdin || ask {
        cmd.arg("--ask");
    }
    cmd.args(["device", "wifi", "connect", ssid]);
    if hidden {
        cmd.args(["hidden", "yes"]);
    }

    if from_stdin {
        // Read the secret from OUR stdin and hand it to nmcli's, so it never
        // appears in either process's command line.
        let mut secret = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut secret)
            .map_err(|e| format!("could not read the password from stdin: {e}"))?;
        if !secret.ends_with('\n') {
            secret.push('\n');
        }
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("failed to run nmcli: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("nmcli did not accept a stdin pipe")?
            .write_all(secret.as_bytes())
            .map_err(|e| format!("could not send the password to nmcli: {e}"))?;
        let out = child.wait().map_err(|e| format!("nmcli did not finish: {e}"))?;
        if !out.success() {
            return Err(format!(
                "could not connect to '{ssid}' — wrong password, or the network is out of range"
            ));
        }
    } else {
        let out = cmd.status().map_err(|e| format!("failed to run nmcli: {e}"))?;
        if !out.success() {
            return Err(format!("could not connect to '{ssid}'"));
        }
    }
    println!("  {} connected to {}", term::green("✓"), term::bold(ssid));
    Ok(())
}

fn cmd_disconnect(args: &[&str]) -> Result<(), String> {
    let dev = match args.first().copied() {
        Some(d) => d.to_string(),
        None => wifi_device().ok_or("no Wi-Fi device to disconnect")?,
    };
    nmcli(&["device", "disconnect", &dev])?;
    println!("  {} disconnected {dev}", term::green("✓"));
    Ok(())
}

fn wifi_device() -> Option<String> {
    let out = nmcli_opt(&["-t", "-f", "DEVICE,TYPE", "device", "status"])?;
    out.lines().find_map(|l| {
        let f = split_terse(l);
        (field(&f, 1) == "wifi").then(|| field(&f, 0))
    })
}

fn cmd_forget(args: &[&str]) -> Result<(), String> {
    let ssid = args.first().copied().ok_or("usage: tezca net forget <SSID>")?;
    validate::ssid(ssid)?;
    nmcli(&["connection", "delete", "id", ssid])?;
    println!("  {} forgot {}", term::green("✓"), term::bold(ssid));
    Ok(())
}

// ---------------------------------------------------------------------------
// radio / airplane
// ---------------------------------------------------------------------------

fn cmd_radio(args: &[&str]) -> Result<(), String> {
    let want = toggle_arg(args.first().copied(), wifi_radio_on())?;
    nmcli(&["radio", "wifi", if want { "on" } else { "off" }])?;
    println!("  {} wifi radio {}", term::green("✓"), if want { "on" } else { "off" });
    Ok(())
}

/// Airplane mode is the inverse of "all radios on", which is why the argument is
/// negated here: `airplane on` means every radio off.
fn cmd_airplane(args: &[&str]) -> Result<(), String> {
    let all_on = nmcli_opt(&["radio", "all"]).map(|s| s.trim() == "enabled").unwrap_or(false);
    let want_airplane = toggle_arg(args.first().copied(), !all_on)?;
    nmcli(&["radio", "all", if want_airplane { "off" } else { "on" }])?;
    println!(
        "  {} airplane mode {}",
        term::green("✓"),
        if want_airplane { "on (wifi + bluetooth off)" } else { "off" }
    );
    Ok(())
}

/// `on` / `off` / `toggle` / nothing (which reports rather than changes).
fn toggle_arg(arg: Option<&str>, current: bool) -> Result<bool, String> {
    match arg {
        Some("on") | Some("enable") => Ok(true),
        Some("off") | Some("disable") => Ok(false),
        Some("toggle") | None => Ok(!current),
        Some(other) => Err(format!("expected on, off or toggle — got {other:?}")),
    }
}

// ---------------------------------------------------------------------------
// vpn
// ---------------------------------------------------------------------------

fn cmd_vpn(args: &[&str]) -> Result<(), String> {
    match args.first().copied() {
        None | Some("list") => {
            let machine = args.iter().any(|a| *a == "--machine" || *a == "-m");
            let out =
                nmcli_opt(&["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"]).unwrap_or_default();
            let mut any = false;
            for line in out.lines() {
                let f = split_terse(line);
                let ty = field(&f, 1);
                if !(ty.contains("vpn") || ty.contains("wireguard")) {
                    continue;
                }
                any = true;
                let (name, active) = (field(&f, 0), field(&f, 2) == "yes");
                if machine {
                    println!("@vpn");
                    println!("name={name}");
                    println!("active={active}");
                } else {
                    let dot = if active { term::green("●") } else { term::dim("○") };
                    println!("  {dot} {}", term::bold(&name));
                }
            }
            if !any && !machine {
                println!("  {}", term::dim("no VPN connections configured"));
            }
            Ok(())
        }
        Some("up") => {
            let name = args.get(1).copied().ok_or("usage: tezca net vpn up <name>")?;
            nmcli(&["connection", "up", "id", name])?;
            println!("  {} {name} up", term::green("✓"));
            Ok(())
        }
        Some("down") => {
            let name = args.get(1).copied().ok_or("usage: tezca net vpn down <name>")?;
            nmcli(&["connection", "down", "id", name])?;
            println!("  {} {name} down", term::green("✓"));
            Ok(())
        }
        Some(other) => Err(format!("unknown vpn subcommand: {other}\n  try: list · up <name> · down <name>")),
    }
}

fn cmd_edit() -> Result<(), String> {
    if !util::has("nm-connection-editor") {
        return Err(
            "nm-connection-editor not found (`paru -S nm-connection-editor`) — \
             or use `nmcli connection edit`"
                .into(),
        );
    }
    Command::new("nm-connection-editor")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch nm-connection-editor: {e}"))?;
    Ok(())
}

fn print_help() {
    println!("{}", term::header("tezca net"));
    println!();
    for (c, d) in [
        ("status", "the active connection (--machine for parsing)"),
        ("list [--rescan]", "visible access points"),
        ("connect <ssid>", "join a network (--password-stdin, --hidden)"),
        ("disconnect", "drop the current Wi-Fi connection"),
        ("forget <ssid>", "delete the saved profile"),
        ("radio on|off|toggle", "the Wi-Fi radio"),
        ("airplane on|off", "every radio, Bluetooth included"),
        ("vpn list|up|down", "VPN connections"),
        ("edit", "nm-connection-editor, for everything else"),
    ] {
        println!("  {:<22} {}", term::cyan(c), term::dim(d));
    }
    println!();
    println!("{}", term::dim("  Passwords are read from stdin, never from the command line:"));
    println!("{}", term::dim("    printf '%s\\n' \"$psk\" | tezca net connect MyWifi --password-stdin"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_terse_records_and_undoes_the_escaping() {
        // The plain case.
        assert_eq!(split_terse("*:MyNet:72:WPA2"), vec!["*", "MyNet", "72", "WPA2"]);
        // An SSID with a colon in it. Splitting naively would both mangle the
        // name and shift the signal and security fields one column left — this
        // is the bug the bar's own nmcli reader shipped with.
        assert_eq!(
            split_terse(r"*:My\:Net:72:WPA2"),
            vec!["*", "My:Net", "72", "WPA2"]
        );
        // A literal backslash.
        assert_eq!(split_terse(r"a\\b:c"), vec![r"a\b", "c"]);
        // Empty fields are real: an open network reports no security.
        assert_eq!(split_terse("*:Cafe:60:"), vec!["*", "Cafe", "60", ""]);
        assert_eq!(split_terse(""), vec![""]);
    }

    #[test]
    fn a_bssid_survives_being_read_as_a_field() {
        // Every colon in a MAC arrives escaped; this must come back as one field.
        let f = split_terse(r"AA\:BB\:CC\:DD\:EE\:FF:MyNet");
        assert_eq!(f, vec!["AA:BB:CC:DD:EE:FF", "MyNet"]);
    }

    #[test]
    fn toggle_argument_defaults_to_flipping_the_current_state() {
        assert!(!toggle_arg(None, true).unwrap());
        assert!(toggle_arg(None, false).unwrap());
        assert!(!toggle_arg(Some("toggle"), true).unwrap());
        assert!(toggle_arg(Some("on"), true).unwrap());
        assert!(!toggle_arg(Some("off"), false).unwrap());
        assert!(toggle_arg(Some("maybe"), false).is_err());
    }
}
