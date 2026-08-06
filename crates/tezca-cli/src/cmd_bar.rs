//! `tezca bar` — control the bespoke top menubar (crates/tezca-bar).
//!
//! The bar is a separate binary (`tezca-bar`) launched at login by
//! conf.d/autostart.lua. This subcommand is a thin lifecycle
//! wrapper — parallel to `tezca dock` — so you can start/stop/reload it by hand.
//! Live control uses signals: SIGUSR1 toggles visibility, SIGUSR2 reloads the
//! palette (sent by `tezca theme`).

use crate::{atomic, repo, term, util};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tezca_barlayout::{unknown_ids, Mod, Region};

const BIN: &str = "tezca-bar";

/// The bar's tunable keys and built-in defaults — mirrors
/// crates/tezca-bar/src/config.rs so `config` reports a complete picture even
/// when config.toml omits a field.
const SCALARS: &[(&str, &str)] = &[
    ("shape", "floating"),
    ("height", "40"),
    ("margin_top", "6"),
    ("margin_side", "10"),
    ("cpu_interval", "3"),
    ("mem_interval", "5"),
    ("gpu_interval", "3"),
    ("net_interval", "5"),
    ("clock_format", "%a %d %b   %H:%M"),
    // Extra zones for the clock popover: `Label=Area/City`, comma separated.
    ("clock_zones", ""),
    ("compact_width", "3000"),
    ("workspace_numerals", "arabic"),
    ("workspace_hide_empty", "false"),
    ("workspace_compact", "false"),
    // Volume on-screen display (the glass pill that flashes on a volume change).
    ("osd_enabled", "true"),
    ("osd_timeout_ms", "1400"),
    // Weather — opt-in, and the only other module that touches the network.
    ("weather_enabled", "false"),
    ("weather_lat", ""),
    ("weather_lon", ""),
    ("weather_place", ""),
    ("weather_interval", "900"),
    ("weather_unit", "celsius"),
    ("weather_aqi", "false"),
    // How the right cluster copes with its own length: all / grouped / hover /
    // tiers. See crates/tezca-bar/src/config.rs::Clutter.
    ("clutter", "all"),
];

/// Per-region module layout (ordered, comma-separated ids). Not in `SCALARS`
/// because the defaults come from `tezca-barlayout`, which the bar itself reads
/// too — the CLI used to keep its own copy of the right cluster and there is no
/// reason for two.
fn layout_scalars() -> Vec<(&'static str, &'static str)> {
    Region::ALL.into_iter().map(|r| (r.key(), r.default_layout())).collect()
}

