//! `tezca audio` — output/input device switching and volume.
//!
//!   status [--machine]
//!   outputs [--machine] | inputs [--machine]
//!   set-output <name> | set-input <name>
//!   volume <N|+N|-N> [--input]
//!   mute on|off|toggle [--input]
//!
//! Devices come from `pactl` (PipeWire's PulseAudio interface) and volume from
//! `wpctl`, which is what the bar already reads — so the bar's numbers and these
//! always agree.
//!
//! ## Why the verbose output and not `-f json`
//!
//! `pactl -f json` would be easier to consume, but this crate carries no
//! dependencies (DESIGN.md §8) and hand-rolling a JSON parser to avoid a line
//! scanner is a poor trade. `pactl list sinks` is block-structured and stable;
//! [`parse_devices`] reads it with the same approach used for `hyprctl monitors`.
//!
//! ## Switching moves what is already playing
//!
//! Setting the default sink only affects *new* streams — so a naive
//! implementation changes the default and the music keeps coming out of the old
//! speakers, which reads as "it didn't work". Every existing stream is moved
//! across too.

use crate::{term, util};
use std::process::Command;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    /// The stable node name, e.g. `alsa_output.pci-0000_01_00.1.hdmi-stereo`.
    /// Volatile numeric ids are deliberately not used as handles: they renumber
    /// whenever something is plugged in.
    pub name: String,
    pub description: String,
    pub muted: bool,
    pub default: bool,
}

