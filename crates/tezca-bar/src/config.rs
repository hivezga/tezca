//! Bar configuration — `~/.config/tezca-bar/config.toml`.
//!
//! Hand-parsed loose `key = value` (mirroring tezca-dock's config.rs) so the bar
//! stays dependency-light. Every field has a baked-in default, so a missing or
//! partial file still runs.

use crate::ai::AiConfig;
use crate::llm::LlmConfig;
use crate::weather::WeatherConfig;
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

/// What to do about the right cluster once it holds twenty-odd modules.
///
/// There is no single right answer — it depends on how wide the monitor is and
/// how much of the cluster you actually read — so the bar offers four and lets
/// you pick, rather than choosing one and calling it the design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clutter {
    /// Every module, all the time. The historical behaviour.
    All,
    /// Privacy indicators and hardware metrics each collapse into one chip you
    /// can click open. Densest without hiding anything.
    Grouped,
    /// Everything stays, but the modules you rarely read fade back until you
    /// hover the bar. Nothing moves, so nothing shifts under the pointer.
    Hover,
    /// The third-tier modules are dropped from the layout entirely.
    Tiers,
}

impl Clutter {
    fn parse(s: &str) -> Option<Clutter> {
        Some(match s.trim().to_lowercase().as_str() {
            "all" | "none" | "off" => Clutter::All,
            "grouped" | "group" | "clusters" => Clutter::Grouped,
            "hover" | "reveal" | "hover-reveal" => Clutter::Hover,
            "tiers" | "tier" | "priority" => Clutter::Tiers,
            _ => return None,
        })
    }
}

/// A placeable bar module — one widget slot in a region's ordered layout.
/// `Sep` is a thin vertical divider and may repeat; every other variant maps to
/// exactly one built-in widget built in `bar.rs`. Unknown names never parse (so
/// a typo in config can't inject anything), mirroring how `ai_providers` filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mod {
    Mirror,
    Appname,
    Workspaces,
    Submap,
    NowPlaying,
    GameMode,
    Camera,
    Microphone,
    Recording,
    Caffeine,
    NightLight,
    Ai,
    Weather,
    Llm,
    Tray,
    Cpu,
    Mem,
    Gpu,
    Network,
    Bluetooth,
    Volume,
    Brightness,
    Battery,
    Bell,
    Clock,
    Power,
    Sep,
}

impl Mod {
    /// Parse one module id (with a few friendly aliases). None for anything
    /// unrecognised — the caller drops it.
    pub fn parse(s: &str) -> Option<Mod> {
        Some(match s.trim().to_lowercase().as_str() {
            "mirror" | "menu" => Mod::Mirror,
            "appname" | "app" | "window" | "title" => Mod::Appname,
            "workspaces" | "ws" => Mod::Workspaces,
            "submap" => Mod::Submap,
            "nowplaying" | "now-playing" | "media" | "mpris" => Mod::NowPlaying,
            "gamemode" | "game" => Mod::GameMode,
            "camera" | "cam" | "webcam" => Mod::Camera,
            "microphone" | "mic" => Mod::Microphone,
            "recording" | "record" | "screenrec" => Mod::Recording,
            "caffeine" | "keepawake" | "keep-awake" | "inhibit" => Mod::Caffeine,
            "night" | "nightlight" | "night-light" | "bluelight" => Mod::NightLight,
            "ai" => Mod::Ai,
            "weather" | "forecast" => Mod::Weather,
            "llm" | "ollama" | "localai" | "local-ai" => Mod::Llm,
            "tray" => Mod::Tray,
            "cpu" => Mod::Cpu,
            "mem" | "memory" | "ram" => Mod::Mem,
            "gpu" => Mod::Gpu,
            "network" | "net" | "wifi" => Mod::Network,
            "bluetooth" | "bt" => Mod::Bluetooth,
            "volume" | "vol" | "audio" => Mod::Volume,
            "brightness" | "backlight" => Mod::Brightness,
            "battery" | "bat" => Mod::Battery,
            "bell" | "notifications" | "notif" => Mod::Bell,
            "clock" | "time" | "date" => Mod::Clock,
            "power" | "logout" => Mod::Power,
            "sep" | "separator" | "|" => Mod::Sep,
            _ => return None,
        })
    }