/// Per-output workspace assignment keys look like `workspaces.<connector>` and
/// so can't live in the fixed `SCALARS` table; `set` accepts anything under
/// this prefix (e.g. `tezca bar set workspaces.DP-1 "1,3,5,7,9"`).
const WS_ASSIGN_PREFIX: &str = "workspaces.";

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(),
        Some("start") => cmd_start(),
        Some("stop") => cmd_stop(),
        Some("restart") => cmd_restart(),
        Some("toggle") => cmd_toggle(),
        Some("config") => cmd_config(),
        Some("modules") => cmd_modules(),
        Some("set") => cmd_set(&args[1..]),
        Some("unset") => cmd_unset(&args[1..]),
        Some("weather") => cmd_weather(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown bar subcommand: {other}\n  try: status · start · stop · restart · toggle · config · modules · set · unset · weather"
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

fn cmd_status() -> Result<(), String> {
    println!("{}", term::header("tezca bar"));
    println!();
    if running() {
        println!("  {} {} is running", term::green("●"), term::bold(BIN));
    } else {
        println!("  {} {} is not running", term::dim("○"), term::bold(BIN));
        println!("    {}", term::dim("start it with `tezca bar start`"));
    }
    Ok(())
}

fn cmd_start() -> Result<(), String> {
    if running() {
        println!("  {} {} already running", term::dim("·"), BIN);
        return Ok(());
    }
    spawn()?;
    println!("  {} started {}", term::green("→"), BIN);
    Ok(())
}

fn cmd_stop() -> Result<(), String> {
    if !running() {
        println!("  {} {} not running", term::dim("·"), BIN);
        return Ok(());
    }
    pkill(&["-x", BIN]);
    println!("  {} stopped {}", term::green("✓"), BIN);
    Ok(())
}

fn cmd_restart() -> Result<(), String> {
    if running() {
        pkill(&["-TERM", "-x", BIN]);
        for _ in 0..50 {
            if !running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    spawn()?;
    println!("  {} restarted {}", term::green("✓"), BIN);
    Ok(())
}

/// Toggle bar visibility (SIGUSR1). Starts it first if it isn't running.
fn cmd_toggle() -> Result<(), String> {
    if !running() {
        return cmd_start();
    }
    pkill(&["-USR1", "-x", BIN]);
    println!("  {} toggled {}", term::green("✓"), BIN);
    Ok(())
}

// --- config (config.toml) --------------------------------------------------

fn bar_toml() -> Result<PathBuf, String> {
    Ok(repo::config_home()?.join("tezca-bar").join("config.toml"))
}

/// `tezca bar modules` — every placeable module id, one per line as
/// `id<TAB>label<TAB>hint`.
///
/// Machine-readable on purpose: it is what the settings Modules editor reads to
/// populate its picker. That editor used to carry its own hardcoded list, which
/// drifted until it was offering two modules the bar has never had and spelling
/// three more by their aliases. Now there is one list and everyone asks it.
fn cmd_modules() -> Result<(), String> {
    for m in Mod::ALL {
        println!("{}\t{}\t{}", m.id(), m.label(), m.hint());
    }
    Ok(())
}

/// `tezca bar config` — the effective values (file over defaults), one per line
/// (`key = value`). Machine-readable for tezca-settings.
fn cmd_config() -> Result<(), String> {
    let text = std::fs::read_to_string(bar_toml()?).unwrap_or_default();
    for (key, default) in SCALARS {
        let val = read_scalar(&text, key).unwrap_or_else(|| default.to_string());
        println!("{key} = {val}");
    }
    for (key, default) in layout_scalars() {
        let val = read_scalar(&text, key).unwrap_or_else(|| default.to_string());
        println!("{key} = {val}");
    }
    // Plus the dynamic keys the file defines: per-output workspace sets and
    // per-monitor layout overrides. Both are `<base>.<connector>`, so neither
    // can live in a fixed table and both have to be read back off the file.
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let k = k.trim();
            if k.starts_with(WS_ASSIGN_PREFIX) || matches!(Region::parse_key(k), Some((_, Some(_))))
            {
                let v = v
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'');
                println!("{k} = {v}");
            }
        }
    }
    Ok(())
}

/// `tezca bar set <key> <value> [<key> <value>…]` — edit config.toml (preserving
/// comments) then restart the bar if it's running so the change takes effect.
/// `tezca bar weather <search <place…> | here | set <lat> <lon> [name…]>`
///
/// The lookups live in `tezca-bar` (it owns the host allowlist and the only
/// HTTP code in this project), so this drives them through the binary rather
/// than opening a second network path from the CLI.
fn cmd_weather(args: &[&str]) -> Result<(), String> {
    match args.first().copied() {
        Some("search") if args.len() > 1 => {
            let query = args[1..].join(" ");
            let out = bar_query(&["--weather-search", &query])?;
            let places = parse_places(&out);
            if places.is_empty() {
                return Err(format!("no place matched {query:?}"));
            }
            println!("{}", term::header("matches"));
            println!();
            for (label, lat, lon) in &places {
                println!("  {}", term::bold(label));
                println!(
                    "    {}",
                    term::dim(&format!("tezca bar weather set {lat} {lon} {label}"))
                );
            }
            println!();
            println!("  {}", term::dim("run one of the lines above to save it"));
            Ok(())
        }
        // Deliberately not called `locate`: this is a guess from your IP, and
        // the name should say that it is asking where you *appear* to be.
        Some("here") => {
            let out = bar_query(&["--weather-locate"])?;
            let places = parse_places(&out);
            let Some((label, lat, lon)) = places.first() else {
                return Err("could not work out a location from your IP".into());
            };
            println!("{}", term::header("looks like"));
            println!();
            println!("  {}  {}", term::bold(label), term::dim(&format!("{lat}, {lon}")));
            println!();
            println!(
                "  {}",
                term::dim("a VPN will place you somewhere you are not — check it, then run:")
            );
            println!("    {}", term::dim(&format!("tezca bar weather set {lat} {lon} {label}")));
            Ok(())
        }
        Some("set") if args.len() >= 3 => {
            let (lat, lon) = (args[1], args[2]);
            lat.parse::<f64>().map_err(|_| format!("not a latitude: {lat}"))?;
            lon.parse::<f64>().map_err(|_| format!("not a longitude: {lon}"))?;
            let place = args[3..].join(" ");
            let mut kvs: Vec<&str> =
                vec!["weather_enabled", "true", "weather_lat", lat, "weather_lon", lon];
            if !place.is_empty() {
                kvs.push("weather_place");
                kvs.push(&place);
            }
            cmd_set(&kvs)
        }
        _ => Err("usage: tezca bar weather <search PLACE… | here | set LAT LON [NAME…]>".into()),
    }
}

