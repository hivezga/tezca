//! The bar's module vocabulary — the single place that knows what a module is
//! called.
//!
//! [`Mod`] is the closed set of built-in widgets, [`Slot`] adds the
//! `custom:<name>` escape hatch, and [`Region`] names the three areas of the bar
//! along with their default contents. Everything that reads or writes a
//! `layout_*` key goes through here: the bar to render, the CLI to validate, and
//! the settings editor (via `tezca bar modules`) to offer.
//!
//! Each module has exactly one *canonical* id — what [`Mod::id`] returns and
//! what anything writing config should emit — plus any number of friendly
//! aliases [`Mod::parse`] also accepts. Keeping those two ideas apart is the
//! point: `mic` and `microphone` must resolve to one module, or a duplicate
//! check comparing strings will think they are two.

#![forbid(unsafe_code)]

/// A placeable bar module — one widget slot in a region's ordered layout.
///
/// `Sep` is a thin vertical divider and may repeat; every other variant maps to
/// exactly one built-in widget built in `bar.rs`. Unknown names never parse, so
/// a typo in config cannot inject anything.
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
    /// Every built-in, in the order the Modules editor offers them: the reading
    /// order of the bar itself (identity, then context, then metrics, then
    /// controls), rather than the order the enum happens to be declared in.
    pub const ALL: [Mod; 26] = [
        Mod::Mirror,
        Mod::Appname,
        Mod::Workspaces,
        Mod::Submap,
        Mod::NowPlaying,
        Mod::GameMode,
        Mod::Camera,
        Mod::Microphone,
        Mod::Recording,
        Mod::Caffeine,
        Mod::NightLight,
        Mod::Ai,
        Mod::Weather,
        Mod::Tray,
        Mod::Cpu,
        Mod::Mem,
        Mod::Gpu,
        Mod::Network,
        Mod::Bluetooth,
        Mod::Volume,
        Mod::Brightness,
        Mod::Battery,
        Mod::Bell,
        Mod::Clock,
        Mod::Power,
        Mod::Sep,
    ];

    /// The canonical id — what `id()` round-trips through [`Mod::parse`], and
    /// what anything writing a `layout_*` key must emit. Always the first
    /// alternative listed in `parse`.
    pub fn id(self) -> &'static str {
        match self {
            Mod::Mirror => "mirror",
            Mod::Appname => "appname",
            Mod::Workspaces => "workspaces",
            Mod::Submap => "submap",
            Mod::NowPlaying => "nowplaying",
            Mod::GameMode => "gamemode",
            Mod::Camera => "camera",
            Mod::Microphone => "microphone",
            Mod::Recording => "recording",
            Mod::Caffeine => "caffeine",
            Mod::NightLight => "night",
            Mod::Ai => "ai",
            Mod::Weather => "weather",
            Mod::Tray => "tray",
            Mod::Cpu => "cpu",
            Mod::Mem => "mem",
            Mod::Gpu => "gpu",
            Mod::Network => "network",
            Mod::Bluetooth => "bluetooth",
            Mod::Volume => "volume",
            Mod::Brightness => "brightness",
            Mod::Battery => "battery",
            Mod::Bell => "bell",
            Mod::Clock => "clock",
            Mod::Power => "power",
            Mod::Sep => "sep",
        }
    }

    /// What to call it on screen in the Modules editor. Not an id — never write
    /// one of these to config.
    pub fn label(self) -> &'static str {
        match self {
            Mod::Mirror => "Tezca menu",
            Mod::Appname => "App name",
            Mod::Workspaces => "Workspaces",
            Mod::Submap => "Submap",
            Mod::NowPlaying => "Now playing",
            Mod::GameMode => "Game mode",
            Mod::Camera => "Camera",
            Mod::Microphone => "Microphone",
            Mod::Recording => "Recording",
            Mod::Caffeine => "Keep awake",
            Mod::NightLight => "Night light",
            Mod::Ai => "AI usage",
            Mod::Weather => "Weather",
            Mod::Tray => "System tray",
            Mod::Cpu => "CPU",
            Mod::Mem => "Memory",
            Mod::Gpu => "GPU",
            Mod::Network => "Network",
            Mod::Bluetooth => "Bluetooth",
            Mod::Volume => "Volume",
            Mod::Brightness => "Brightness",
            Mod::Battery => "Battery",
            Mod::Bell => "Notifications",
            Mod::Clock => "Clock",
            Mod::Power => "Power",
            Mod::Sep => "Separator",
        }
    }

    /// One line of explanation for the picker, so choosing a module does not
    /// require already knowing what it does.
    pub fn hint(self) -> &'static str {
        match self {
            Mod::Mirror => "The smoking-mirror menu",
            Mod::Appname => "Focused window's app name",
            Mod::Workspaces => "Workspace pills",
            Mod::Submap => "Active submap; auto-hides",
            Mod::NowPlaying => "Media title, artist and equaliser",
            Mod::GameMode => "Game-mode glyph; auto-hides",
            Mod::Camera => "Camera in use; red when live",
            Mod::Microphone => "Mic in use; gold when recording",
            Mod::Recording => "Screen recording in progress",
            Mod::Caffeine => "Hold or release the idle inhibitor",
            Mod::NightLight => "Shown only while the filter is on",
            Mod::Ai => "AI subscription usage; auto-hides",
            Mod::Weather => "Current conditions; needs coordinates",
            Mod::Tray => "StatusNotifierItem icons; auto-hides",
            Mod::Cpu => "CPU sparkline and percentage",
            Mod::Mem => "Memory sparkline and percentage",
            Mod::Gpu => "GPU sparkline; auto-hides without a source",
            Mod::Network => "Wi-Fi/ethernet glyph and throughput",
            Mod::Bluetooth => "Adapter and connected devices",
            Mod::Volume => "Output volume and mixer",
            Mod::Brightness => "Backlight; auto-hides on desktops",
            Mod::Battery => "Charge level; auto-hides on desktops",
            Mod::Bell => "Notification bell",
            Mod::Clock => "Clock and calendar",
            Mod::Power => "Power and logout",
            Mod::Sep => "A thin divider; may be repeated",
        }
    }

    /// Parse one module id, accepting a few friendly aliases. `None` for
    /// anything unrecognised — the caller decides whether to drop or reject it.
    ///
    /// The first alternative in each arm is the canonical id and must match
    /// [`Mod::id`]; `id_round_trips` in the tests below enforces that.
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

    /// Modules the `tiers` clutter strategy drops.
    ///
    /// The test is "would you notice this were gone for an hour". A GPU
    /// percentage and a keep-awake glyph both fail it; a clock and a battery do
    /// not. Deliberately not configurable — a per-module priority list is a
    /// second layout to keep in step with the first, and the layout keys already
    /// let you remove anything outright.
    pub fn is_tier3(self) -> bool {
        matches!(self, Mod::Caffeine | Mod::NightLight | Mod::Tray | Mod::Gpu | Mod::Brightness)
    }

    /// Modules the `hover` clutter strategy fades until the bar is hovered.
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
/// `custom:<name>` exec module. Kept separate from [`Mod`] so the built-in
/// vocabulary stays `Copy` and exhaustively matched.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Mod(Mod),
    Custom(String),
}

