//! Thin wrappers over `hyprctl` shared by `tezca hypr` and `tezca display`.

use std::process::Command;

/// True when we're inside a live Hyprland session (so `hyprctl` is meaningful).
pub fn in_session() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// Run `hyprctl <args…>` and normalise the result.
///
/// `hyprctl` exits 0 for a good many failures and reports them on stdout
/// instead, so success is judged on the output as well as the status.
fn hyprctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("hyprctl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run hyprctl: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }
    // hyprctl answers a rejected command on stdout with exit 0, so the text has
    // to be inspected too. The refusals do not share a common word — none of
    // these three contains "error":
    //
    //   [string "…"]:1: '}' expected near …          (a Lua syntax error)
    //   keyword can't work with non-legacy parsers.  (legacy-only entry point)
    //   eval is only supported with the lua config manager
    //
    // The last one is what a session still running hyprlang answers, and missing
    // it made every `tezca hypr set` report success while changing nothing.
    if is_refusal(&stdout) {
        return Err(stdout);
    }
    if stderr.to_lowercase().contains("error") {
        return Err(stderr);
    }
    Ok(stdout)
}

/// Evaluate a Lua snippet in the running compositor: `hyprctl eval <code>`.
///
/// This is the live-apply path under the Lua config manager. `hyprctl keyword`
/// used to do this job and is gone — it is implemented only on the legacy
/// parser, and answers "keyword can't work with non-legacy parsers. Use eval."
///
/// Keep snippets small: Hyprland guards `eval` with a 250 ms watchdog
/// (`LUA_TIMEOUT_EVAL_MS`) and kills anything slower.
pub fn eval(code: &str) -> Result<(), String> {
    hyprctl(&["eval", code]).map(|_| ())
}

/// Apply one option live, e.g. `set_option("decoration:rounding", "14")`.
pub fn set_option(option: &str, value: &str) -> Result<(), String> {
    eval(&lua_config_call(option, &crate::managed::lua_value(value)))
}

/// Apply one monitor spec live.
pub fn set_monitor(m: &crate::managed::Monitor) -> Result<(), String> {
    eval(&lua_monitor_call(m))
}

/// Read the current value of an option as a normalized scalar. Handles every
/// `hyprctl getoption` shape (int / float / custom "a b c d" / str) by taking
/// the first whitespace token of the value after the leading `type:` label.
///   `int: 12`              → "12"
///   `float: 0.980000`      → "0.980000"
///   `custom type: 5 5 5 5` → "5"
///
/// Unaffected by the Lua migration: `getoption` is `getConfigValue` on the
/// shared config-manager interface, which both parsers implement.
pub fn getoption(kw: &str) -> Option<String> {
    let out = Command::new("hyprctl").args(["getoption", kw]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let first = s.lines().next()?;
    let (_ty, val) = first.split_once(':')?;
    Some(val.split_whitespace().next()?.to_string())
}

/// `hyprctl reload` — re-read the whole config (best-effort). Still valid under
/// the Lua parser: `reload()` is on the shared config-manager interface.
pub fn reload() -> Result<(), String> {
    hyprctl(&["reload"]).map(|_| ())
}

// --- Lua snippet construction (unit-tested) ---------------------------------

/// Build `hl.config({ a = { b = <value> } })` from an option path `a:b`.
///
/// Hyprland spells option paths with `:` between categories and sometimes `.`
/// within one (`general:col.active_border`), and both mean "descend a level", so
/// the path is split on either.
fn lua_config_call(option: &str, value_literal: &str) -> String {
    let mut inner = value_literal.to_string();
    let segments: Vec<&str> = option.split([':', '.']).filter(|s| !s.is_empty()).collect();
    for seg in segments.iter().rev() {
        inner = format!("{{ {seg} = {inner} }}");
    }
    format!("hl.config({inner})")
}

fn lua_monitor_call(m: &crate::managed::Monitor) -> String {
    let mut s = format!(
        "hl.monitor({{ output = \"{}\", mode = \"{}\", position = \"{}\", scale = {}",
        m.output,
        m.mode,
        m.position,
        crate::managed::lua_value(&m.scale)
    );
    if !m.transform.is_empty() && m.transform != "0" {
        s.push_str(&format!(", transform = {}", crate::managed::lua_value(&m.transform)));
    }
    s.push_str(" })");
    s
}

/// True when `out` is one of hyprctl's exit-0 refusals rather than a result.
/// Split out from [`hyprctl`] so the classification is testable without a
/// running compositor.
fn is_refusal(out: &str) -> bool {
    let low = out.to_lowercase();
    low.contains("error")
        || low.contains("can't work with")
        || low.contains("only supported with")
        || low.starts_with("[string")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::Monitor;

    #[test]
    fn recognises_every_exit_zero_refusal_hyprctl_produces() {
        // None of these contains the word "error"; each was observed from a real
        // hyprctl. Treating one as success made `tezca hypr set` claim it had
        // applied a setting that never changed.
        assert!(is_refusal("eval is only supported with the lua config manager"));
        assert!(is_refusal("keyword can't work with non-legacy parsers. Use eval."));
        assert!(is_refusal("[string \"return hl.dispatch(exec true)\"]:1: ')' expected"));
        assert!(is_refusal("Error: something broke"));
        // …and a real success is not mistaken for one.
        assert!(!is_refusal("ok"));
        assert!(!is_refusal("int: 12"));
    }

    #[test]
    fn nests_an_option_path_into_a_config_call() {
        assert_eq!(
            lua_config_call("decoration:rounding", "14"),
            "hl.config({ decoration = { rounding = 14 } })"
        );
        assert_eq!(
            lua_config_call("decoration:blur:size", "8"),
            "hl.config({ decoration = { blur = { size = 8 } } })"
        );
    }

    #[test]
    fn treats_a_dot_inside_a_category_as_another_level() {
        // `hyprctl getoption general:col.active_border` is one key with one of
        // each separator; both have to descend, or the gradient lands on a key
        // literally named "col.active_border" and is silently ignored.
        assert_eq!(
            lua_config_call("general:col.active_border", "\"rgba(3FB8AFff)\""),
            "hl.config({ general = { col = { active_border = \"rgba(3FB8AFff)\" } } })"
        );
    }

    #[test]
    fn builds_a_monitor_call_and_omits_an_absent_transform() {
        let m = Monitor {
            output: "DP-1".into(),
            mode: "3440x1440@165".into(),
            position: "0x0".into(),
            scale: "1".into(),
            transform: String::new(),
        };
        assert_eq!(
            lua_monitor_call(&m),
            "hl.monitor({ output = \"DP-1\", mode = \"3440x1440@165\", position = \"0x0\", scale = 1 })"
        );
        let rotated = Monitor { transform: "1".into(), ..m };
        assert!(lua_monitor_call(&rotated).ends_with(", transform = 1 })"));
    }
}
