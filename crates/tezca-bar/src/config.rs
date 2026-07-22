//! Bar configuration — `~/.config/tezca-bar/config.toml`.
//!
//! Hand-parsed loose `key = value` (mirroring tezca-dock's config.rs) so the bar
//! stays dependency-light. Every field has a baked-in default, so a missing or
//! partial file still runs.

use std::path::PathBuf;

/// Bar shape. `floating` is a rounded glass strip inset from the edges; `edge`
/// is a full-width, square, edge-to-edge bar with a single bottom hairline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Floating,
    Edge,
}

impl Shape {
    fn parse(s: &str) -> Option<Shape> {
        match s.trim().to_lowercase().as_str() {
            "floating" | "float" => Some(Shape::Floating),
            "edge" | "full" => Some(Shape::Edge),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub shape: Shape,
    /// Bar height, px (the glass strip; the layer surface reserves this + margin).
    pub height: i32,
    /// Gap above the bar (floating only), px.
    pub margin_top: i32,
    /// Gap left/right of the bar (floating only), px.
    pub margin_side: i32,
    /// Poll intervals, seconds.
    pub cpu_interval: u32,
    pub mem_interval: u32,
    pub gpu_interval: u32,
    pub net_interval: u32,
    /// strftime-style clock format (glib::DateTime::format).
    pub clock_format: String,
    /// Monitors narrower than this (px) render the compact layout: no per-app
    /// menu bar, tighter padding. The ultrawide primary stays full.
    pub compact_width: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shape: Shape::Floating,
            height: 40,
            margin_top: 6,
            margin_side: 10,
            cpu_interval: 3,
            mem_interval: 5,
            gpu_interval: 3,
            net_interval: 5,
            clock_format: "%a %d %b   %H:%M".to_string(),
            compact_width: 3000,
        }
    }
}

impl Config {
    /// Load from the standard path, falling back to defaults for anything the
    /// file doesn't set. Never fails.
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
        Some(base.join("tezca-bar").join("config.toml"))
    }

    fn apply(&mut self, text: &str) {
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim().trim_matches(|c| c == '"' || c == '\''));
            match k {
                "shape" => {
                    if let Some(s) = Shape::parse(v) {
                        self.shape = s;
                    }
                }
                "height" => set_i32(&mut self.height, v),
                "margin_top" => set_i32(&mut self.margin_top, v),
                "margin_side" => set_i32(&mut self.margin_side, v),
                "cpu_interval" => set_u32(&mut self.cpu_interval, v),
                "mem_interval" => set_u32(&mut self.mem_interval, v),
                "gpu_interval" => set_u32(&mut self.gpu_interval, v),
                "net_interval" => set_u32(&mut self.net_interval, v),
                "clock_format" => self.clock_format = v.to_string(),
                "compact_width" => set_i32(&mut self.compact_width, v),
                _ => {}
            }
        }
        // Clamp to sane ranges so a typo can't wedge the geometry.
        self.height = self.height.clamp(20, 80);
        self.cpu_interval = self.cpu_interval.max(1);
        self.mem_interval = self.mem_interval.max(1);
        self.gpu_interval = self.gpu_interval.max(1);
        self.net_interval = self.net_interval.max(1);
    }
}

fn set_i32(dst: &mut i32, v: &str) {
    if let Ok(n) = v.parse() {
        *dst = n;
    }
}
fn set_u32(dst: &mut u32, v: &str) {
    if let Ok(n) = v.parse() {
        *dst = n;
    }
}