pub fn run(args: &[&str]) -> i32 {
    if !util::has("pactl") {
        eprintln!("{} pactl not found — install PipeWire's pulse interface", term::red("error:"));
        return 1;
    }
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(args.get(1..).unwrap_or(&[])),
        Some("outputs") | Some("sinks") => cmd_list(false, &args[1..]),
        Some("inputs") | Some("sources") => cmd_list(true, &args[1..]),
        Some("set-output") => cmd_set_default(false, &args[1..]),
        Some("set-input") => cmd_set_default(true, &args[1..]),
        Some("volume") | Some("vol") => cmd_volume(&args[1..]),
        Some("mute") => cmd_mute(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown audio subcommand: {other}\n  try: status · outputs · inputs · \
             set-output <name> · set-input <name> · volume <N> · mute toggle"
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

fn pactl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run pactl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "pactl failed".into() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn pactl_opt(args: &[&str]) -> Option<String> {
    pactl(args).ok()
}

/// Parse the block-structured `pactl list sinks|sources` output.
///
/// `default_name` marks the active device. Monitor sources (the loopback of an
/// output, used for screen recording) are dropped from the input list: they are
/// not microphones, and offering one as "your input device" is a support ticket.
fn parse_devices(text: &str, default_name: &str, drop_monitors: bool) -> Vec<Device> {
    let mut out: Vec<Device> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("Sink #") || line.starts_with("Source #") {
            out.push(Device::default());
            continue;
        }
        let Some(d) = out.last_mut() else { continue };
        if let Some(v) = line.strip_prefix("Name: ") {
            d.name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Description: ") {
            d.description = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Mute: ") {
            d.muted = v.trim() == "yes";
        }
    }
    out.retain(|d| {
        let is_monitor = drop_monitors && d.name.ends_with(".monitor");
        !d.name.is_empty() && !is_monitor
    });
    for d in &mut out {
        d.default = d.name == default_name;
        if d.description.is_empty() {
            d.description = d.name.clone();
        }
    }
    out
}

fn devices(input: bool) -> Vec<Device> {
    let (list, default) =
        if input { ("sources", "get-default-source") } else { ("sinks", "get-default-sink") };
    let text = pactl_opt(&["list", list]).unwrap_or_default();
    let def = pactl_opt(&[default]).unwrap_or_default().trim().to_string();
    parse_devices(&text, &def, input)
}

/// `wpctl get-volume <id>` → "Volume: 0.46 [MUTED]".
fn volume_of(id: &str) -> Option<(u32, bool)> {
    let out = Command::new("wpctl").args(["get-volume", id]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let muted = s.contains("[MUTED]");
    let vol = s.split_whitespace().find_map(|t| t.parse::<f64>().ok())?;
    Some(((vol * 100.0).round() as u32, muted))
}

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

fn cmd_status(args: &[&str]) -> Result<(), String> {
    let (ovol, omute) = volume_of(SINK).unwrap_or((0, true));
    let (ivol, imute) = volume_of(SOURCE).unwrap_or((0, true));
    let out_dev = devices(false).into_iter().find(|d| d.default).unwrap_or_default();
    let in_dev = devices(true).into_iter().find(|d| d.default).unwrap_or_default();

    if args.iter().any(|a| *a == "--machine" || *a == "-m") {
        println!("output={}", out_dev.description);
        println!("output_name={}", out_dev.name);
        println!("output_volume={ovol}");
        println!("output_muted={omute}");
        println!("input={}", in_dev.description);
        println!("input_name={}", in_dev.name);
        println!("input_volume={ivol}");
        println!("input_muted={imute}");
        return Ok(());
    }
    println!("{}", term::header("tezca audio"));
    println!();
    println!(
        "  {} {:<8} {}",
        term::green("●"),
        "output",
        term::dim(&format!(
            "{}  {}%{}",
            out_dev.description,
            ovol,
            if omute { " (muted)" } else { "" }
        ))
    );
    println!(
        "  {} {:<8} {}",
        term::green("●"),
        "input",
        term::dim(&format!(
            "{}  {}%{}",
            in_dev.description,
            ivol,
            if imute { " (muted)" } else { "" }
        ))
    );
    println!();
    Ok(())
}

fn cmd_list(input: bool, args: &[&str]) -> Result<(), String> {
    let ds = devices(input);
    if args.iter().any(|a| *a == "--machine" || *a == "-m") {
        for d in &ds {
            println!("@device");
            println!("name={}", d.name);
            println!("description={}", d.description);
            println!("default={}", d.default);
            println!("muted={}", d.muted);
        }
        return Ok(());
    }
    println!("{}", term::header(if input { "tezca audio inputs" } else { "tezca audio outputs" }));
    println!();
    if ds.is_empty() {
        println!("  {}", term::dim("no devices"));
    }
    for d in &ds {
        let dot = if d.default { term::green("●") } else { term::dim("○") };
        println!("  {dot} {}", term::bold(&d.description));
        println!("    {}", term::dim(&d.name));
    }
    println!();
    Ok(())
}

fn cmd_set_default(input: bool, args: &[&str]) -> Result<(), String> {
    let want = args.first().copied().ok_or_else(|| {
        format!("usage: tezca audio set-{} <name>", if input { "input" } else { "output" })
    })?;

    // Accept an exact node name or a unique description substring — the GUI sends
    // the former, a human typing at a prompt has only ever seen the latter.
    let ds = devices(input);
    let exact = ds.iter().find(|d| d.name == want);
    let dev = match exact {
        Some(d) => d,
        None => {
            let low = want.to_lowercase();
            let matches: Vec<&Device> =
                ds.iter().filter(|d| d.description.to_lowercase().contains(&low)).collect();
            match matches.len() {
                1 => matches[0],
                0 => return Err(format!("no audio device matching '{want}'")),
                n => return Err(format!("'{want}' matches {n} devices — use the exact name")),
            }
        }
    };

    if input {
        pactl(&["set-default-source", &dev.name])?;
        move_streams("source-outputs", "move-source-output", &dev.name);
    } else {
        pactl(&["set-default-sink", &dev.name])?;
        // Without this the default changes but whatever is already playing keeps
        // coming out of the old device.
        move_streams("sink-inputs", "move-sink-input", &dev.name);
    }
    println!("  {} {}", term::green("✓"), term::bold(&dev.description));
    Ok(())
}

/// Move every existing stream onto `target` (best-effort, per stream).
fn move_streams(list: &str, verb: &str, target: &str) {
    let Some(out) = pactl_opt(&["list", "short", list]) else { return };
    for line in out.lines() {
        let Some(id) = line.split_whitespace().next() else { continue };
        // A stream can refuse to move (it may be locked to a device); that is not
        // a reason to abandon the others.
        let _ = pactl(&[verb, id, target]);
    }
}

fn cmd_volume(args: &[&str]) -> Result<(), String> {
    let input = args.contains(&"--input");
    let id = if input { SOURCE } else { SINK };
    let v = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .copied()
        .ok_or("usage: tezca audio volume <N|+N|-N> [--input]")?;

    // wpctl speaks percentages with an explicit sign for relative moves.
    let arg = if let Some(rest) = v.strip_prefix('+') {
        format!("{}%+", parse_percent(rest)?)
    } else if let Some(rest) = v.strip_prefix('-') {
        format!("{}%-", parse_percent(rest)?)
    } else {
        format!("{}%", parse_percent(v)?)
    };
    let status = Command::new("wpctl")
        .args(["set-volume", "-l", "1.5", id, &arg])
        .status()
        .map_err(|e| format!("failed to run wpctl: {e}"))?;
    if !status.success() {
        return Err("could not set the volume".into());
    }
    let (vol, _) = volume_of(id).unwrap_or((0, false));
    println!("  {} {vol}%", term::green("✓"));
    Ok(())
}

fn parse_percent(v: &str) -> Result<u32, String> {
    v.trim_end_matches('%')
        .parse()
        .map_err(|_| format!("invalid volume {v:?} — expected a percentage like 40, +5 or -5"))
}

fn cmd_mute(args: &[&str]) -> Result<(), String> {
    let input = args.contains(&"--input");
    let id = if input { SOURCE } else { SINK };
    let arg = match args.iter().find(|a| !a.starts_with("--")).copied() {
        Some("on") => "1",
        Some("off") => "0",
        Some("toggle") | None => "toggle",
        Some(other) => return Err(format!("expected on, off or toggle — got {other:?}")),
    };
    let status = Command::new("wpctl")
        .args(["set-mute", id, arg])
        .status()
        .map_err(|e| format!("failed to run wpctl: {e}"))?;
    if !status.success() {
        return Err("could not change the mute state".into());
    }
    let (_, muted) = volume_of(id).unwrap_or((0, false));
    println!("  {} {}", term::green("✓"), if muted { "muted" } else { "unmuted" });
    Ok(())
}

fn print_help() {
    println!("{}", term::header("tezca audio"));
    println!();
    for (c, d) in [
        ("status", "current output/input, volume and mute"),
        ("outputs / inputs", "list devices (--machine for parsing)"),
        ("set-output <name>", "switch output and move playing streams"),
        ("set-input <name>", "switch the capture device"),
        ("volume <N|+N|-N>", "set or adjust volume (--input for the mic)"),
        ("mute on|off|toggle", "mute (--input for the mic)"),
    ] {
        println!("  {:<22} {}", term::cyan(c), term::dim(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = "Sink #52
\tState: RUNNING
\tName: alsa_output.pci-0000_01_00.1.hdmi-stereo
\tDescription: GA104 Digital Stereo (HDMI)
\tMute: no
Sink #61
\tState: SUSPENDED
\tName: alsa_output.usb-Focusrite_Scarlett.analog-stereo
\tDescription: Scarlett Solo Analog Stereo
\tMute: yes
";

    const SOURCES: &str = "Source #51
\tName: alsa_output.pci-0000_01_00.1.hdmi-stereo.monitor
\tDescription: Monitor of GA104 Digital Stereo (HDMI)
\tMute: no
Source #62
\tName: alsa_input.usb-Focusrite_Scarlett.analog-stereo
\tDescription: Scarlett Solo Analog Stereo
\tMute: no
";

    #[test]
    fn reads_devices_and_marks_the_default() {
        let ds = parse_devices(SINKS, "alsa_output.usb-Focusrite_Scarlett.analog-stereo", false);
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].description, "GA104 Digital Stereo (HDMI)");
        assert!(!ds[0].default);
        assert!(ds[1].default);
        assert!(ds[1].muted);
    }

    #[test]
    fn monitor_sources_are_not_offered_as_microphones() {
        // A ".monitor" source is the loopback of an output — it is what screen
        // recording captures, and picking it as "your microphone" records the
        // desktop instead of your voice.
        let ds = parse_devices(SOURCES, "", true);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].name, "alsa_input.usb-Focusrite_Scarlett.analog-stereo");
        // …but they are still visible when not filtered.
        assert_eq!(parse_devices(SOURCES, "", false).len(), 2);
    }

    #[test]
    fn a_device_with_no_description_falls_back_to_its_name() {
        let ds = parse_devices("Sink #1\n\tName: bare.sink\n", "", false);
        assert_eq!(ds[0].description, "bare.sink");
    }

    #[test]
    fn volume_arguments_accept_absolute_and_relative_forms() {
        assert_eq!(parse_percent("40").unwrap(), 40);
        assert_eq!(parse_percent("5%").unwrap(), 5);
        assert!(parse_percent("loud").is_err());
    }
}