/// Run `tezca-bar <args>` and capture stdout.
fn bar_query(args: &[&str]) -> Result<String, String> {
    let out =
        Command::new(BIN).args(args).output().map_err(|e| format!("could not run {BIN}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "lookup failed".into() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `label\tlat\tlon` lines into triples.
fn parse_places(out: &str) -> Vec<(String, String, String)> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?.to_string(), f.next()?.to_string(), f.next()?.to_string()))
        })
        .collect()
}

/// `tezca bar unset <key>…` — delete a key so it falls back to its default (or,
/// for a `layout_*.<connector>` override, back to following the global layout).
fn cmd_unset(args: &[&str]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: tezca bar unset <key> [<key>…]".into());
    }
    let path = bar_toml()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut hit = false;
    for key in args {
        hit |= unset_line(&mut lines, key);
    }
    if !hit {
        // Not an error: "make sure this isn't set" is a reasonable thing to ask,
        // and it already isn't.
        println!("  {} nothing to remove", term::dim("·"));
        return Ok(());
    }
    write_and_reload(&path, lines)
}

fn cmd_set(args: &[&str]) -> Result<(), String> {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Err("usage: tezca bar set <key> <value> [<key> <value>…]".into());
    }
    let path = bar_toml()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    for pair in args.chunks(2) {
        let (key, val) = (pair[0], pair[1]);
        // `layout_<region>` and `layout_<region>.<connector>` carry module ids,
        // and those are checked here rather than left to the bar. The bar drops
        // an id it doesn't know — the right call at render time, since a config
        // from another build must still produce a usable bar — but that makes a
        // typo invisible: you set it, the bar restarts, and the module simply is
        // not there. Refusing at the point of writing is where the mistake is
        // still attached to the thing that made it.
        if let Some((_, output)) = Region::parse_key(key) {
            if let Some(out) = output {
                if out.contains(char::is_whitespace) {
                    return Err(format!("`{out}` is not a monitor connector name"));
                }
            }
            let bad = unknown_ids(val);
            if !bad.is_empty() {
                return Err(format!(
                    "no bar module called {} \n  {} tezca bar modules  lists every id",
                    bad.iter().map(|b| format!("`{b}`")).collect::<Vec<_>>().join(" or "),
                    term::dim("try:"),
                ));
            }
            set_line(&mut lines, key, val);
            continue;
        }
        if SCALARS.iter().any(|(k, _)| *k == key) || key.starts_with(WS_ASSIGN_PREFIX) {
            set_line(&mut lines, key, val);
        } else {
            return Err(format!("unknown bar key: {key}"));
        }
    }

    write_and_reload(&path, lines)
}

/// Save the edited file and put the change on screen, shared by `set` and
/// `unset` so both report the same way.
fn write_and_reload(path: &Path, lines: Vec<String>) -> Result<(), String> {
    let mut body = lines.join("\n");
    body.push('\n');
    atomic::write(path, &body)?;

    if running() {
        let _ = cmd_restart();
    } else {
        println!(
            "  {} saved (bar not running — starts with your settings next time)",
            term::green("✓")
        );
    }
    Ok(())
}

/// Read a scalar `key = value` (ignoring any trailing `# comment`).
fn read_scalar(text: &str, key: &str) -> Option<String> {
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix('=') {
                let val = after.split('#').next().unwrap_or("").trim();
                if !val.is_empty() {
                    return Some(val.trim_matches(|c| c == '"' || c == '\'').to_string());
                }
            }
        }
    }
    None
}

