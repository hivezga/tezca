//! Now-playing — MPRIS via `playerctl` shell-out.
//!
//! The prototype centres a media pill (art, title, artist, live equaliser) and
//! expands it into a transport popover. We source both from `playerctl` (the
//! ubiquitous MPRIS CLI) rather than pulling a D-Bus dependency into the bar,
//! matching the crate's shell-out philosophy. Absent player → `None`, and the
//! pill is hidden.
//!
//! The split matters: [`current`] is one command and runs on the bar's
//! two-second tick, while [`detail`] costs four more and only runs when the
//! popover opens.

use std::process::Command;

pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// True while the player reports `Playing` — the equaliser animates only
    /// then, and the transport button shows pause rather than play.
    pub playing: bool,
    /// Elapsed and total, in seconds. Either is `None` for a stream, which has
    /// no length, or for a player that does not publish a position.
    pub position: Option<u64>,
    pub length: Option<u64>,
    /// `mpris:artUrl`. Carried on the cheap per-tick record rather than in
    /// [`Detail`] because it is one more field on a call already being made,
    /// and the strip's thumbnail needs it every time the track changes.
    pub art_url: String,
}

impl NowPlaying {
    /// The strip's sub-line: `artist · 2:14`, dropping whichever half is absent.
    pub fn subtitle(&self) -> String {
        match (self.artist.is_empty(), self.position.map(clock)) {
            (true, Some(p)) => p,
            (false, Some(p)) => format!("{} \u{00B7} {p}", self.artist),
            (_, None) => self.artist.clone(),
        }
    }
}

/// The fields [`current`] asks playerctl for, unit-separated so a title or an
/// artist containing spaces — or a comma — survives the round trip.
const CURRENT_FORMAT: &str =
    "{{status}}\x1f{{title}}\x1f{{artist}}\x1f{{position}}\x1f{{mpris:length}}\x1f{{mpris:artUrl}}";

/// Current track, or None if no MPRIS player is present.
pub fn current() -> Option<NowPlaying> {
    parse_current(&meta(CURRENT_FORMAT)?)
}

/// Split one [`CURRENT_FORMAT`] record.
///
/// A missing field arrives as an empty string rather than being omitted, so the
/// positions are fixed — but a player that publishes no title has nothing worth
/// showing at all, and that is the one field whose absence hides the pill.
fn parse_current(line: &str) -> Option<NowPlaying> {
    let mut parts = line.split('\x1f');
    let status = parts.next().unwrap_or("");
    let title = parts.next().unwrap_or("").trim().to_string();
    let artist = parts.next().unwrap_or("").trim().to_string();
    let position = micros(parts.next().unwrap_or(""));
    let length = micros(parts.next().unwrap_or(""));
    let art_url = parts.next().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return None;
    }
    Some(NowPlaying {
        title,
        artist,
        playing: status.trim() == "Playing",
        position,
        length,
        art_url,
    })
}

/// The rest of the metadata, for the popover only.
#[derive(Default)]
pub struct Detail {
    pub album: String,
    pub player: String,
    pub shuffle: bool,
    /// MPRIS `LoopStatus`: `None`, `Track` or `Playlist`.
    pub loop_status: String,
}

pub fn detail() -> Detail {
    let mut d = Detail::default();
    if let Some(line) = meta("{{album}}\x1f{{playerName}}") {
        let mut p = line.split('\x1f');
        d.album = p.next().unwrap_or("").trim().to_string();
        d.player = p.next().unwrap_or("").trim().to_string();
    }
    // Both are separate verbs, not metadata fields, and both are optional in
    // the MPRIS spec — a player that does not implement them errors out here
    // and simply leaves the chip in its off state.
    d.shuffle = run(&["shuffle"]).map(|s| s.trim() == "On").unwrap_or(false);
    d.loop_status = run(&["loop"]).map(|s| s.trim().to_string()).unwrap_or_default();
    d
}