impl Slot {
    /// Parse one layout token. `custom:<name>` is the explicit escape hatch for
    /// a third-party module; every other token must be a known built-in id (a
    /// bare unknown token never parses, so only an intentional `custom:` prefix
    /// can introduce a non-built-in slot).
    pub fn parse(s: &str) -> Option<Slot> {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix("custom:") {
            let name = rest.trim();
            return (!name.is_empty()).then(|| Slot::Custom(name.to_string()));
        }
        Mod::parse(t).map(Slot::Mod)
    }

    /// The canonical spelling, suitable for writing back to config.
    pub fn id(&self) -> String {
        match self {
            Slot::Mod(m) => m.id().to_string(),
            Slot::Custom(name) => format!("custom:{name}"),
        }
    }

    pub fn is_sep(&self) -> bool {
        matches!(self, Slot::Mod(Mod::Sep))
    }

    pub fn is_appname(&self) -> bool {
        matches!(self, Slot::Mod(Mod::Appname))
    }
}

/// Parse a comma-separated region layout, dropping unknown ids.
///
/// Dropping rather than failing is right *here*, at render time: a config file
/// written by an older or newer build must still produce a usable bar. Anything
/// that *writes* a layout should call [`unknown_ids`] first and refuse, so the
/// unknown token is reported when it can still be corrected instead of
/// disappearing silently hours later.
pub fn parse_layout(v: &str) -> Vec<Slot> {
    v.split(',').filter_map(Slot::parse).collect()
}

/// The tokens in a layout string that no module answers to, in order.
///
/// Empty means the whole string is placeable. Blank entries are ignored, so a
/// trailing comma is not an error.
pub fn unknown_ids(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| Slot::parse(t).is_none())
        .map(str::to_string)
        .collect()
}

/// Which area of the bar a layout key describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Left,
    Center,
    Right,
}

impl Region {
    pub const ALL: [Region; 3] = [Region::Left, Region::Center, Region::Right];

