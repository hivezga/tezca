//! `tezca display` — per-monitor mode / scale / position + hardware brightness.
//!
//!   list [--machine]   enumerate monitors (current mode, scale, available modes)
//!   set <name> [--mode WxH@R] [--scale S] [--pos XxY] [--transform N]
//!   reset <name>       drop the monitor override and reload
//!   brightness <name> [0-100]   read / set DDC/CI brightness (external monitors)
//!
//! Mode/scale/pos changes apply live via `hyprctl eval 'hl.monitor{…}'` and persist
//! in ~/.config/tezca/overrides.lua (survive reload/relogin). A bad mode is
//! always recoverable with `hyprctl reload` or `tezca display reset <name>`.
//!
//! Brightness uses `ddcutil` (DDC/CI over the monitor's i2c bus) since desktop
//! monitors have no backlight sysfs. The Hyprland output name → i2c bus mapping
//! is cached at ~/.cache/tezca/ddc.map so we skip a slow `ddcutil detect` on
//! every call.

use crate::{hypr, managed, term, util, validate};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// Monitor model + `hyprctl monitors` parsing
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Monitor {
    name: String,
    desc: String,
    res: String,   // "3440x1440"
    rate: String,  // "165.00"
    pos: String,   // "0x0"
    scale: String, // "1.00"
    transform: String,
    /// As the compositor reports it: a **bool**, i.e. whether adaptive sync is
    /// engaged right now — not the 0/1/2 mode that was configured. The configured
    /// value lives in the override store and is what `config` prints.
    vrr: String,
    /// Derived from `currentFormat`, the only place bit depth is observable:
    /// `XRGB2101010` is 10-bit, anything else 8.
    bitdepth: String,
    /// From `mirrorOf`, empty when "none".
    mirror: String,
    modes: Vec<String>, // "3440x1440@165.00"
    disabled: bool,
}