    /// Modules the [`Clutter::Tiers`] strategy drops.
    ///
    /// The test is "would you notice this were gone for an hour". A GPU
    /// percentage and a keep-awake glyph both fail it; a clock and a battery
    /// do not. Deliberately not configurable — a per-module priority list is a
    /// second layout to keep in step with the first, and the layout keys
    /// already let you remove anything outright.
    pub fn is_tier3(self) -> bool {
        matches!(self, Mod::Caffeine | Mod::NightLight | Mod::Tray | Mod::Gpu | Mod::Brightness)
    }

    /// Modules the [`Clutter::Hover`] strategy fades until the bar is hovered.
    ///
    /// A superset of [`Self::is_tier3`] minus the GPU (a metric you glance at
    /// mid-task) plus Bluetooth (a battery you check on purpose, never by
    /// accident).
    pub fn is_ambient(self) -> bool {
        matches!(
            self,
            Mod::Caffeine | Mod::NightLight | Mod::Tray | Mod::Bluetooth | Mod::Brightness
        )
    }
}

/// One entry in a region's layout: either a built-in module or a user/community
/// `custom:<name>` exec module (see `custom.rs`). Kept separate from `Mod` so the
/// built-in vocabulary stays `Copy` and exhaustively matched.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Mod(Mod),
    Custom(String),
}

impl Slot {
    /// Parse one layout token. `custom:<name>` is the explicit escape hatch for a
    /// third-party module; every other token must be a known built-in id (a bare
    /// unknown token is dropped, so only an intentional `custom:` prefix can ever
    /// introduce a non-built-in slot).
    pub fn parse(s: &str) -> Option<Slot> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix("custom:") {
            let name = rest.trim();
            return (!name.is_empty()).then(|| Slot::Custom(name.to_string()));
        }
        Mod::parse(t).map(Slot::Mod)
    }

    pub fn is_sep(&self) -> bool {
        matches!(self, Slot::Mod(Mod::Sep))
    }
    pub fn is_appname(&self) -> bool {
        matches!(self, Slot::Mod(Mod::Appname))
    }
}

/// Parse a comma-separated region layout into slots, dropping unknown ids.
fn parse_layout(v: &str) -> Vec<Slot> {
    v.split(',').filter_map(Slot::parse).collect()
}