/// Toggle play/pause on the active player.
pub fn play_pause() {
    let _ = Command::new("playerctl").arg("play-pause").status();
}

pub fn next() {
    let _ = Command::new("playerctl").arg("next").status();
}

pub fn previous() {
    let _ = Command::new("playerctl").arg("previous").status();
}

pub fn toggle_shuffle() {
    let _ = Command::new("playerctl").args(["shuffle", "toggle"]).status();
}

/// Advance `None → Playlist → Track → None`, the order the MPRIS spec lists.
pub fn cycle_loop(current: &str) -> &'static str {
    let next = next_loop(current);
    let _ = Command::new("playerctl").args(["loop", next]).status();
    next
}

/// The successor in the loop cycle. Anything unrecognised — including the empty
/// string a player without `LoopStatus` returns — falls back to `None`, so the
/// first click always lands on a defined state.
fn next_loop(current: &str) -> &'static str {
    match current.trim() {
        "None" => "Playlist",
        "Playlist" => "Track",
        _ => "None",
    }
}

/// Nudge the position by `secs` (negative rewinds).
pub fn seek(secs: i32) {
    let arg = if secs >= 0 { format!("{secs}+") } else { format!("{}-", -secs) };
    let _ = Command::new("playerctl").args(["position", &arg]).status();
}

