//! Dock configuration — `~/.config/tezca-dock/dock.toml`.
//!
//! Hand-parsed (loose key = value + comma/array lists), mirroring the CLI's
//! `cmd_theme::read_meta` style so tezca-dock stays dependency-light. Every
//! field has a baked-in default, so a missing or partial file still runs.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    /// Base (unmagnified) icon edge length, px.
    pub icon_size: f64,
    /// Peak magnification factor for the icon directly under the cursor.
    pub max_scale: f64,
    /// Half-width of the magnification falloff, px — how far the bulge reaches.
    pub influence: f64,
    /// Gap between adjacent icons, px.
    pub gap: f64,
    /// Inner padding of the glass pill, px (x, y).
    pub pad_x: f64,
    pub pad_y: f64,
    /// Gap between the pill and the screen edge, px.
    pub margin_bottom: i32,
    /// Height of the always-present reveal hotspot at the screen bottom, px.
    pub hotspot_height: i32,
    /// Delay before an un-hovered dock hides, ms.
    pub hide_delay_ms: u64,
    /// Ordered favourites, by desktop-app id or window class.
    pub pinned: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            icon_size: 48.0,
            max_scale: 1.6,
            influence: 110.0,
            gap: 10.0,
            pad_x: 12.0,
            pad_y: 8.0,
            margin_bottom: 8,
            hotspot_height: 6,
            hide_delay_ms: 350,
            pinned: ["kitty", "firefox"].iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Config {
    /// Load from the standard path, falling back to defaults for anything the
    /// file doesn't set. Never fails — a bad file just yields defaults.
    pub fn load() -> Self {
        let mut cfg = Config::default();
        let Some(path) = Self::path() else { return cfg };
        let Ok(text) = std::fs::read_to_string(&path) else { return cfg };
        cfg.apply(&text);
        cfg
    }

    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("tezca-dock").join("dock.toml"))
    }

    fn apply(&mut self, text: &str) {
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "icon_size" => set_f64(&mut self.icon_size, v),
                "max_scale" => set_f64(&mut self.max_scale, v),
                "influence" => set_f64(&mut self.influence, v),
                "gap" => set_f64(&mut self.gap, v),
                "pad_x" => set_f64(&mut self.pad_x, v),
                "pad_y" => set_f64(&mut self.pad_y, v),
                "margin_bottom" => set_i32(&mut self.margin_bottom, v),
                "hotspot_height" => set_i32(&mut self.hotspot_height, v),
                "hide_delay_ms" => set_u64(&mut self.hide_delay_ms, v),
                "pinned" => {
                    let list = parse_list(v);
                    if !list.is_empty() {
                        self.pinned = list;
                    }
                }
                _ => {}
            }
        }
        // Clamp to sane ranges so a typo can't wedge the geometry.
        self.max_scale = self.max_scale.clamp(1.0, 3.0);
        self.icon_size = self.icon_size.clamp(16.0, 128.0);
        self.influence = self.influence.max(1.0);
    }
}

fn set_f64(dst: &mut f64, v: &str) {
    if let Ok(n) = v.parse() {
        *dst = n;
    }
}
fn set_i32(dst: &mut i32, v: &str) {
    if let Ok(n) = v.parse() {
        *dst = n;
    }
}
fn set_u64(dst: &mut u64, v: &str) {
    if let Ok(n) = v.parse() {
        *dst = n;
    }
}

/// Parse either a TOML-ish array `["a", "b"]` or a bare comma list `a, b`.
fn parse_list(v: &str) -> Vec<String> {
    v.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}