/// Parse plain `hyprctl monitors` into structured monitors.
fn parse_monitors(text: &str) -> Vec<Monitor> {
    let mut mons: Vec<Monitor> = Vec::new();
    let mut cur: Option<Monitor> = None;
    let mut saw_mode = false;

    for raw in text.lines() {
        // "Monitor DP-1 (ID 0):"
        if let Some(rest) = raw.strip_prefix("Monitor ") {
            if let Some(m) = cur.take() {
                mons.push(m);
            }
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            cur = Some(Monitor { name, ..Default::default() });
            saw_mode = false;
            continue;
        }
        let Some(m) = cur.as_mut() else { continue };
        let line = raw.trim();

        // The first indented line is the active mode: "3440x1440@165.00000 at 0x0".
        if !saw_mode && line.contains('@') && line.contains(" at ") {
            if let Some((mode, pos)) = line.split_once(" at ") {
                if let Some((res, rate)) = mode.split_once('@') {
                    m.res = res.trim().to_string();
                    m.rate = fmt_rate(rate.trim());
                }
                m.pos = pos.trim().to_string();
            }
            saw_mode = true;
            continue;
        }
        if let Some(v) = line.strip_prefix("description: ") {
            m.desc = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("scale: ") {
            m.scale = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("transform: ") {
            m.transform = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("vrr: ") {
            m.vrr = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("disabled: ") {
            m.disabled = v.trim() == "true";
        } else if let Some(v) = line.strip_prefix("currentFormat: ") {
            // XRGB2101010 / XBGR2101010 → 10 bits per channel; everything else 8.
            m.bitdepth = if v.contains("2101010") { "10".into() } else { "8".into() };
        } else if let Some(v) = line.strip_prefix("mirrorOf: ") {
            let v = v.trim();
            m.mirror = if v == "none" { String::new() } else { v.to_string() };
        } else if let Some(v) = line.strip_prefix("availableModes: ") {
            m.modes = v
                .split_whitespace()
                .filter_map(|tok| {
                    let (res, rate) = tok.split_once('@')?;
                    Some(format!("{res}@{}", fmt_rate(rate.trim_end_matches("Hz"))))
                })
                .collect();
            dedup_keep_order(&mut m.modes);
        }
    }
    if let Some(m) = cur.take() {
        mons.push(m);
    }
    mons
}

/// Normalize a refresh rate to two decimals: "165.00000" / "165" → "165.00".
fn fmt_rate(s: &str) -> String {
    match s.parse::<f64>() {
        Ok(n) => format!("{n:.2}"),
        Err(_) => s.to_string(),
    }
}

fn dedup_keep_order(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
}

/// `all = true` adds the outputs Hyprland is currently *not* driving — the only
/// way to see a monitor you switched off, and therefore the only way to switch it
/// back on from a GUI.
fn monitors_opt(all: bool) -> Result<Vec<Monitor>, String> {
    let mut cmd = Command::new("hyprctl");
    cmd.arg("monitors");
    if all {
        cmd.arg("all");
    }
    let out = cmd.output().map_err(|e| format!("failed to run hyprctl: {e}"))?;
    if !out.status.success() {
        return Err("hyprctl monitors failed".into());
    }
    Ok(parse_monitors(&String::from_utf8_lossy(&out.stdout)))
}

fn monitors() -> Result<Vec<Monitor>, String> {
    monitors_opt(true)
}

fn find<'a>(mons: &'a [Monitor], name: &str) -> Option<&'a Monitor> {
    mons.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("list") => cmd_list(args.get(1..).unwrap_or(&[])),
        Some("set") => cmd_set(&args[1..]),
        Some("reset") => cmd_reset(&args[1..]),
        Some("config") => cmd_config(),
        Some("profile") => cmd_profile(&args[1..]),
        Some("brightness") | Some("bri") => cmd_brightness(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown display subcommand: {other}\n  try: list · set <name> … · reset <name> · config · profile … · brightness <name> [0-100]"
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

fn cmd_list(args: &[&str]) -> Result<(), String> {
    let mons = monitors()?;
    let machine = args.iter().any(|a| *a == "--machine" || *a == "-m");
    // Disabled outputs are hidden unless asked for: `list` answers "what am I
    // looking at", and a switched-off monitor is not that. The GUI passes --all
    // because it also has to offer switching one back on.
    let all = args.iter().any(|a| *a == "--all" || *a == "-a");
    let shown = mons.iter().filter(|m| all || !m.disabled);

    if machine {
        for m in shown {
            println!("@monitor {}", m.name);
            println!("desc={}", m.desc);
            println!("res={}", m.res);
            println!("rate={}", m.rate);
            println!("pos={}", m.pos);
            println!("scale={}", m.scale);
            println!("transform={}", m.transform);
            println!("vrr={}", m.vrr);
            println!("bitdepth={}", m.bitdepth);
            println!("mirror={}", m.mirror);
            println!("disabled={}", m.disabled);
            println!("modes={}", m.modes.join(" "));
        }
        return Ok(());
    }
    println!("{}", term::header("tezca display"));
    println!();
    for m in shown {
        let dot = if m.disabled { term::dim("○") } else { term::green("●") };
        let summary = if m.disabled {
            "disabled".to_string()
        } else {
            let mut s = format!("{}@{} @ {}  scale {}", m.res, m.rate, m.pos, m.scale);
            if m.transform != "0" && !m.transform.is_empty() {
                s.push_str(&format!("  transform {}", m.transform));
            }
            if m.vrr == "true" {
                s.push_str("  vrr");
            }
            if m.bitdepth == "10" {
                s.push_str("  10-bit");
            }
            if !m.mirror.is_empty() {
                s.push_str(&format!("  mirror of {}", m.mirror));
            }
            s
        };
        println!("  {} {}  {}", dot, term::bold(&m.name), term::dim(&summary));
        println!("    {}", term::dim(&m.desc));
    }
    println!();
    Ok(())
}

/// `tezca display config` — the *persisted* per-monitor overrides, as
/// `monitor:<NAME>.<field> = <value>` pairs.
///
/// This exists because two settings cannot be read back off the compositor:
/// `hyprctl monitors` reports VRR as a bool (engaged right now, not the mode that
/// was asked for) and never reports the requested bit depth at all. A GUI seeded
/// from the compositor would therefore show "VRR off" for a fullscreen-only
/// monitor that simply had no fullscreen window at the time, and switch it off on
/// the next Apply.
fn cmd_config() -> Result<(), String> {
    for m in managed::monitors() {
        let name = &m.output;
        println!("monitor:{name}.mode = {}", m.mode);
        println!("monitor:{name}.position = {}", m.position);
        println!("monitor:{name}.scale = {}", m.scale);
        println!("monitor:{name}.transform = {}", m.transform);
        println!("monitor:{name}.vrr = {}", m.vrr);
        println!("monitor:{name}.bitdepth = {}", m.bitdepth);
        println!("monitor:{name}.mirror = {}", m.mirror);
        println!("monitor:{name}.disabled = {}", m.disabled);
    }
    Ok(())
}

/// `tezca display set <name> [--mode WxH@R] [--scale S] [--pos XxY] [--transform N]
///                           [--vrr off|on|fullscreen] [--bitdepth 8|10]
///                           [--mirror NAME|off] [--off|--on]
///                           [--right-of NAME|--left-of NAME|--above NAME|--below NAME]`
fn cmd_set(args: &[&str]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .copied()
        .ok_or("usage: tezca display set <name> [--mode WxH@R] [--scale S] [--pos XxY]")?;
    if !hypr::in_session() {
        return Err("not in a Hyprland session (nothing to apply)".into());
    }

    let mons = monitors()?;
    let cur = find(&mons, name).ok_or_else(|| format!("no monitor named '{name}'"))?;
    // Anything already persisted wins over the live reading for the two fields
    // the compositor cannot report faithfully — otherwise `set --scale 1.25` on a
    // fullscreen-only-VRR monitor would quietly demote it to plain "on".
    let stored = managed::monitors().into_iter().find(|m| m.output == name);

    // Start from the current values so unspecified fields are preserved verbatim.
    let mut mode = format!("{}@{}", cur.res, cur.rate);
    let mut scale = cur.scale.clone();
    let mut pos = cur.pos.clone();
    let mut transform = cur.transform.clone();
    let mut vrr = stored.as_ref().map(|m| m.vrr.clone()).unwrap_or_default();
    let mut bitdepth = stored.as_ref().map(|m| m.bitdepth.clone()).unwrap_or_default();
    let mut mirror = stored.as_ref().map(|m| m.mirror.clone()).unwrap_or_else(|| cur.mirror.clone());
    let mut disabled = cur.disabled;

    let mut it = args[1..].iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--mode" => mode = it.next().ok_or("--mode needs a value like 3440x1440@165")?.to_string(),
            "--scale" => scale = it.next().ok_or("--scale needs a value")?.to_string(),
            "--pos" => pos = it.next().ok_or("--pos needs a value like 0x0")?.to_string(),
            "--transform" => transform = it.next().ok_or("--transform needs 0-7")?.to_string(),
            "--vrr" => vrr = vrr_value(it.next().ok_or("--vrr needs off|on|fullscreen")?)?,
            "--bitdepth" => bitdepth = it.next().ok_or("--bitdepth needs 8 or 10")?.to_string(),
            "--mirror" => {
                let v = it.next().ok_or("--mirror needs a monitor name (or 'off')")?;
                mirror = if v == "off" || v == "none" { String::new() } else { v.to_string() };
            }
            "--off" => disabled = true,
            "--on" => disabled = false,
            "--right-of" | "--left-of" | "--above" | "--below" => {
                let anchor = it.next().ok_or_else(|| format!("{a} needs a monitor name"))?;
                pos = place_relative(&mons, name, anchor, a)?;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    // Every field below is formatted verbatim into a `monitor = …` line in the
    // managed block, so check each one before building the spec. A comma would
    // shift the fields after it; a newline would append a whole extra directive
    // that `tezca display reset` could not remove, because the block keys an
    // entry by its first line only. Typos are caught here too, rather than at the
    // next relogin.
    validate::monitor_name(name)?;
    validate::display_mode(&mode)?;
    validate::display_pos(&pos)?;
    validate::display_scale(&scale)?;
    validate::display_transform(&transform)?;
    validate::display_vrr(&vrr)?;
    validate::display_bitdepth(&bitdepth)?;
    if !mirror.is_empty() {
        validate::monitor_name(&mirror)?;
        if mirror == name {
            return Err(format!("'{name}' cannot mirror itself"));
        }
        if find(&mons, &mirror).is_none() {
            return Err(format!("cannot mirror '{mirror}': no such monitor is connected"));
        }
    }

    // The one change that can end the session: switching off the only screen you
    // have left leaves no way back except a TTY. Checked here rather than in the
    // GUI so `tezca display set DP-1 --off` over SSH is just as safe.
    if disabled && !cur.disabled {
        let others_live = mons.iter().any(|m| m.name != name && !m.disabled);
        if !others_live {
            return Err(format!(
                "refusing to disable '{name}': it is the only enabled monitor, and turning it \
                 off would leave the session with no display"
            ));
        }
    }

    let m = managed::Monitor {
        output: name.to_string(),
        mode: mode.clone(),
        position: pos.clone(),
        scale: scale.clone(),
        transform: transform.clone(),
        vrr: vrr.clone(),
        bitdepth: bitdepth.clone(),
        mirror: mirror.clone(),
        disabled,
    };

    hypr::set_monitor(&m).map_err(|e| format!("hyprctl eval monitor: {e}"))?;
    managed::set_monitor(m)?;

    let mut spec = format!("{name},{mode},{pos},{scale}");
    for (label, v) in [("transform", &transform), ("vrr", &vrr), ("bitdepth", &bitdepth)] {
        let is_default_transform = label == "transform" && v == "0";
        if !v.is_empty() && !is_default_transform {
            spec.push_str(&format!(",{label},{v}"));
        }
    }
    if !mirror.is_empty() {
        spec.push_str(&format!(",mirror,{mirror}"));
    }
    if disabled {
        spec.push_str(",disabled");
    }
    println!("  {} {name}  {}", term::green("✓"), term::dim(&spec));

    // 10-bit is the one setting that can be accepted and then silently not taken:
    // the link may not have the bandwidth for it at this mode and refresh rate.
    // Say so rather than leaving a switch on that did nothing.
    if bitdepth == "10" {
        if let Some(live) = monitors().ok().and_then(|ms| find(&ms, name).cloned()) {
            if live.bitdepth == "8" {
                println!(
                    "  {} 10-bit did not take — the output is still 8-bit. The link may not \
                     have the bandwidth at {mode}; try a lower refresh rate.",
                    term::yellow("!")
                );
            }
        }
    }
    Ok(())
}

/// `off|on|fullscreen` (and the raw 0/1/2) → the number Hyprland wants.
fn vrr_value(v: &str) -> Result<String, String> {
    Ok(match v.trim().to_lowercase().as_str() {
        "off" | "0" => "0".to_string(),
        "on" | "1" => "1".to_string(),
        "fullscreen" | "fullscreen-only" | "2" => "2".to_string(),
        "" | "inherit" | "default" => String::new(),
        other => {
            return Err(format!(
                "invalid vrr {other:?} — expected off, on, fullscreen (or inherit to unset)"
            ))
        }
    })
}

/// Position `name` beside `anchor`, in Hyprland's *logical* coordinate space
/// (i.e. pixels divided by scale — a 3440 px monitor at scale 1.25 is 2752 wide).
fn place_relative(mons: &[Monitor], name: &str, anchor: &str, side: &str) -> Result<String, String> {
    if anchor == name {
        return Err(format!("cannot place '{name}' relative to itself"));
    }
    let a = find(mons, anchor).ok_or_else(|| format!("no monitor named '{anchor}'"))?;
    let me = find(mons, name).ok_or_else(|| format!("no monitor named '{name}'"))?;
    let (ax, ay) = parse_pos(&a.pos).ok_or_else(|| format!("cannot read {anchor}'s position"))?;
    let (aw, ah) = logical_size(a).ok_or_else(|| format!("cannot read {anchor}'s size"))?;
    let (mw, mh) = logical_size(me).ok_or_else(|| format!("cannot read {name}'s size"))?;

    let (x, y) = match side {
        "--right-of" => (ax + aw, ay),
        "--left-of" => (ax - mw, ay),
        "--above" => (ax, ay - mh),
        "--below" => (ax, ay + ah),
        _ => return Err(format!("unknown placement {side}")),
    };
    Ok(format!("{x}x{y}"))
}

fn parse_pos(p: &str) -> Option<(i32, i32)> {
    let neg = p.starts_with('-');
    let body = p.strip_prefix('-').unwrap_or(p);
    let (x, y) = body.split_once('x')?;
    let x: i32 = x.parse().ok()?;
    Some((if neg { -x } else { x }, y.parse().ok()?))
}

/// Width/height after scaling, rounded — what Hyprland lays out with.
fn logical_size(m: &Monitor) -> Option<(i32, i32)> {
    let (w, h) = m.res.split_once('x')?;
    let w: f64 = w.parse().ok()?;
    let h: f64 = h.parse().ok()?;
    let s: f64 = m.scale.parse().unwrap_or(1.0);
    let s = if s > 0.0 { s } else { 1.0 };
    Some(((w / s).round() as i32, (h / s).round() as i32))
}

fn cmd_reset(args: &[&str]) -> Result<(), String> {
    let name = args.first().copied().ok_or("usage: tezca display reset <name>")?;
    validate::monitor_name(name)?;
    managed::remove(&format!("monitor:{name}"))?;
    if hypr::in_session() {
        hypr::reload()?;
    }
    println!("  {} reset {name} to the shipped config", term::green("✓"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Layout profiles
// ---------------------------------------------------------------------------

/// `tezca display profile save|apply|list|rm <name>`.
///
/// A profile is a snapshot of every monitor's full spec, so switching between
/// "both screens" and "just the ultrawide" (which games are happier with) is one
/// command instead of four. Stored as its own data table rather than inside
/// `overrides.lua`: Hyprland never reads this file, and mixing a catalogue of
/// inactive layouts into the file the compositor *does* read invites applying one
/// by accident.
fn cmd_profile(args: &[&str]) -> Result<(), String> {
    match args.first().copied() {
        Some("save") => {
            let name = args.get(1).copied().ok_or("usage: tezca display profile save <name>")?;
            validate::profile_name(name)?;
            let mons = monitors()?;
            if mons.is_empty() {
                return Err("no monitors to save".into());
            }
            let entries: Vec<managed::Monitor> = mons
                .iter()
                .map(|m| {
                    // Prefer what was configured over what is live, for the two
                    // fields the compositor cannot report back faithfully.
                    let stored = managed::monitors().into_iter().find(|s| s.output == m.name);
                    managed::Monitor {
                        output: m.name.clone(),
                        mode: format!("{}@{}", m.res, m.rate),
                        position: m.pos.clone(),
                        scale: m.scale.clone(),
                        transform: m.transform.clone(),
                        vrr: stored.as_ref().map(|s| s.vrr.clone()).unwrap_or_default(),
                        bitdepth: stored.as_ref().map(|s| s.bitdepth.clone()).unwrap_or_default(),
                        mirror: m.mirror.clone(),
                        disabled: m.disabled,
                    }
                })
                .collect();
            let n = entries.len();
            profiles::save(name, entries)?;
            println!("  {} saved profile {} ({n} monitor(s))", term::green("✓"), term::bold(name));
            Ok(())
        }
        Some("apply") => {
            let name = args.get(1).copied().ok_or("usage: tezca display profile apply <name>")?;
            validate::profile_name(name)?;
            if !hypr::in_session() {
                return Err("not in a Hyprland session (nothing to apply)".into());
            }
            let entries = profiles::load(name)?
                .ok_or_else(|| format!("no profile named '{name}' (see `tezca display profile list`)"))?;
            // Enable before disable: applying a profile that switches screens
            // around must never pass through a state with nothing enabled.
            let (on, off): (Vec<_>, Vec<_>) = entries.into_iter().partition(|m| !m.disabled);
            for m in on.into_iter().chain(off) {
                hypr::set_monitor(&m).map_err(|e| format!("hyprctl eval monitor: {e}"))?;
                managed::set_monitor(m)?;
            }
            println!("  {} applied profile {}", term::green("✓"), term::bold(name));
            Ok(())
        }
        Some("list") | None => {
            let names = profiles::names()?;
            if names.is_empty() {
                println!("  {}", term::dim("no saved display profiles"));
                return Ok(());
            }
            for n in names {
                println!("{n}");
            }
            Ok(())
        }
        Some("rm") | Some("remove") | Some("delete") => {
            let name = args.get(1).copied().ok_or("usage: tezca display profile rm <name>")?;
            validate::profile_name(name)?;
            if !profiles::remove(name)? {
                return Err(format!("no profile named '{name}'"));
            }
            println!("  {} removed profile {name}", term::green("✓"));
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown profile subcommand: {other}\n  try: save <name> · apply <name> · list · rm <name>"
        )),
    }
}

/// The `~/.config/tezca/display-profiles.lua` store.
mod profiles {
    use crate::{atomic, managed, repo};
    use std::path::PathBuf;

    const HEADER: &str = "\
-- ~/.config/tezca/display-profiles.lua — generated by `tezca display profile`.
--
-- Saved monitor layouts. Hyprland does NOT read this file; `tezca display
-- profile apply <name>` replays the entries below through the same path as
-- `tezca display set`. Same shape as overrides.lua's `monitors` array, one array
-- per profile name.
";

    fn path() -> Result<PathBuf, String> {
        Ok(repo::config_home()?.join("tezca").join("display-profiles.lua"))
    }

    type Store = Vec<(String, Vec<managed::Monitor>)>;

    fn read() -> Result<Store, String> {
        let text = std::fs::read_to_string(path()?).unwrap_or_default();
        Ok(parse(&text))
    }

    pub fn names() -> Result<Vec<String>, String> {
        Ok(read()?.into_iter().map(|(n, _)| n).collect())
    }

    pub fn load(name: &str) -> Result<Option<Vec<managed::Monitor>>, String> {
        Ok(read()?.into_iter().find(|(n, _)| n == name).map(|(_, m)| m))
    }

    pub fn save(name: &str, entries: Vec<managed::Monitor>) -> Result<(), String> {
        let mut store = read()?;
        match store.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = entries,
            None => store.push((name.to_string(), entries)),
        }
        write(&store)
    }

    pub fn remove(name: &str) -> Result<bool, String> {
        let mut store = read()?;
        let before = store.len();
        store.retain(|(n, _)| n != name);
        if store.len() == before {
            return Ok(false);
        }
        write(&store)?;
        Ok(true)
    }

    fn write(store: &Store) -> Result<(), String> {
        atomic::write(&path()?, &render(store))
    }

    fn render(store: &Store) -> String {
        let mut s = String::from(HEADER);
        s.push_str("\nreturn {\n");
        for (name, entries) in store {
            s.push_str(&format!("    [\"{name}\"] = {{\n"));
            for m in entries {
                s.push_str(&format!("        {{ {} }},\n", managed::monitor_fields(m)));
            }
            s.push_str("    },\n");
        }
        s.push_str("}\n");
        s
    }

    /// Line scanner over the grammar [`render`] emits — same approach, and same
    /// reasoning, as the override store: we are the only writer.
    fn parse(text: &str) -> Store {
        let mut store: Store = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with("--") {
                continue;
            }
            if let Some(rest) = line.strip_prefix("[\"") {
                if let Some((name, _)) = rest.split_once("\"]") {
                    store.push((name.to_string(), Vec::new()));
                    continue;
                }
            }
            if line.starts_with('{') {
                if let Some(m) = managed::parse_monitor_entry(line) {
                    if let Some(last) = store.last_mut() {
                        last.1.push(m);
                    }
                }
            }
        }
        store
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn mon(output: &str, pos: &str) -> managed::Monitor {
            managed::Monitor {
                output: output.into(),
                mode: "3440x1440@165".into(),
                position: pos.into(),
                scale: "1".into(),
                ..Default::default()
            }
        }

        #[test]
        fn round_trips_multiple_profiles() {
            let store: Store = vec![
                ("dual".to_string(), vec![mon("DP-1", "0x0"), mon("DP-3", "3440x0")]),
                (
                    "solo".to_string(),
                    vec![
                        mon("DP-1", "0x0"),
                        managed::Monitor { disabled: true, ..mon("DP-3", "3440x0") },
                    ],
                ),
            ];
            let text = render(&store);
            assert_eq!(parse(&text), store);
            // Stable: rewriting what we read produces the same bytes.
            assert_eq!(render(&parse(&text)), text);
        }

        #[test]
        fn an_absent_file_is_an_empty_catalogue_not_an_error() {
            assert!(parse("").is_empty());
        }
    }
}

// ---------------------------------------------------------------------------
// Brightness (ddcutil / DDC-CI)
// ---------------------------------------------------------------------------

fn cmd_brightness(args: &[&str]) -> Result<(), String> {
    if !util::has("ddcutil") {
        return Err("ddcutil not found — install it for external-monitor brightness (`paru -S ddcutil`)".into());
    }
    // `brightness` / `brightness list` → print NAME=VALUE for every DDC monitor.
    if args.is_empty() || args[0] == "list" {
        let map = ddc_map(false)?;
        for (name, bus) in &map {
            if let Some(v) = ddc_get(*bus) {
                println!("{name}={v}");
            }
        }
        return Ok(());
    }

    let refresh = args.contains(&"--refresh");
    let positional: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
    let name = positional.first().ok_or("usage: tezca display brightness <name> [0-100]")?;

    let map = ddc_map(refresh)?;
    let bus = map
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| *b)
        .ok_or_else(|| format!("'{name}' has no DDC/CI bus (not a DDC-capable monitor?)"))?;

    match positional.get(1) {
        // Read.
        None => {
            let v = ddc_get(bus).ok_or("could not read brightness")?;
            println!("{v}");
            Ok(())
        }
        // Set.
        Some(val) => {
            let n: i32 = val.parse().map_err(|_| "brightness must be an integer 0-100")?;
            let n = n.clamp(0, 100);
            let out = Command::new("ddcutil")
                .args(["setvcp", "10", &n.to_string(), "--bus", &bus.to_string()])
                .output()
                .map_err(|e| format!("ddcutil setvcp: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
    }
}

/// Read VCP 0x10 (brightness) on a bus → current value, or None.
/// `ddcutil getvcp 10 --bus N --brief` → "VCP 10 C <current> <max>".
fn ddc_get(bus: u32) -> Option<String> {
    let out = Command::new("ddcutil")
        .args(["getvcp", "10", "--bus", &bus.to_string(), "--brief"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let cur = s.split_whitespace().nth(3)?;
    cur.parse::<i32>().ok().map(|n| n.to_string())
}

/// Hyprland output name → i2c bus number, cached at ~/.cache/tezca/ddc.map.
/// Rebuilt (via `ddcutil detect`) when the cache is missing or `refresh`.
fn ddc_map(refresh: bool) -> Result<Vec<(String, u32)>, String> {
    let cache = cache_path()?;
    if !refresh {
        if let Ok(text) = fs::read_to_string(&cache) {
            let map = parse_ddc_map(&text);
            if !map.is_empty() {
                return Ok(map);
            }
        }
    }
    let out = Command::new("ddcutil")
        .args(["detect", "--brief"])
        .output()
        .map_err(|e| format!("ddcutil detect: {e}"))?;
    if !out.status.success() {
        return Err("ddcutil detect failed (check i2c permissions / DDC support)".into());
    }
    let map = parse_ddc_detect(&String::from_utf8_lossy(&out.stdout));
    // Persist the cache (best-effort).
    if let Some(dir) = cache.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let body: String = map.iter().map(|(n, b)| format!("{n}\t{b}\n")).collect();
    let _ = fs::write(&cache, body);
    Ok(map)
}

fn parse_ddc_detect(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut bus: Option<u32> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("Display ") {
            bus = None;
        } else if let Some(rest) = l.strip_prefix("I2C bus:") {
            // "/dev/i2c-3" → 3
            bus = rest
                .trim()
                .rsplit('-')
                .next()
                .and_then(|n| n.trim().parse().ok());
        } else if let Some(rest) = l.strip_prefix("DRM connector:") {
            // "card1-DP-1" → "DP-1"
            if let Some(b) = bus {
                let conn = rest.trim();
                if let Some((_, name)) = conn.split_once('-') {
                    out.push((name.to_string(), b));
                }
            }
        }
    }
    out
}

fn parse_ddc_map(text: &str) -> Vec<(String, u32)> {
    text.lines()
        .filter_map(|l| {
            let (n, b) = l.split_once('\t')?;
            Some((n.trim().to_string(), b.trim().parse().ok()?))
        })
        .collect()
}

fn cache_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("neither $XDG_CACHE_HOME nor $HOME is set")?;
    Ok(base.join("tezca").join("ddc.map"))
}


fn print_help() {
    println!("{}", term::header("tezca display"));
    println!();
    println!("  {}                 list monitors + available modes ({})", term::cyan("list"), term::dim("--all for disabled"));
    println!("  {}  set mode/scale/position (live + persisted)", term::cyan("set <name> --mode WxH@R"));
    println!("  {}          revert a monitor to the shipped config", term::cyan("reset <name>"));
    println!("  {}               the persisted per-monitor overrides", term::cyan("config"));
    println!("  {}     save/apply/list/rm a monitor layout", term::cyan("profile <sub> [name]"));
    println!("  {}   read / set DDC/CI brightness (0-100)", term::cyan("brightness <name> [val]"));
    println!();
    println!("{}", term::bold("SET FLAGS"));
    for (f, d) in [
        ("--mode WxH@R", "resolution + refresh rate"),
        ("--scale S", "fractional scale, or 'auto'"),
        ("--pos XxY", "logical position"),
        ("--transform 0-7", "rotation / flip"),
        ("--vrr off|on|fullscreen", "per-monitor adaptive sync"),
        ("--bitdepth 8|10", "colour depth (10-bit needs link bandwidth)"),
        ("--mirror NAME|off", "clone another output"),
        ("--off / --on", "disable / enable the output"),
        ("--right-of NAME", "place beside another monitor (also --left-of/--above/--below)"),
    ] {
        println!("  {:<26} {}", term::cyan(f), term::dim(d));
    }
}
