//! The active palette, read from the same file the bar reads.
//!
//! `tezca theme set` copies the chosen `themes/<name>/colors.css` to
//! `~/.config/tezca/current/colors.css`, and every GTK surface in the project
//! `@import`s it. The webview cannot, so this parses the twelve
//! `@define-color tz_* #rrggbb` lines out of it and hands them over as CSS
//! custom properties.
//!
//! Parsing the live file rather than shipping a copy is the point: there is
//! exactly one definition of what "accent" means on this machine, and a theme
//! the user added by hand works here without this crate knowing its name.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The palette plus the derived values a stylesheet needs but a GTK theme file
/// does not carry, because GTK computes them with `alpha()` at use site.
#[derive(Serialize, Default)]
pub struct Tokens {
    /// `tz_accent` → `#D2E4E2`, verbatim from the theme file.
    pub colors: BTreeMap<String, String>,
    /// The theme's own name, when `theme.state` names a curated one.
    pub name: String,
    /// True when the palette is light, so the front end can pick the shadow and
    /// hairline weights that survive on it. Derived from the base colour's
    /// luminance rather than from the theme's name — a user's own theme file is
    /// as entitled to be light as `smoke` is.
    pub light: bool,
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// Parse `@define-color tz_accent   #D2E4E2;` lines into a map.
///
/// Deliberately tolerant: anything that is not such a line is skipped rather
/// than treated as an error, because the file is allowed to carry comments and
/// this is not the place a malformed theme should surface.
pub fn parse(css: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in css.lines() {
        let Some(rest) = line.trim().strip_prefix("@define-color") else { continue };
        let mut it = rest.split_whitespace();
        let Some(name) = it.next() else { continue };
        let Some(value) = it.next() else { continue };
        let value = value.trim_end_matches(';').trim();
        if let Some(short) = name.strip_prefix("tz_") {
            out.insert(short.replace('_', "-"), value.to_string());
        }
    }
    out
}

/// True when `#rrggbb` is light enough that dark text belongs on it.
///
/// Rec. 709 luma, which weights green the way the eye does; a flat mean calls
/// the blue-heavy themes light when they are not.
fn is_light(hex: &str) -> bool {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return false;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    (0.2126 * c(0) + 0.7152 * c(2) + 0.0722 * c(4)) > 128.0
}

/// Read the live palette. Falls back to an empty map, which leaves the
/// stylesheet's own defaults standing rather than painting an unstyled window.
pub fn tokens() -> Tokens {
    let Some(dir) = config_dir() else { return Tokens::default() };
    let css = std::fs::read_to_string(dir.join("tezca/current/colors.css")).unwrap_or_default();
    let colors = parse(&css);
    let name = std::fs::read_to_string(dir.join("tezca/current/theme.state"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let light = colors.get("base").map(|b| is_light(b)).unwrap_or(false);
    Tokens { light, name: if name.starts_with("dynamic:") { String::new() } else { name }, colors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn define_color_lines_become_custom_property_names() {
        let css = "/* comment */\n@define-color tz_accent      #D2E4E2;\n\
                   @define-color tz_on_accent   #0B0E0F;\n\
                   @define-color unrelated      #FFFFFF;\nnot a rule\n";
        let m = parse(css);
        assert_eq!(m.get("accent").map(String::as_str), Some("#D2E4E2"));
        // Underscores become hyphens so the front end can write --tz-on-accent.
        assert_eq!(m.get("on-accent").map(String::as_str), Some("#0B0E0F"));
        // Non-tz colours are somebody else's business.
        assert!(!m.contains_key("unrelated"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn lightness_follows_luma_not_the_theme_name() {
        // smoke's base, and the four dark themes' shared base.
        assert!(is_light("#F1F4F5"));
        assert!(!is_light("#0B0E0F"));
        // A saturated blue is dark even though its blue channel is maxed; a
        // flat mean would get this one wrong.
        assert!(!is_light("#0000FF"));
        assert!(is_light("#00FF00"));
        assert!(!is_light("nonsense"));
    }
}
