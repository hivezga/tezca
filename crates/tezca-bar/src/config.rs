//! Bar configuration — `~/.config/tezca-bar/config.toml`.
//!
//! Hand-parsed loose `key = value` (mirroring tezca-dock's config.rs) so the bar
//! stays dependency-light. Every field has a baked-in default, so a missing or
//! partial file still runs.

use std::collections::HashMap;
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

/// How workspace pills are labelled: Western digits or Mayan bar-and-dot
/// numerals (the Mesoamerican vigesimal glyphs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Numerals {
    Arabic,
    Mayan,
}

impl Numerals {
    fn parse(s: &str) -> Option<Numerals> {
        match s.trim().to_lowercase().as_str() {
            "arabic" | "latin" | "western" | "digits" => Some(Numerals::Arabic),
            // `nahuatl`/`aztec` kept as friendly aliases for the same glyph mode.
            "mayan" | "maya" | "nahuatl" | "aztec" | "mexica" => Some(Numerals::Mayan),
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
    /// Workspace pill labels — Western digits or Nahuatl words.
    pub numerals: Numerals,
    /// Per-output workspace assignment: connector name → the workspace ids that
    /// output's bar always shows, in this order. Empty = the default behaviour
    /// (each bar shows whatever workspaces Hyprland has placed on its monitor).
    pub ws_assign: HashMap<String, Vec<i32>>,
    /// Show only occupied (windowed) workspaces plus the focused one, hiding
    /// empty pills. Applies whether the set is assigned or dynamic.
    pub hide_empty: bool,
    /// Auto-compact each assigned workspace set: when a non-visible workspace
    /// empties, pull the higher workspaces in that monitor's set down to close
    /// the gap (windows move, staying on the same monitor). Needs `ws_assign`.
    pub compact: bool,
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
            numerals: Numerals::Arabic,
            ws_assign: HashMap::new(),
            hide_empty: false,
            compact: false,
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
            // Strip a trailing `# comment` (matching `tezca bar config`'s reader)
            // before quotes, so inline-documented values parse correctly.
            let k = k.trim();
            let v = v.split('#').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
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
                "workspace_numerals" | "numerals" => {
                    if let Some(n) = Numerals::parse(v) {
                        self.numerals = n;
                    }
                }
                "workspace_hide_empty" | "hide_empty_workspaces" => set_bool(&mut self.hide_empty, v),
                "workspace_compact" | "compact_workspaces" => set_bool(&mut self.compact, v),
                // `workspaces.<connector> = <spec>` — per-output workspace sets.
                _ if k.starts_with("workspaces.") => {
                    let output = k["workspaces.".len()..].trim();
                    match parse_ws_spec(v) {
                        Some(ids) => {
                            self.ws_assign.insert(output.to_string(), ids);
                        }
                        None => {
                            self.ws_assign.remove(output);
                        }
                    }
                }
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

/// Parse a per-output workspace spec into an explicit id list:
///   * `auto` / empty  → None (fall back to Hyprland's live placement)
///   * `odd` / `even`  → 1,3,5,7,9 / 2,4,6,8,10
///   * `1-5`           → an inclusive range
///   * `1,3,5,7,9`     → an explicit comma list (order preserved)
fn parse_ws_spec(v: &str) -> Option<Vec<i32>> {
    let s = v.trim().to_lowercase();
    match s.as_str() {
        "" | "auto" | "dynamic" => None,
        "odd" => Some((1..=10).filter(|n| n % 2 == 1).collect()),
        "even" => Some((1..=10).filter(|n| n % 2 == 0).collect()),
        _ => {
            if let Some((a, b)) = s.split_once('-') {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<i32>(), b.trim().parse::<i32>()) {
                    if a >= 1 && a <= b {
                        return Some((a..=b).collect());
                    }
                }
            }
            let ids: Vec<i32> = s.split(',').filter_map(|p| p.trim().parse().ok()).filter(|n| *n > 0).collect();
            (!ids.is_empty()).then_some(ids)
        }
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
fn set_bool(dst: &mut bool, v: &str) {
    match v.trim().to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => *dst = true,
        "false" | "no" | "off" | "0" => *dst = false,
        _ => {}
    }
}