/// `m:ss`, or `h:mm:ss` past an hour — how every media player writes a time.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn meta(format: &str) -> Option<String> {
    let line = run(&["metadata", "--format", format])?;
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("playerctl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// MPRIS reports both position and length in microseconds; players that have
/// neither emit an empty field rather than a zero.
fn micros(field: &str) -> Option<u64> {
    let v: f64 = field.trim().parse().ok()?;
    (v > 0.0).then(|| (v / 1_000_000.0) as u64)
}

/// The side we decode cover art to, in px.
///
/// Twice the 26px the pill draws it at, so the thumbnail stays crisp if the
/// output is ever scaled, and small enough that the texture cannot drive layout
/// — see [`art_texture`].
pub const ART_MAX_PX: i32 = 52;

/// Cover art, when the player publishes it as a local file.
///
/// `mpris:artUrl` is frequently an `http(s)` URL for a streaming client. We do
/// not fetch those: the bar's two network-touching modules are opt-in and
/// documented, and album art is not worth making a third.
///
/// Returned centre-cropped to a square and scaled down, which is a layout
/// constraint rather than an optimisation. A widget showing a paintable reports
/// the paintable's intrinsic size as its natural size, and a layer-shell surface
/// is sized to its content — so a full-resolution cover makes the pill as large
/// as the artwork and the whole bar grows to fit. One player publishing a 480px
/// thumbnail was enough to take a 40px bar to 93px. A size request does not
/// help: that raises the minimum, and it is the natural size that wins.
///
/// Cropping here rather than letting the widget do it keeps the square the
/// design asks for without the pill having to letterbox a 16:9 thumbnail.
pub fn art_texture(url: &str) -> Option<gtk4::gdk::Texture> {
    use gtk4::gdk_pixbuf::{InterpType, Pixbuf};
    let path = url.strip_prefix("file://")?;
    let full = Pixbuf::from_file(path).ok()?;

    // Centre square — the subject of a cover is centred far more often than it
    // is in a corner, and a 26px pill has no room to be clever about it.
    let side = full.width().min(full.height());
    let square = Pixbuf::new_subpixbuf(
        &full,
        (full.width() - side) / 2,
        (full.height() - side) / 2,
        side,
        side,
    );
    let scaled = square.scale_simple(ART_MAX_PX, ART_MAX_PX, InterpType::Bilinear)?;
    Some(gtk4::gdk::Texture::for_pixbuf(&scaled))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cover art must come back square and small whatever the player published,
    /// because the pill's size — and through it the bar's height — follows the
    /// texture's own dimensions. A landscape thumbnail once took the bar to
    /// 93px, so this is a layout regression test, not an image-quality one.
    #[test]
    fn cover_art_is_cropped_square_and_capped_whatever_shape_it_arrives_in() {
        use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
        use gtk4::prelude::TextureExt;
        let dir = std::env::temp_dir().join("tezca-art-test");
        std::fs::create_dir_all(&dir).unwrap();

        for (w, h) in [(150, 83), (83, 150), (600, 600), (20, 12)] {
            let path = dir.join(format!("cover-{w}x{h}.png"));
            let pb = Pixbuf::new(Colorspace::Rgb, false, 8, w, h).unwrap();
            pb.fill(0x336699ff);
            pb.savev(&path, "png", &[]).unwrap();

            let tex = art_texture(&format!("file://{}", path.display()))
                .unwrap_or_else(|| panic!("{w}x{h} produced no texture"));
            assert_eq!(
                (tex.width(), tex.height()),
                (ART_MAX_PX, ART_MAX_PX),
                "a {w}x{h} cover must still decode to a {ART_MAX_PX}px square"
            );
            std::fs::remove_file(&path).ok();
        }
    }

    /// Anything that is not a local file is left alone — the bar does not fetch
    /// album art over the network.
    #[test]
    fn remote_art_is_not_fetched() {
        assert!(art_texture("https://example.com/cover.jpg").is_none());
        assert!(art_texture("").is_none());
    }

    #[test]
    fn times_read_as_a_player_writes_them() {
        assert_eq!(clock(134), "2:14");
        assert_eq!(clock(9), "0:09");
        assert_eq!(clock(3725), "1:02:05");
    }

    #[test]
    fn an_absent_position_leaves_the_artist_alone() {
        let mut np = NowPlaying {
            title: "Teotihuacan".into(),
            artist: "Rodrigo Amarante".into(),
            playing: true,
            position: Some(134),
            length: Some(318),
            art_url: String::new(),
        };
        assert_eq!(np.subtitle(), "Rodrigo Amarante \u{00B7} 2:14");
        np.position = None;
        assert_eq!(np.subtitle(), "Rodrigo Amarante");
        np.artist.clear();
        np.position = Some(134);
        assert_eq!(np.subtitle(), "2:14");
    }

    #[test]
    fn a_paused_track_reads_back_whole() {
        let np = parse_current(
            "Paused\x1fTeotihuacan\x1fRodrigo Amarante\x1f134000000\x1f318000000\x1ffile:///t/a.jpg",
        )
        .expect("a titled record is a track");
        assert_eq!(np.title, "Teotihuacan");
        assert_eq!(np.artist, "Rodrigo Amarante");
        assert!(!np.playing);
        assert_eq!((np.position, np.length), (Some(134), Some(318)));
        assert_eq!(np.art_url, "file:///t/a.jpg");
        assert_eq!(np.subtitle(), "Rodrigo Amarante \u{00B7} 2:14");
    }

    #[test]
    fn a_live_stream_has_no_length_and_a_titleless_player_is_not_a_track() {
        // Internet radio: playing, position counts up, length is 0, no cover.
        let np = parse_current("Playing\x1fSomaFM\x1f\x1f42000000\x1f0\x1f").expect("has a title");
        assert!(np.playing);
        assert_eq!(np.length, None);
        assert!(np.art_url.is_empty());
        assert_eq!(np.subtitle(), "0:42");
        // A player that is up but holds no track publishes empty fields.
        assert!(parse_current("Stopped\x1f\x1f\x1f\x1f\x1f").is_none());
    }

    #[test]
    fn micros_rejects_the_zero_a_streams_length_reports() {
        assert_eq!(micros("134000000"), Some(134));
        assert_eq!(micros("0"), None);
        assert_eq!(micros(""), None);
    }

    #[test]
    fn the_loop_cycle_returns_to_none_and_tolerates_a_silent_player() {
        assert_eq!(next_loop("None"), "Playlist");
        assert_eq!(next_loop("Playlist"), "Track");
        assert_eq!(next_loop("Track"), "None");
        assert_eq!(next_loop(""), "None");
    }
}
