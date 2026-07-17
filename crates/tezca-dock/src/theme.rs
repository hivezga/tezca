//! Palette — read the live theme-engine colors so the dock tracks `tezca theme`.
//!
//! Every other Tezca component `@import`s `~/.config/tezca/current/colors.css`;
//! the dock is self-drawn (no GTK CSS for its glass), so it parses that same
//! file for the `@define-color tz_*` tokens and paints from them. Re-read on
//! SIGUSR2 when a theme switch repoints `current/`. Falls back to the signature
//! obsidian constants (matching themes/obsidian/colors.css) if a token is absent.

use gtk4::gdk::RGBA;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Palette {
    pub base: RGBA,
    pub surface: RGBA,
    pub text: RGBA,
    pub subtext: RGBA,
    pub muted: RGBA,
    pub accent: RGBA,
    pub accent_dim: RGBA,
    pub gold: RGBA,
}

impl Default for Palette {
    fn default() -> Self {
        // Signature obsidian — mirrors themes/obsidian/colors.css.
        Self {
            base: hex("#0B0E0F"),
            surface: hex("#14191B"),
            text: hex("#E8EAED"),
            subtext: hex("#C3C8CC"),
            muted: hex("#8B9398"),
            accent: hex("#3FB8AF"),
            accent_dim: hex("#2A8C86"),
            gold: hex("#C9A24B"),
        }
    }
}

impl Palette {
    /// Load from `~/.config/tezca/current/colors.css`, keeping defaults for any
    /// token the file omits. Never fails.
    pub fn load() -> Self {
        let mut p = Palette::default();
        let Some(path) = Self::path() else { return p };
        let Ok(text) = std::fs::read_to_string(&path) else { return p };
        for (name, rgba) in parse_define_colors(&text) {
            match name.as_str() {
                "tz_base" => p.base = rgba,
                "tz_surface" => p.surface = rgba,
                "tz_text" => p.text = rgba,
                "tz_subtext" => p.subtext = rgba,
                "tz_muted" => p.muted = rgba,
                "tz_accent" => p.accent = rgba,
                "tz_accent_dim" => p.accent_dim = rgba,
                "tz_gold" => p.gold = rgba,
                _ => {}
            }
        }
        p
    }

    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("tezca").join("current").join("colors.css"))
    }
}

/// `@define-color tz_accent   #3FB8AF;` → ("tz_accent", RGBA). Ignores anything
/// that isn't a well-formed hex color line.
fn parse_define_colors(text: &str) -> Vec<(String, RGBA)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("@define-color") else { continue };
        let rest = rest.trim().trim_end_matches(';');
        let mut it = rest.split_whitespace();
        let (Some(name), Some(value)) = (it.next(), it.next()) else { continue };
        if let Ok(rgba) = RGBA::parse(value) {
            out.push((name.to_string(), rgba));
        }
    }
    out
}

/// Parse a hex string known-good at compile time; obsidian fallback on the
/// (impossible) parse failure so this stays panic-free.
fn hex(s: &str) -> RGBA {
    RGBA::parse(s).unwrap_or_else(|_| RGBA::new(0.043, 0.055, 0.059, 1.0))
}

/// Return `rgba` with its alpha multiplied by `a` — the dock's glass tints.
pub fn with_alpha(rgba: RGBA, a: f32) -> RGBA {
    RGBA::new(rgba.red(), rgba.green(), rgba.blue(), rgba.alpha() * a)
}
