//! Volume on-screen display — the transient glass pill that fades in when the
//! volume goes up, down, or mutes, then fades itself away.
//!
//! Two halves:
//!   * [`subscribe`] runs `pactl subscribe` on a background thread and pings the
//!     GTK loop whenever the default *sink* changes (volume or mute). This makes
//!     the OSD react to ANY source of a change — the volume keys, the bar's own
//!     mixer, `wpctl`, an app — the way macOS does, not just our own keybinds.
//!   * [`Osd`] is one layer-shell overlay window per monitor (built alongside
//!     each bar `Surface`). [`Osd::show`] sets the glyph + level, reveals it with
//!     a CSS opacity fade, and arms an auto-hide timer that re-fades it out.
//!
//! Deliberately event-driven, not polled: the 2-second control tick is far too
//! coarse for an OSD, which has to appear the instant a key is pressed.

use std::cell::RefCell;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, LevelBar, Orientation, Window};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

// Glyphs (MDI, supplementary plane so the Write tool keeps them).
const G_VOL: [&str; 3] = ["\u{F057F}", "\u{F0580}", "\u{F057E}"]; // low / mid / high
const G_MUTED: &str = "\u{F075F}";
const G_BRIGHT: &str = "\u{F00DF}"; // matches the bar's brightness module glyph

/// Watch PipeWire/PulseAudio for default-sink changes on a background thread,
/// pinging `tx` (with no payload — the GTK side re-reads the real volume) each
/// time. Silently does nothing if `pactl` is missing.
pub fn subscribe(tx: async_channel::Sender<()>) {
    std::thread::spawn(move || loop {
        let Ok(mut child) = Command::new("pactl")
            .arg("subscribe")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return; // no pactl → no OSD events (module stays silent)
        };
        let Some(out) = child.stdout.take() else { return };
        let reader = BufReader::new(out);
        for line in reader.lines().map_while(Result::ok) {
            // Lines look like: `Event 'change' on sink #49`. We want master-sink
            // volume/mute only — NOT `sink-input` (per-app streams) and not
            // sources/cards. `on sink #` matches "sink" but not "sink-input".
            if line.contains("on sink #") {
                if tx.send_blocking(()).is_err() {
                    let _ = child.kill();
                    return; // GTK side gone
                }
            }
        }
        // `pactl subscribe` ended (server restart?). Reap and reconnect.
        let _ = child.wait();
    });
}

/// One monitor's volume OSD: a bottom-centred glass pill, hidden until `show`.
pub struct Osd {
    window: Window,
    root: GtkBox,
    glyph: Label,
    level: LevelBar,
    value: Label,
    timeout_ms: u32,
    /// Pending fade-out + hide timers, cancelled and rescheduled on each `show`
    /// so holding volume-up keeps the pill up instead of flickering.
    fade: Rc<RefCell<Option<glib::SourceId>>>,
    hide: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Osd {
    /// Build the overlay window for one monitor. It reserves no space (exclusive
    /// zone 0) and never takes keyboard focus; it just floats above everything.
    pub fn build(
        app: &gtk4::Application,
        monitor: &gtk4::gdk::Monitor,
        timeout_ms: u32,
    ) -> Rc<Osd> {
        let glyph = Label::new(Some(G_VOL[2]));
        glyph.add_css_class("osd-glyph");

        let level = LevelBar::new();
        level.set_min_value(0.0);
        level.set_max_value(100.0);
        level.set_value(0.0);
        level.add_css_class("osd-level");
        level.set_hexpand(true);
        level.set_valign(Align::Center);

        let value = Label::new(Some("0%"));
        value.add_css_class("osd-value");

        let row = GtkBox::new(Orientation::Horizontal, 12);
        row.append(&glyph);
        row.append(&level);
        row.append(&value);

        let root = GtkBox::new(Orientation::Horizontal, 0);
        root.add_css_class("osd");
        root.append(&row);

        let window = Window::builder().application(app).child(&root).build();
        window.add_css_class("tezca-osd");
        window.init_layer_shell();
        window.set_monitor(Some(monitor));
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("tezca-osd"));
        // Anchor to the bottom only → the compositor centres us horizontally.
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, 140);
        window.set_visible(false);

        Rc::new(Osd {
            window,
            root,
            glyph,
            level,
            value,
            timeout_ms,
            fade: Rc::new(RefCell::new(None)),
            hide: Rc::new(RefCell::new(None)),
        })
    }

    /// Reveal the OSD for the given sink state, then arm the auto-hide. Called on
    /// every volume change event; re-calling resets the timer.
    pub fn show(self: &Rc<Self>, volume: u32, muted: bool) {
        if muted {
            self.glyph.set_text(G_MUTED);
            self.value.set_text("Muted");
            self.level.set_value(0.0);
            self.root.add_css_class("muted");
        } else {
            let idx = match volume {
                0 => 0,
                1..=50 => 1,
                _ => 2,
            };
            self.glyph.set_text(G_VOL[idx]);
            self.value.set_text(&format!("{volume}%"));
            self.level.set_value(volume.min(100) as f64);
            self.root.remove_css_class("muted");
        }
        self.flash();
    }

    /// Reveal the OSD for a backlight brightness change (laptop panels). Same
    /// pill as the volume OSD, a sun glyph, no mute concept.
    pub fn show_brightness(self: &Rc<Self>, percent: u32) {
        self.glyph.set_text(G_BRIGHT);
        self.value.set_text(&format!("{percent}%"));
        self.level.set_value(percent.min(100) as f64);
        self.root.remove_css_class("muted");
        self.flash();
    }

    /// Shared reveal + auto-hide: cancel any pending timers, show the pill with
    /// the opacity-fade class, then schedule the fade-out and unmap. Re-calling
    /// resets the dwell so holding a key keeps it up.
    fn flash(self: &Rc<Self>) {
        cancel(&self.fade);
        cancel(&self.hide);

        self.window.set_visible(true);
        self.root.add_css_class("show");

        // Fade out after the dwell, then unmap once the opacity transition ends
        // so a fully-transparent window is never left mapped over the screen.
        let root = self.root.clone();
        let window = self.window.clone();
        let fade_cell = self.fade.clone();
        let hide_cell = self.hide.clone();
        let fade_id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(self.timeout_ms as u64),
            move || {
                *fade_cell.borrow_mut() = None;
                root.remove_css_class("show");
                let window = window.clone();
                let hide_cell2 = hide_cell.clone();
                let hide_id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(260), // ≥ the CSS transition
                    move || {
                        *hide_cell2.borrow_mut() = None;
                        window.set_visible(false);
                    },
                );
                *hide_cell.borrow_mut() = Some(hide_id);
            },
        );
        *self.fade.borrow_mut() = Some(fade_id);
    }
}

/// Cancel a pending timer if one is scheduled (and hasn't already fired).
fn cancel(cell: &Rc<RefCell<Option<glib::SourceId>>>) {
    if let Some(id) = cell.borrow_mut().take() {
        id.remove();
    }
}