/// Upsert `key = value`, preserving indentation and any trailing `# comment`.
fn set_line(lines: &mut Vec<String>, key: &str, value: &str) {
    for l in lines.iter_mut() {
        let trimmed = l.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix('=') {
                let indent = &l[..l.len() - trimmed.len()];
                let comment = after
                    .split_once('#')
                    .map(|(_, c)| format!("  # {}", c.trim()))
                    .unwrap_or_default();
                *l = format!("{indent}{key} = {value}{comment}");
                return;
            }
        }
    }
    lines.push(format!("{key} = {value}"));
}

/// Delete `key`'s line outright, comment and all.
///
/// Distinct from setting it empty, which for a per-monitor layout override is a
/// meaningful value ("this monitor shows nothing here"). Without a way to remove
/// the line there would be no way back from an override to inheriting the global
/// layout — a one-way door in the settings editor.
fn unset_line(lines: &mut Vec<String>, key: &str) -> bool {
    let before = lines.len();
    lines.retain(|l| {
        let trimmed = l.trim_start();
        if trimmed.starts_with('#') {
            return true;
        }
        match trimmed.strip_prefix(key) {
            Some(rest) => !rest.trim_start().starts_with('='),
            None => true,
        }
    });
    lines.len() != before
}

// --- helpers ---------------------------------------------------------------

/// Launch the bar. Prefer `uwsm app --` so it lands in the session's systemd
/// slice (matching autostart.conf), and wrap in `setsid` so the bar starts in a
/// fresh session — otherwise the launched process stays in this CLI's (and its
/// terminal's) process group and dies when that terminal closes. At login via
/// `exec-once` this is moot, but `tezca bar start` from a shell needs it.
fn spawn() -> Result<(), String> {
    if !util::has(BIN) {
        return Err(format!("{BIN} not found on PATH — build + install it (install.sh) first"));
    }
    let has_setsid = util::has("setsid");
    let has_uwsm = util::has("uwsm");
    let mut cmd = if has_setsid {
        let mut c = Command::new("setsid");
        if has_uwsm {
            c.args(["uwsm", "app", "--", BIN]);
        } else {
            c.arg(BIN);
        }
        c
    } else if has_uwsm {
        let mut c = Command::new("uwsm");
        c.args(["app", "--", BIN]);
        c
    } else {
        Command::new(BIN)
    };
    // Detach the child's stdio. A long-lived process that inherits our stdout keeps
    // the write end of any pipe open, so `tezca bar restart | tee log` (or any
    // caller that captures output) blocks forever waiting for EOF that never comes.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch bar: {e}"))?;
    Ok(())
}

fn running() -> bool {
    Command::new("pkill").args(["-0", "-x", BIN]).status().map(|s| s.success()).unwrap_or(false)
}

fn pkill(args: &[&str]) {
    let _ = Command::new("pkill").args(args).status();
}

/// `tezca bar --help`.
fn print_help() {
    println!("{}", term::header("tezca bar"));
    println!("{}", term::dim("  control the bespoke top menubar (tezca-bar)"));
    println!();
    println!("  {}                  is the bar running?", term::cyan("status"));
    println!("  {}  lifecycle", term::cyan("start · stop · restart"));
    println!(
        "  {}                  hide/show it (SIGUSR1 — the ALT+Right-Ctrl bind)",
        term::cyan("toggle")
    );
    println!("  {}                  print the effective configuration", term::cyan("config"));
    println!("  {}                 every placeable module id", term::cyan("modules"));
    println!("  {}      edit ~/.config/tezca-bar/config.toml", term::cyan("set <key> <value>…"));
    println!("  {}              drop a key back to its default", term::cyan("unset <key>…"));
    println!();
    println!("{}", term::dim("  weather — find coordinates for the weather module"));
    println!("  {}   look a town or city up by name", term::cyan("weather search <place…>"));
    println!(
        "  {}            guess from your IP {}",
        term::cyan("weather here"),
        term::dim("(the one call that tells a third party where you are)")
    );
    println!("  {}  save them and switch the module on", term::cyan("weather set <lat> <lon>"));
    println!();
    println!("{}", term::dim("  e.g. tezca bar set height 38 layout_center nowplaying"));
    println!("{}", term::dim("       tezca bar set layout_right.DP-3 \"cpu, mem, clock, power\""));
    println!("{}", term::dim("       tezca bar weather search Guadalajara"));
}