fn default_layout_left() -> Vec<Slot> {
    use Mod::*;
    [Mirror, Sep, Appname, Sep, Workspaces, Submap].into_iter().map(Slot::Mod).collect()
}
fn default_layout_center() -> Vec<Slot> {
    vec![Slot::Mod(Mod::NowPlaying)]
}
fn default_layout_right() -> Vec<Slot> {
    use Mod::*;
    [
        GameMode, Camera, Microphone, Recording, Caffeine, NightLight, Ai, Tray, Cpu, Mem, Gpu,
        Sep, Network, Bluetooth, Volume, Brightness, Battery, Sep, Bell, Clock, Power,
    ]
    .into_iter()
    .map(Slot::Mod)
    .collect()
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
    /// AI provider usage module — opt-in, and the only module that can make a
    /// network request. See `ai.rs` for the privacy posture.
    pub ai: AiConfig,
    /// Weather module — opt-in, and the second (and only other) module that can
    /// make a network request. See `weather.rs` for the privacy posture.
    pub weather: WeatherConfig,
    /// Local AI (Ollama). Loopback only — see `llm.rs`.
    pub llm: LlmConfig,
    /// Show the transient volume on-screen display (the glass pill that fades in
    /// when the volume changes or mutes). See `osd.rs`.
    pub osd_enabled: bool,
    /// How long the volume OSD dwells before fading out, milliseconds.
    pub osd_timeout_ms: u32,
    /// How the right cluster copes with its own length. See [`Clutter`].
    pub clutter: Clutter,
    /// Which modules each region shows, in order. `layout_*` config keys (a
    /// comma-separated list of module ids) override these; the defaults
    /// reproduce the historical hardcoded arrangement exactly. Entries are
    /// built-ins or `custom:<name>` exec modules (see `custom.rs`).
    pub layout_left: Vec<Slot>,
    pub layout_center: Vec<Slot>,
    pub layout_right: Vec<Slot>,
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
            ai: AiConfig::default(),
            weather: WeatherConfig::default(),
            llm: LlmConfig::default(),
            osd_enabled: true,
            osd_timeout_ms: 1400,
            clutter: Clutter::All,
            layout_left: default_layout_left(),
            layout_center: default_layout_center(),
            layout_right: default_layout_right(),
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

    /// Does any region's layout include this built-in module? Lets callers skip
    /// work for a module the user has removed (e.g. the camera `/proc` scan).
    pub fn uses_mod(&self, m: Mod) -> bool {
        let want = Slot::Mod(m);
        self.layout_left.contains(&want)
            || self.layout_center.contains(&want)
            || self.layout_right.contains(&want)
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
            let v =
                v.split('#').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
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
                "workspace_hide_empty" | "hide_empty_workspaces" => {
                    set_bool(&mut self.hide_empty, v)
                }
                "workspace_compact" | "compact_workspaces" => set_bool(&mut self.compact, v),
                // --- AI usage module -------------------------------------
                "ai_enabled" => set_bool(&mut self.ai.enabled, v),
                "ai_providers" => {
                    // Unknown names are dropped rather than passed through, so
                    // a typo can never become a request to an unexpected host.
                    self.ai.providers = v
                        .split(',')
                        .map(|s| s.trim().to_lowercase())
                        .filter(|s| matches!(s.as_str(), "anthropic" | "openai" | "google"))
                        .collect();
                }
                "ai_interval" => set_u32(&mut self.ai.interval, v),
                "ai_live" => set_bool(&mut self.ai.live, v),
                "ai_local" => set_bool(&mut self.ai.local, v),
                "ai_warn" => set_f64(&mut self.ai.warn, v),
                "ai_critical" => set_f64(&mut self.ai.critical, v),
                // --- Weather ---------------------------------------------
                "weather_enabled" => set_bool(&mut self.weather.enabled, v),
                // Absent or unparseable leaves the coordinate as None, which
                // keeps the module unusable — never a guessed location.
                "weather_lat" | "weather_latitude" => self.weather.lat = v.parse().ok(),
                "weather_lon" | "weather_longitude" => self.weather.lon = v.parse().ok(),
                "weather_place" | "weather_label" => self.weather.place = v.to_string(),
                "weather_interval" => set_u32(&mut self.weather.interval, v),
                "weather_unit" => {
                    self.weather.fahrenheit = matches!(
                        v.trim().to_lowercase().as_str(),
                        "f" | "fahrenheit" | "imperial" | "us"
                    )
                }
                "weather_aqi" => set_bool(&mut self.weather.aqi, v),
                // --- Local AI (Ollama) ------------------------------------
                "llm_enabled" => set_bool(&mut self.llm.enabled, v),
                // Blank / "auto" leaves it None, which probes for whichever
                // server is actually running.
                "llm_backend" => self.llm.backend = crate::llm::Backend::parse(v),
                "llm_port" => {
                    if let Ok(n) = v.parse() {
                        self.llm.port = n;
                    }
                }
                "llm_interval" => set_u32(&mut self.llm.interval, v),
                "llm_model" => self.llm.model = v.to_string(),
                "llm_system" => self.llm.system = v.to_string(),
                // --- Volume OSD ------------------------------------------
                "osd_enabled" => set_bool(&mut self.osd_enabled, v),
                "osd_timeout_ms" | "osd_timeout" => set_u32(&mut self.osd_timeout_ms, v),
                "clutter" | "right_cluster" => {
                    if let Some(c) = Clutter::parse(v) {
                        self.clutter = c;
                    }
                }
                // --- Module layout (per region, ordered) -----------------
                // An empty / all-unknown value keeps the built-in default so a
                // stray line can never blank out a region.
                "layout_left" => {
                    let l = parse_layout(v);
                    if !l.is_empty() {
                        self.layout_left = l;
                    }
                }
                "layout_center" => {
                    let l = parse_layout(v);
                    if !l.is_empty() {
                        self.layout_center = l;
                    }
                }
                "layout_right" => {
                    let l = parse_layout(v);
                    if !l.is_empty() {
                        self.layout_right = l;
                    }
                }
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
        // A too-eager AI poll earns a machine-wide 429 from the provider — and
        // the same 429 bucket is the one Claude Code itself uses. Clamp hard.
        self.ai.interval = self.ai.interval.max(60);
        self.ai.warn = self.ai.warn.clamp(0.0, 100.0);
        self.ai.critical = self.ai.critical.clamp(self.ai.warn, 100.0);
        self.osd_timeout_ms = self.osd_timeout_ms.clamp(400, 10_000);
        // Weather changes on the scale of tens of minutes; a tighter loop is
        // free load on a service that charges nothing for it.
        self.weather.interval = self.weather.interval.max(300);
        // A coordinate outside the globe is a typo, not a place.
        self.weather.lat = self.weather.lat.filter(|v| (-90.0..=90.0).contains(v));
        self.weather.lon = self.weather.lon.filter(|v| (-180.0..=180.0).contains(v));
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
            let ids: Vec<i32> =
                s.split(',').filter_map(|p| p.trim().parse().ok()).filter(|n| *n > 0).collect();
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
fn set_f64(dst: &mut f64, v: &str) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn m(x: Mod) -> Slot {
        Slot::Mod(x)
    }

    #[test]
    fn layout_defaults_match_historical_arrangement() {
        let c = Config::default();
        assert_eq!(c.layout_left, default_layout_left());
        assert_eq!(c.layout_center, vec![m(Mod::NowPlaying)]);
        // The right cluster, in the exact order bar.rs used to hardcode.
        use Mod::*;
        assert_eq!(
            c.layout_right,
            [
                GameMode, Camera, Microphone, Recording, Caffeine, NightLight, Ai, Tray, Cpu, Mem,
                Gpu, Sep, Network, Bluetooth, Volume, Brightness, Battery, Sep, Bell, Clock, Power
            ]
            .into_iter()
            .map(Slot::Mod)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn layout_csv_round_trips_with_aliases_and_whitespace() {
        let mut c = Config::default();
        c.apply("layout_right = clock , power ,vol\nlayout_center = media");
        assert_eq!(c.layout_right, vec![m(Mod::Clock), m(Mod::Power), m(Mod::Volume)]);
        assert_eq!(c.layout_center, vec![m(Mod::NowPlaying)]);
    }

    #[test]
    fn unknown_module_ids_are_dropped_not_injected() {
        let mut c = Config::default();
        c.apply("layout_left = mirror, bogus, workspaces");
        assert_eq!(c.layout_left, vec![m(Mod::Mirror), m(Mod::Workspaces)]);
    }

    #[test]
    fn empty_or_all_unknown_layout_keeps_default() {
        let mut c = Config::default();
        c.apply("layout_left =\nlayout_right = nonsense, alsobad");
        assert_eq!(c.layout_left, default_layout_left());
        assert_eq!(c.layout_right, default_layout_right());
    }

    #[test]
    fn sep_token_repeats() {
        let mut c = Config::default();
        c.apply("layout_right = cpu, sep, sep, mem");
        assert_eq!(c.layout_right, vec![m(Mod::Cpu), m(Mod::Sep), m(Mod::Sep), m(Mod::Mem)]);
    }

    #[test]
    fn custom_prefix_makes_a_custom_slot_bare_unknown_is_still_dropped() {
        let mut c = Config::default();
        c.apply("layout_right = cpu, custom:barometer, custom: , barometer");
        // `custom:barometer` → a custom slot; the empty `custom:` and the bare
        // `barometer` (not a built-in id) are both dropped.
        assert_eq!(c.layout_right, vec![m(Mod::Cpu), Slot::Custom("barometer".into())]);
    }

    #[test]
    fn a_custom_module_may_share_a_name_with_a_builtin() {
        // `weather` became a built-in after `custom:weather` was a plausible
        // thing to have installed. The prefix is what disambiguates, so both
        // can sit in the same layout and neither shadows the other.
        let mut c = Config::default();
        c.apply("layout_right = weather, custom:weather");
        assert_eq!(c.layout_right, vec![m(Mod::Weather), Slot::Custom("weather".into())]);
    }

    #[test]
    fn weather_needs_both_coordinates_and_rejects_impossible_ones() {
        let mut c = Config::default();
        c.apply("weather_enabled = true\nweather_lat = 19.4326");
        assert!(!c.weather.usable(), "half a coordinate is not a location");

        let mut c = Config::default();
        c.apply("weather_enabled = true\nweather_lat = 19.4326\nweather_lon = -99.1332");
        assert!(c.weather.usable());

        // Off the globe is a typo, not a place — and the module must not poll.
        let mut c = Config::default();
        c.apply("weather_enabled = true\nweather_lat = 991.0\nweather_lon = -99.1332");
        assert_eq!(c.weather.lat, None);
        assert!(!c.weather.usable());
    }

    #[test]
    fn weather_stays_off_unless_asked_for() {
        // The default must never reach the network: coordinates alone are not
        // consent, and neither is the module being placed in a layout.
        let mut c = Config::default();
        assert!(!c.weather.enabled);
        c.apply("weather_lat = 19.4326\nweather_lon = -99.1332");
        assert!(!c.weather.usable());
    }

    #[test]
    fn weather_interval_has_a_floor() {
        let mut c = Config::default();
        c.apply("weather_interval = 5");
        assert_eq!(c.weather.interval, 300);
    }
}
