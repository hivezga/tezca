//! Palette + live GTK CSS — the bar tracks `tezca theme` with zero restart.
//!
//! Two consumers of the theme:
//!   * the widget tree — styled by GTK CSS. We load the user's own
//!     `~/.config/tezca/current/colors.css` (valid GTK `@define-color` tokens)
//!     as one provider, and our `bar.css` — which references those `@tz_*`
//!     names via `alpha()`/`mix()`, exactly like `config/waybar/style.css` — as
//!     a second. On SIGUSR2 both reload, so a theme switch reskins instantly.
//!   * the self-drawn bits (mirror glyph, sparklines, equalizer) — cairo needs
//!     concrete RGBA, so we ALSO parse that same file into a [`Palette`], the
//!     way `tezca-dock`'s theme.rs does.
//!
//! Everything falls back to the signature obsidian constants (matching
//! `themes/obsidian/colors.css`) when a token or the file is absent.

use gtk4::gdk::{Display, RGBA};
use gtk4::CssProvider;
use std::path::PathBuf;

/// Bundled GTK stylesheet (references the `@tz_*` colors the palette provider
/// defines). Kept in-crate so the binary is self-contained.
const BAR_CSS: &str = include_str!("bar.css");

#[derive(Clone, Debug)]
pub struct Palette {
    pub base: RGBA,
    pub surface: RGBA,
    pub text: RGBA,
    pub subtext: RGBA,
    pub muted: RGBA,
    pub faint: RGBA,
    pub accent: RGBA,
    pub accent_dim: RGBA,
    pub on_accent: RGBA,
    pub gold: RGBA,
    pub urgent: RGBA,
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
            faint: hex("#5A6166"),
            accent: hex("#3FB8AF"),
            accent_dim: hex("#2A8C86"),
            on_accent: hex("#0B0E0F"),
            gold: hex("#C9A24B"),
            urgent: hex("#E06C75"),
        }
    }
}

impl Palette {
    /// Load from `~/.config/tezca/current/colors.css`, keeping defaults for any
    /// token the file omits. Never fails.
    pub fn load() -> Self {
        let mut p = Palette::default();
        let Some(path) = colors_path() else { return p };
        let Ok(text) = std::fs::read_to_string(&path) else { return p };
        for (name, rgba) in parse_define_colors(&text) {
            match name.as_str() {
                "tz_base" => p.base = rgba,
                "tz_surface" => p.surface = rgba,
                "tz_text" => p.text = rgba,
                "tz_subtext" => p.subtext = rgba,
                "tz_muted" => p.muted = rgba,
                "tz_faint" => p.faint = rgba,
                "tz_accent" => p.accent = rgba,
                "tz_accent_dim" => p.accent_dim = rgba,
                "tz_on_accent" => p.on_accent = rgba,
                "tz_gold" => p.gold = rgba,
                "tz_urgent" => p.urgent = rgba,
                _ => {}
            }
        }
        p
    }
}

/// Owns the two application-level CSS providers so a theme reload can swap them.
pub struct CssStack {
    palette_provider: CssProvider,
    bar_provider: CssProvider,
}

impl CssStack {
    /// Install both providers on the default display and load them once.
    pub fn install(display: &Display) -> Self {
        let palette_provider = CssProvider::new();
        let bar_provider = CssProvider::new();
        // The palette must be added at a lower priority than bar.css so the
        // `@define-color` tokens are defined before bar.css references them; GTK
        // resolves named colors globally, so order-within-priority is enough.
        gtk4::style_context_add_provider_for_display(
            display,
            &palette_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk4::style_context_add_provider_for_display(
            display,
            &bar_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let stack = CssStack { palette_provider, bar_provider };
        stack.reload();
        stack
    }

    /// Re-read colors.css and re-apply bar.css. Called at startup and on SIGUSR2.
    pub fn reload(&self) {
        // colors.css is plain `@define-color` lines — valid GTK CSS. Fall back
        // to the obsidian tokens inlined if the file can't be read.
        let colors = colors_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(fallback_colors);
        self.palette_provider.load_from_data(&colors);
        self.bar_provider.load_from_data(BAR_CSS);
    }
}

// ---------------------------------------------------------------------------

fn colors_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tezca").join("current").join("colors.css"))
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

/// The obsidian tokens as a GTK CSS string — used when colors.css is missing so
/// bar.css's named-color references still resolve.
fn fallback_colors() -> String {
    "\
@define-color tz_base #0B0E0F;
@define-color tz_surface #14191B;
@define-color tz_text #E8EAED;
@define-color tz_subtext #C3C8CC;
@define-color tz_muted #8B9398;
@define-color tz_faint #5A6166;
@define-color tz_accent #3FB8AF;
@define-color tz_accent_dim #2A8C86;
@define-color tz_on_accent #0B0E0F;
@define-color tz_gold #C9A24B;
@define-color tz_urgent #E06C75;
"
    .to_string()
}