    /// The config key for this region's global (all-monitors) layout. A
    /// per-monitor override is this key with `.<connector>` appended.
    pub fn key(self) -> &'static str {
        match self {
            Region::Left => "layout_left",
            Region::Center => "layout_center",
            Region::Right => "layout_right",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Region::Left => "Left",
            Region::Center => "Center",
            Region::Right => "Right",
        }
    }

    /// The built-in arrangement, reproducing the bar's historical hardcoded one.
    pub fn default_layout(self) -> &'static str {
        match self {
            Region::Left => "mirror, sep, appname, sep, workspaces, submap",
            Region::Center => "nowplaying",
            Region::Right => {
                "gamemode, camera, microphone, recording, caffeine, night, ai, tray, cpu, mem, \
                 gpu, sep, network, bluetooth, volume, brightness, battery, sep, bell, clock, power"
            }
        }
    }

    /// Split a `layout_*` key into its region and optional monitor connector.
    /// `layout_right` → `(Right, None)`; `layout_right.DP-3` → `(Right, Some("DP-3"))`.
    pub fn parse_key(key: &str) -> Option<(Region, Option<&str>)> {
        let (base, output) = match key.split_once('.') {
            Some((b, o)) if !o.is_empty() => (b, Some(o)),
            Some(_) => return None,
            None => (key, None),
        };
        let region = Region::ALL.into_iter().find(|r| r.key() == base)?;
        Some((region, output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical id must be one the parser accepts, and must come back as
    /// itself. Without this, `id()` and `parse()` can drift and a round-trip
    /// through the settings editor silently rewrites a module into another.
    #[test]
    fn id_round_trips() {
        for m in Mod::ALL {
            assert_eq!(Mod::parse(m.id()), Some(m), "{} did not round-trip", m.id());
        }
    }

    /// `ALL` must actually be all of them — a module added to the enum but not
    /// the list would be invisible in the Modules editor forever.
    #[test]
    fn all_is_exhaustive() {
        let mut ids: Vec<&str> = Mod::ALL.iter().map(|m| m.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Mod::ALL.len(), "duplicate id in Mod::ALL");
        // Every default layout is written in canonical ids, so between them the
        // three defaults exercise most of the vocabulary; anything they name
        // must be in ALL.
        for region in Region::ALL {
            for slot in parse_layout(region.default_layout()) {
                if let Slot::Mod(m) = slot {
                    assert!(Mod::ALL.contains(&m), "{} missing from Mod::ALL", m.id());
                }
            }
        }
    }

    /// Aliases resolve to the same module as the canonical id. This is the
    /// property the settings editor's duplicate check needs: it must compare
    /// resolved modules, not strings, or `mic` and `microphone` look distinct.
    #[test]
    fn aliases_resolve_to_canonical() {
        for (alias, canonical) in [
            ("mic", "microphone"),
            ("net", "network"),
            ("notifications", "bell"),
            ("media", "nowplaying"),
            ("nightlight", "night"),
            ("bat", "battery"),
        ] {
            assert_eq!(Mod::parse(alias), Mod::parse(canonical), "{alias} != {canonical}");
        }
    }

    /// The ids the settings editor used to offer, which the bar has never had.
    /// They must stay unparseable so `unknown_ids` reports them.
    #[test]
    fn never_invented_modules() {
        // `llm`/`ollama` were the local-AI module, removed with the rest of that
        // feature. They stay listed here so a config that still names one gets
        // reported rather than silently ignored.
        for bogus in ["appmenu", "privacy", "nonsense", "llm", "ollama", "local-ai"] {
            assert_eq!(Mod::parse(bogus), None, "{bogus} should not parse");
        }
        assert_eq!(unknown_ids("mirror, appmenu, workspaces, privacy"), ["appmenu", "privacy"]);
        assert!(unknown_ids("mirror, sep, workspaces, custom:foo").is_empty());
        assert!(unknown_ids("clock,").is_empty(), "a trailing comma is not an error");
    }

    /// Every default layout must be fully placeable — a default that names a
    /// module the parser rejects would ship a bar with a hole in it.
    #[test]
    fn defaults_are_valid() {
        for region in Region::ALL {
            assert!(
                unknown_ids(region.default_layout()).is_empty(),
                "{} default has unknown ids",
                region.key()
            );
        }
    }

    #[test]
    fn parses_layout_keys() {
        assert_eq!(Region::parse_key("layout_right"), Some((Region::Right, None)));
        assert_eq!(Region::parse_key("layout_right.DP-3"), Some((Region::Right, Some("DP-3"))));
        assert_eq!(Region::parse_key("layout_center.DP-1"), Some((Region::Center, Some("DP-1"))));
        assert_eq!(Region::parse_key("layout_middle"), None);
        assert_eq!(Region::parse_key("layout_right."), None);
        assert_eq!(Region::parse_key("workspaces.DP-1"), None);
    }

    #[test]
    fn custom_slots_round_trip() {
        assert_eq!(Slot::parse("custom:weather").map(|s| s.id()), Some("custom:weather".into()));
        assert_eq!(Slot::parse("custom:"), None);
    }
}
