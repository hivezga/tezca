//! Where the panel's settings come from, and where they go back to.
//!
//! Both ends are the bar's config: `tezca bar config` to read, `tezca bar set`
//! to write. The panel does not keep a file of its own, because the bar module
//! and the panel are two views of one thing — a system prompt the panel edited
//! into its own store would be a system prompt the bar knew nothing about.
//!
//! Shelling out rather than parsing `config.toml` here: the CLI already owns
//! the only authoritative parse, and a second one would drift the first time a
//! key gained a default.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use tezca_llm as llm;

/// Absolute path to `tezca` — prefer ~/.local/bin, where install.sh puts it and
/// which is not always on a GUI process's PATH.
fn tezca_bin() -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/bin/tezca");
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    "tezca".into()
}

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new(tezca_bin()).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The `llm_*` half of the bar's effective config.
pub fn load() -> llm::LlmConfig {
    let mut cfg = llm::LlmConfig::default();
    let Some(out) = run(&["bar", "config"]) else { return cfg };
    for line in out.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "llm_enabled" => cfg.enabled = matches!(v, "true" | "on" | "1" | "yes"),
            "llm_backend" => cfg.backend = llm::Backend::parse(v),
            "llm_port" => cfg.port = v.parse().unwrap_or(0),
            "llm_interval" => cfg.interval = v.parse().unwrap_or(5),
            "llm_model" => cfg.model = v.to_string(),
            "llm_system" => cfg.system = v.to_string(),
            _ => {}
        }
    }
    cfg
}

/// Persist one key. `key` is the bare name (`model`, `system`, …) and is
/// prefixed here, so a caller cannot reach a config key outside `llm_*`.
pub fn set(key: &str, value: &str) -> bool {
    const ALLOWED: &[&str] = &["enabled", "backend", "port", "interval", "model", "system"];
    if !ALLOWED.contains(&key) {
        return false;
    }
    let full = format!("llm_{key}");
    Command::new(tezca_bin())
        .args(["bar", "set", &full, value])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The live palette, as CSS custom properties.
///
/// Same source and same reasoning as `tezca-settings`: parse the file every GTK
/// surface imports, so a theme the user added by hand works here without this
/// crate knowing its name.
#[derive(Serialize, Default)]
pub struct Tokens {
    pub colors: BTreeMap<String, String>,
    pub light: bool,
}

pub fn tokens() -> Tokens {
    let Some(dir) = config_dir() else { return Tokens::default() };
    let css = std::fs::read_to_string(dir.join("tezca/current/colors.css")).unwrap_or_default();
    let mut colors = BTreeMap::new();
    for line in css.lines() {
        let Some(rest) = line.trim().strip_prefix("@define-color") else { continue };
        let mut it = rest.split_whitespace();
        let (Some(name), Some(value)) = (it.next(), it.next()) else { continue };
        if let Some(short) = name.strip_prefix("tz_") {
            colors.insert(short.replace('_', "-"), value.trim_end_matches(';').to_string());
        }
    }
    let light = colors.get("base").map(|b| is_light(b)).unwrap_or(false);
    Tokens { colors, light }
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// Rec. 709 luma — a flat mean calls the blue-heavy themes light when they are
/// not.
fn is_light(hex: &str) -> bool {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return false;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    (0.2126 * c(0) + 0.7152 * c(2) + 0.0722 * c(4)) > 128.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_llm_keys_are_writable() {
        // The panel's drawer must not be a general-purpose config editor: a key
        // outside the llm_* namespace is refused before it reaches the CLI.
        assert!(!set("height", "9999"));
        assert!(!set("../../etc/passwd", "x"));
        assert!(!set("", ""));
    }
}
