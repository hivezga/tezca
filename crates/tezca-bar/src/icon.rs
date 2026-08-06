//! Line icons, drawn — the bar's answer to the design's `<svg>` set.
//!
//! Every icon in the mock is a 24×24 SVG stroked at `1.7` with round caps and
//! joins. The bar was drawing Nerd Font Material glyphs in their place, and that
//! is the single largest reason the strip did not read like the prototype: those
//! glyphs are *filled* shapes rather than stroked ones, each was drawn to its
//! own optical weight, and their size follows font metrics instead of a shared
//! grid. Twenty of them in a row read as twenty icons from twenty sets.
//!
//! So the icons are traced here instead, from the mock's own path data. Keeping
//! the literal `d` strings — rather than hand-deriving arc centres into cairo
//! calls — is the point: an icon can be diffed against `BarStrip.dc.html`
//! character by character, and a design revision is a copy-paste. [`trace`] is
//! the small SVG evaluator that costs.
//!
//! Colour comes from the widget's own CSS `color`, exactly as [`crate::draw`]'s
//! Mayan numerals do, so an icon follows its module through
//! idle → warn → crit → urgent without knowing those states exist. The few parts
//! the design paints with a fixed token instead — the mirror's accent, the
//! weather sun's gold — name it via [`Ink`].
//!
//! This also drops a runtime dependency: a bar rendered on a machine without a
//! Nerd Font installed used to show tofu boxes where its status icons were.

use crate::draw::SharedPalette;
use gtk4::cairo::{Context, LineCap, LineJoin};
use gtk4::prelude::*;
use gtk4::DrawingArea;
use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

/// Which colour a sub-path takes. `Current` is the CSS `color` the module gives
/// it — the default, and what keeps the state classes working.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ink {
    Current,
    Accent,
    Gold,
    /// The bar's own background, for a shape that has to *occlude* — the
    /// weather cloud sits over the sun's rays.
    Base,
}

/// One sub-path: the design's `d`, its ink, and whether it is filled or stroked.
struct Part {
    d: &'static str,
    ink: Ink,
    fill: bool,
}

const fn stroke(d: &'static str) -> Part {
    Part { d, ink: Ink::Current, fill: false }
}
const fn stroke_in(d: &'static str, ink: Ink) -> Part {
    Part { d, ink, fill: false }
}
const fn fill(d: &'static str) -> Part {
    Part { d, ink: Ink::Current, fill: true }
}
const fn fill_in(d: &'static str, ink: Ink) -> Part {
    Part { d, ink, fill: true }
}

/// An icon's box and stroke weight. `w`/`h` are the design's `width`/`height`
/// attributes; the 24-unit grid is fitted into the smaller of the two and
/// centred, which is what an SVG with the default `preserveAspectRatio` does
/// (the battery is the only one that is not square).
struct Spec {
    w: i32,
    h: i32,
    sw: f64,
    parts: &'static [Part],
}

/// Every icon the bar strip and the OSD draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Mirror,
    GameMode,
    Camera,
    Mic,
    Recording,
    Caffeine,
    Night,
    AiUsage,
    Weather,
    Wifi,
    Ethernet,
    Disconnected,
    Bluetooth,
    BluetoothOff,
    BluetoothConnected,
    VolumeLow,
    VolumeMid,
    VolumeHigh,
    VolumeMuted,
    Brightness,
    Battery,
    BatteryCharging,
    Bell,
    BellUnread,
    ChevronDown,
    /// The "fold this cluster back up" mark — the design's inward corners.
    Collapse,
    Power,
    // The transport, which the design draws *filled* rather than stroked — a
    // media control is a solid target, not a diagram, and the mock's own
    // now-playing panel switches language here.
    TransportPrev,
    TransportPlay,
    TransportPause,
    TransportNext,
}

// The path data below is the design's, verbatim from `BarStrip.dc.html` where
// the mock has the icon. The handful it does not cover — a wired link, the two
// Bluetooth variants, the quieter volume steps, a gamepad — are drawn in the
// same language: 24-unit grid, one weight, round ends.

/// The speaker cone every volume step starts from.
const SPEAKER: &str = "M11 5 6.5 9H3v6h3.5L11 19z";
/// The nearer arc — one wave.
const WAVE_1: &str = "M15 9.8a3.2 3.2 0 0 1 0 4.4";
/// The farther arc, added for the loudest step (the design's OSD shows both).
const WAVE_2: &str = "M17.6 7.4a6.8 6.8 0 0 1 0 9.2";
/// The bell body, shared by the read and unread states.
const BELL: &str = "M6.5 10a5.5 5.5 0 0 1 11 0c0 4 1.5 5.5 1.5 5.5H5s1.5-1.5 1.5-5.5z";
const BELL_CLAPPER: &str = "M10 18.5a2.2 2.2 0 0 0 4 0";
/// The Bluetooth rune — one stroke, drawn as the design draws it.
const BT: &str = "M7 7.5 17 16.5 12 21V3l5 4.5L7 16.5";
/// A slash across the whole box, for the two "off" states.
const SLASH: &str = "M3.6 3.6 20.4 20.4";

fn spec(icon: Icon) -> Spec {
    use Icon::*;
    match icon {
        // The obsidian mirror: a ring, a filled pupil, and four cardinal ticks.
        // This replaces a rotated gradient square — the mock and the settings
        // window's header mark are both this ring, and the bar was the only
        // surface still wearing the old shape.
        Mirror => Spec {
            w: 17,
            h: 17,
            sw: 1.7,
            parts: &const {
                [
                    stroke_in("M3 12a9 9 0 1 0 18 0a9 9 0 1 0-18 0", Ink::Accent),
                    fill_in("M8.6 12a3.4 3.4 0 1 0 6.8 0a3.4 3.4 0 1 0-6.8 0", Ink::Accent),
                    stroke_in("M12 3v2.6M12 18.4V21M3 12h2.6M18.4 12H21", Ink::Accent),
                ]
            },
        },
        GameMode => {
            Spec {
                w: 16,
                h: 16,
                sw: 1.7,
                parts: &const {
                    [
                stroke("M6 9h12a4 4 0 0 1 4 4v2a4 4 0 0 1-4 4H6a4 4 0 0 1-4-4v-2a4 4 0 0 1 4-4z"),
                stroke("M7 12.5v3M5.5 14h3"),
                fill("M15.2 13a1 1 0 1 0 2 0a1 1 0 1 0-2 0"),
                fill("M17.6 15.6a1 1 0 1 0 2 0a1 1 0 1 0-2 0"),
            ]
                },
            }
        }
        Camera => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                stroke("M15 9.5 21 6v12l-6-3.5z"),
                stroke("M5.5 6h7a2.5 2.5 0 0 1 2.5 2.5v7a2.5 2.5 0 0 1-2.5 2.5h-7A2.5 2.5 0 0 1 3 15.5v-7A2.5 2.5 0 0 1 5.5 6z"),
            ]
            },
        },
        Mic => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                    stroke("M12 3a3 3 0 0 1 3 3v5a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3z"),
                    stroke("M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v3"),
                ]
            },
        },
        // The design draws this one as a plain dot rather than an outline: a
        // recording light is a lamp, not a symbol.
        Recording => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const { [fill("M6 12a6 6 0 1 0 12 0a6 6 0 1 0-12 0")] },
        },
        Caffeine => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                    stroke("M5 8h11v5a5.5 5.5 0 0 1-11 0z"),
                    stroke("M16 9h1.8a2.2 2.2 0 0 1 0 4.4H16M5 20h11"),
                ]
            },
        },
        Night => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const { [stroke("M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z")] },
        },
        // Current, not the mock's fixed gold: this module already speaks the
        // bar's three-step language (subtext → gold at warn → urgent at crit),
        // and pinning the star gold would mute the one state worth seeing.
        AiUsage => {
            Spec {
                w: 15,
                h: 15,
                sw: 1.7,
                parts: &const {
                    [stroke("M12 3.5 14.3 9l5.7.6-4.3 3.9 1.2 5.6L12 16.3 7.1 19.1l1.2-5.6L4 9.6 9.7 9z")]
                },
            }
        }
        // The sun keeps its gold — that is decoration, not a threshold, and it
        // colours no number. The cloud is filled with the bar's own background
        // so it occludes the rays behind it, exactly as the mock does.
        Weather => Spec {
            w: 17,
            h: 17,
            sw: 1.7,
            parts: &const {
                [
                stroke_in("M5.3 8.5a3.2 3.2 0 1 0 6.4 0a3.2 3.2 0 1 0-6.4 0", Ink::Gold),
                stroke_in(
                    "M8.5 2.6v1.4M8.5 13v1.4M2.6 8.5H4M13 8.5h1.4M4.3 4.3l1 1M11.7 11.7l1 1M12.7 4.3l-1 1M5.3 11.7l-1 1",
                    Ink::Gold,
                ),
                fill_in("M9 20.5h8.6a3.1 3.1 0 0 0 .3-6.2 4.4 4.4 0 0 0-8.5-.7A2.9 2.9 0 0 0 9 20.5z", Ink::Base),
                stroke("M9 20.5h8.6a3.1 3.1 0 0 0 .3-6.2 4.4 4.4 0 0 0-8.5-.7A2.9 2.9 0 0 0 9 20.5z"),
            ]
            },
        },
        Wifi => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                    stroke("M4.5 10.5a11 11 0 0 1 15 0M8 14a6 6 0 0 1 8 0"),
                    fill("M10.9 18a1.1 1.1 0 1 0 2.2 0a1.1 1.1 0 1 0-2.2 0"),
                ]
            },
        },
        // Not in the mock — its bar is on Wi-Fi. The conventional wired mark, in
        // the same weight: a node above a bus, two nodes below it.
        Ethernet => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                    stroke("M9 3h6v4H9z"),
                    stroke("M2.5 17h6v4h-6z"),
                    stroke("M15.5 17h6v4h-6z"),
                    stroke("M12 7v6.5M5.5 17v-3.5h13V17"),
                ]
            },
        },
        Disconnected => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                    stroke("M4.5 10.5a11 11 0 0 1 15 0M8 14a6 6 0 0 1 8 0"),
                    fill("M10.9 18a1.1 1.1 0 1 0 2.2 0a1.1 1.1 0 1 0-2.2 0"),
                    stroke(SLASH),
                ]
            },
        },
        Bluetooth => Spec { w: 15, h: 15, sw: 1.7, parts: &const { [stroke(BT)] } },
        BluetoothOff => {
            Spec { w: 15, h: 15, sw: 1.7, parts: &const { [stroke(BT), stroke(SLASH)] } }
        }
        // Connected adds the pair of side ticks the radio spec's own logo uses —
        // "on" and "in use" have to read apart, which is the module's whole job.
        BluetoothConnected => Spec {
            w: 15,
            h: 15,
            sw: 1.7,
            parts: &const { [stroke(BT), stroke("M2.4 12h1.8M19.8 12h1.8")] },
        },
        VolumeLow => Spec { w: 16, h: 16, sw: 1.7, parts: &const { [stroke(SPEAKER)] } },
        VolumeMid => {
            Spec { w: 16, h: 16, sw: 1.7, parts: &const { [stroke(SPEAKER), stroke(WAVE_1)] } }
        }
        VolumeHigh => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const { [stroke(SPEAKER), stroke(WAVE_1), stroke(WAVE_2)] },
        },
        VolumeMuted => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const { [stroke(SPEAKER), stroke("M15.5 9.5 20.5 14.5M20.5 9.5 15.5 14.5")] },
        },
        Brightness => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                stroke("M8 12a4 4 0 1 0 8 0a4 4 0 1 0-8 0"),
                stroke("M12 2.5v2.2M12 19.3v2.2M2.5 12h2.2M19.3 12h2.2M5.2 5.2l1.6 1.6M17.2 17.2l1.6 1.6M18.8 5.2l-1.6 1.6M6.8 17.2l-1.6 1.6"),
            ]
            },
        },
        Battery => Spec {
            w: 20,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                stroke("M4.5 7h12a2.5 2.5 0 0 1 2.5 2.5v5a2.5 2.5 0 0 1-2.5 2.5h-12A2.5 2.5 0 0 1 2 14.5v-5A2.5 2.5 0 0 1 4.5 7z"),
                stroke("M21.5 10.5v3"),
                fill("M5 9h7a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1z"),
            ]
            },
        },
        // Charging swaps the level for a bolt: the cell is filling, so a fixed
        // level beside a rising percentage would contradict it.
        BatteryCharging => Spec {
            w: 20,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                stroke("M4.5 7h12a2.5 2.5 0 0 1 2.5 2.5v5a2.5 2.5 0 0 1-2.5 2.5h-12A2.5 2.5 0 0 1 2 14.5v-5A2.5 2.5 0 0 1 4.5 7z"),
                stroke("M21.5 10.5v3"),
                fill("M11.4 8.6 6.4 12.4h3l-.8 3 5-3.8h-3z"),
            ]
            },
        },
        Bell => {
            Spec { w: 16, h: 16, sw: 1.7, parts: &const { [stroke(BELL), stroke(BELL_CLAPPER)] } }
        }
        // Unread adds the ringing arcs rather than a second silhouette, so the
        // two states are the same bell and the change is unmistakable.
        BellUnread => Spec {
            w: 16,
            h: 16,
            sw: 1.7,
            parts: &const {
                [
                    stroke(BELL),
                    stroke(BELL_CLAPPER),
                    stroke("M2.6 8a7 7 0 0 1 2.6-4.2M21.4 8a7 7 0 0 0-2.6-4.2"),
                ]
            },
        },
        ChevronDown => Spec { w: 11, h: 11, sw: 2.4, parts: &const { [stroke("m8 10 4 4 4-4")] } },
        Collapse => {
            Spec { w: 12, h: 12, sw: 2.4, parts: &const { [stroke("M4 9h6V3M20 15h-6v6")] } }
        }
        Power => Spec {
            w: 16,
            h: 16,
            sw: 1.8,
            parts: &const { [stroke("M12 3v8.5"), stroke("M6.8 6.6a8 8 0 1 0 10.4 0")] },
        },
        TransportPrev => {
            Spec { w: 17, h: 17, sw: 0.0, parts: &const { [fill("M7 6h2v12H7zM10 12l9-6v12z")] } }
        }
        // The mock only ever draws the pause face; play is its mirror.
        TransportPlay => {
            Spec { w: 21, h: 21, sw: 0.0, parts: &const { [fill("M8 5 19 12 8 19z")] } }
        }
        TransportPause => {
            Spec { w: 21, h: 21, sw: 0.0, parts: &const { [fill("M8 5h3v14H8zM13 5h3v14h-3z")] } }
        }
        TransportNext => {
            Spec { w: 17, h: 17, sw: 0.0, parts: &const { [fill("M15 6h2v12h-2zM14 12 5 6v12z")] } }
        }
    }
}

/// A drawn icon whose glyph can change — the network module's link state, the
/// volume steps, the bell's unread flag.
pub struct IconArea {
    pub area: DrawingArea,
    kind: Rc<Cell<Icon>>,
    glow: Rc<Cell<bool>>,
}

impl IconArea {
    /// Swap the glyph. A no-op when it is already the one showing, so the
    /// per-tick refreshes do not queue a repaint each time round.
    pub fn set(&self, icon: Icon) {
        if self.kind.get() == icon {
            return;
        }
        self.kind.set(icon);
        self.area.queue_draw();
    }

    /// Halo the icon in its own colour.
    ///
    /// The privacy indicators carried `text-shadow: 0 0 8px` in the stylesheet,
    /// which is a property of *text* — it does nothing for a drawn widget, and
    /// losing it would quietly demote "you are on camera" to one more grey icon.
    /// Two widening translucent passes under the icon stand in for the blur.
    pub fn set_glow(&self, on: bool) {
        if self.glow.get() == on {
            return;
        }
        self.glow.set(on);
        self.area.queue_draw();
    }
}

/// Build an icon widget at the design's size.
pub fn icon(pal: &SharedPalette, kind: Icon) -> IconArea {
    build(pal, kind, None, 0)
}

/// An icon with room around it for [`IconArea::set_glow`] to paint into.
///
/// A halo drawn inside the design's own box would be clipped by the widget
/// allocation, so the glowing icons — the privacy family — reserve the
/// difference. Only they pay for it: six transparent pixels on every module
/// would widen the whole cluster.
pub fn glowing_icon(pal: &SharedPalette, kind: Icon) -> IconArea {
    build(pal, kind, None, 6)
}

/// An icon at a size the design sets somewhere other than the strip — the OSD
/// pill's is 22px. Glow room comes with it, since that is the surface where a
/// mute reads as a warning.
pub fn icon_at(pal: &SharedPalette, kind: Icon, px: i32) -> IconArea {
    build(pal, kind, Some(px), 6)
}

/// The box is the design's plus `pad`, and it never resizes when the glyph
/// changes — every icon a module can swap between shares a size, so a module's
/// width does not twitch when its state does.
fn build(pal: &SharedPalette, kind: Icon, size: Option<i32>, pad: i32) -> IconArea {
    let area = DrawingArea::new();
    let s = spec(kind);
    area.set_content_width(size.unwrap_or(s.w) + pad);
    area.set_content_height(size.unwrap_or(s.h) + pad);
    area.set_valign(gtk4::Align::Center);
    area.set_halign(gtk4::Align::Center);

    let cell = Rc::new(Cell::new(kind));
    let glow = Rc::new(Cell::new(false));
    let pal = pal.clone();
    let cell_c = cell.clone();
    let glow_c = glow.clone();
    area.set_draw_func(move |a, cr, w, h| {
        let s = spec(cell_c.get());
        let p = pal.borrow();
        let cur = a.color();
        // The 24-unit grid is scaled by the icon's own size — not by whatever
        // the parent allocated — and centred in the allocation, so glow padding
        // and a stretched box both leave the glyph the size it was drawn at.
        let k = (size.unwrap_or(s.w.min(s.h)) as f64) / 24.0;
        let (ox, oy) = ((w as f64 - 24.0 * k) / 2.0, (h as f64 - 24.0 * k) / 2.0);

        cr.set_line_cap(LineCap::Round);
        cr.set_line_join(LineJoin::Round);
        for part in s.parts {
            let c = match part.ink {
                Ink::Current => cur,
                Ink::Accent => p.accent,
                Ink::Gold => p.gold,
                Ink::Base => p.base,
            };
            let set = |alpha: f64| {
                cr.set_source_rgba(
                    c.red() as f64,
                    c.green() as f64,
                    c.blue() as f64,
                    c.alpha() as f64 * alpha,
                );
            };
            // The halo goes under this part, not under the whole icon, so an
            // occluding fill still covers what it is meant to.
            if glow_c.get() && part.ink != Ink::Base {
                for (width, alpha) in [(4.0, 0.16), (2.4, 0.26)] {
                    cr.set_line_width(s.sw * k * width);
                    set(alpha);
                    cr.new_path();
                    trace(cr, part.d, k, ox, oy);
                    let _ = cr.stroke();
                }
            }
            cr.set_line_width(s.sw * k);
            // The occluding fill takes the bar's own translucency; everything
            // else is drawn at the colour's own alpha.
            set(if part.ink == Ink::Base { 0.70 } else { 1.0 });
            cr.new_path();
            trace(cr, part.d, k, ox, oy);
            let _ = if part.fill { cr.fill() } else { cr.stroke() };
        }
    });

    IconArea { area, kind: cell, glow }
}

// ---------------------------------------------------------------------------
// The SVG path evaluator
// ---------------------------------------------------------------------------

/// Walk an SVG `d` string onto a cairo context, scaled by `k` and offset to the
/// widget's box.
///
/// Supports the subset the icon set uses: `M L H V C Z` in both cases and the
/// elliptical arc `A`/`a`. Anything else is skipped rather than guessed at — a
/// silently mis-drawn icon is worse than a missing one, and every path here is
/// in this file where it can be checked.
fn trace(cr: &Context, d: &str, k: f64, ox: f64, oy: f64) {
    let b = d.as_bytes();
    let mut i = 0usize;
    // Current point and the last sub-path start, both on the 24-unit grid.
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    let mut cmd = b' ';
    let put = |px: f64, py: f64| (ox + px * k, oy + py * k);

    loop {
        skip_sep(b, &mut i);
        if i >= b.len() {
            break;
        }
        // A command letter, or a repeat of the previous one with fresh operands.
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
        }
        let rel = cmd.is_ascii_lowercase();
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (nx, ny) = (num(b, &mut i), num(b, &mut i));
                x = if rel { x + nx } else { nx };
                y = if rel { y + ny } else { ny };
                let (px, py) = put(x, y);
                cr.move_to(px, py);
                (sx, sy) = (x, y);
                // Further pairs after a moveto are implicit linetos.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (nx, ny) = (num(b, &mut i), num(b, &mut i));
                x = if rel { x + nx } else { nx };
                y = if rel { y + ny } else { ny };
                let (px, py) = put(x, y);
                cr.line_to(px, py);
            }
            b'H' => {
                let nx = num(b, &mut i);
                x = if rel { x + nx } else { nx };
                let (px, py) = put(x, y);
                cr.line_to(px, py);
            }
            b'V' => {
                let ny = num(b, &mut i);
                y = if rel { y + ny } else { ny };
                let (px, py) = put(x, y);
                cr.line_to(px, py);
            }
            b'C' => {
                let (x1, y1) = (num(b, &mut i), num(b, &mut i));
                let (x2, y2) = (num(b, &mut i), num(b, &mut i));
                let (nx, ny) = (num(b, &mut i), num(b, &mut i));
                let (bx, by) = if rel { (x, y) } else { (0.0, 0.0) };
                let (c1, c2) = (put(bx + x1, by + y1), put(bx + x2, by + y2));
                x = bx + nx;
                y = by + ny;
                let e = put(x, y);
                cr.curve_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
            }
            b'A' => {
                let (rx, ry) = (num(b, &mut i), num(b, &mut i));
                let rot = num(b, &mut i);
                // The two flags are single characters and may be run together
                // with what follows (`0 010.5`), so they are read as one digit
                // each rather than as numbers.
                let large = flag(b, &mut i);
                let sweep = flag(b, &mut i);
                let (nx, ny) = (num(b, &mut i), num(b, &mut i));
                let (ex, ey) = if rel { (x + nx, y + ny) } else { (nx, ny) };
                arc_to(cr, (x, y), (rx, ry), rot, large, sweep, (ex, ey), k, ox, oy);
                x = ex;
                y = ey;
            }
            b'Z' => {
                cr.close_path();
                x = sx;
                y = sy;
            }
            // Unsupported command: stop rather than emit a wrong shape.
            _ => break,
        }
    }
}

fn skip_sep(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i] == b' ' || b[*i] == b',' || b[*i] == b'\n' || b[*i] == b'\t') {
        *i += 1;
    }
}

/// A single arc flag — exactly one character, per the SVG grammar.
fn flag(b: &[u8], i: &mut usize) -> bool {
    skip_sep(b, i);
    let v = *i < b.len() && b[*i] == b'1';
    if *i < b.len() {
        *i += 1;
    }
    v
}

/// Scan one number. Handles the leading-dot and packed-sign forms the design's
/// minified paths use (`.3-6.2` is two numbers).
fn num(b: &[u8], i: &mut usize) -> f64 {
    skip_sep(b, i);
    let start = *i;
    if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
        *i += 1;
    }
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i < b.len() && b[*i] == b'.' {
        *i += 1;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
    }
    std::str::from_utf8(&b[start..*i]).ok().and_then(|s| s.parse().ok()).unwrap_or(0.0)
}

/// SVG's endpoint-parameterised arc → cairo's centre-parameterised one, by the
/// conversion in SVG 1.1 §F.6.5.
///
/// cairo draws only circles, so the ellipse is produced by scaling the CTM
/// around the computed centre — which is also why this is wrapped in
/// save/restore. The current path survives both (it is not part of the graphics
/// state), so the arc joins the segments either side of it.
#[allow(clippy::too_many_arguments)]
fn arc_to(
    cr: &Context,
    (x1, y1): (f64, f64),
    (rx, ry): (f64, f64),
    rot_deg: f64,
    large: bool,
    sweep: bool,
    (x2, y2): (f64, f64),
    k: f64,
    ox: f64,
    oy: f64,
) {
    // Degenerate radii are a straight line, per the spec.
    if rx == 0.0 || ry == 0.0 || (x1 == x2 && y1 == y2) {
        cr.line_to(ox + x2 * k, oy + y2 * k);
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    let phi = rot_deg.to_radians();
    let (cos_p, sin_p) = (phi.cos(), phi.sin());

    let (dx2, dy2) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let x1p = cos_p * dx2 + sin_p * dy2;
    let y1p = -sin_p * dx2 + cos_p * dy2;

    // Grow radii that are too small to span the chord, rather than failing.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }

    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let co = sign * (num / den).sqrt();
    let cxp = co * rx * y1p / ry;
    let cyp = -co * ry * x1p / rx;
    let cx = cos_p * cxp - sin_p * cyp + (x1 + x2) / 2.0;
    let cy = sin_p * cxp + cos_p * cyp + (y1 + y2) / 2.0;

    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = (ux * vx + uy * vy) / ((ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt());
        let a = dot.clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            -a
        } else {
            a
        }
    };
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta1 = ang(1.0, 0.0, ux, uy);
    let mut sweep_ang = ang(ux, uy, vx, vy);
    if !sweep && sweep_ang > 0.0 {
        sweep_ang -= 2.0 * PI;
    } else if sweep && sweep_ang < 0.0 {
        sweep_ang += 2.0 * PI;
    }

    cr.save().ok();
    cr.translate(ox + cx * k, oy + cy * k);
    cr.rotate(phi);
    cr.scale(rx * k, ry * k);
    if sweep {
        cr.arc(0.0, 0.0, 1.0, theta1, theta1 + sweep_ang);
    } else {
        cr.arc_negative(0.0, 0.0, 1.0, theta1, theta1 + sweep_ang);
    }
    cr.restore().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk a path and record the points it visits, so the evaluator can be
    /// checked without a surface to look at.
    fn endpoint(d: &str) -> (f64, f64) {
        let surface =
            gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, 24, 24).unwrap();
        let cr = Context::new(&surface).unwrap();
        trace(&cr, d, 1.0, 0.0, 0.0);
        cr.current_point().unwrap_or((f64::NAN, f64::NAN))
    }

    #[test]
    fn relative_and_absolute_commands_land_on_the_same_point() {
        let (ax, ay) = endpoint("M4 4L10 4L10 12");
        let (rx, ry) = endpoint("m4 4l6 0l0 8");
        assert!((ax - rx).abs() < 1e-9 && (ay - ry).abs() < 1e-9, "{ax},{ay} vs {rx},{ry}");
        assert!((ax - 10.0).abs() < 1e-9 && (ay - 12.0).abs() < 1e-9);
    }

    /// The form the design's minified paths use — `1-11` is two operands, and a
    /// number scanner that swallowed the sign would draw the wrong shape.
    #[test]
    fn a_packed_sign_separates_two_numbers() {
        let (x, y) = endpoint("M16 13a5.5 5.5 0 0 1-11 0");
        assert!((x - 5.0).abs() < 1e-9 && (y - 13.0).abs() < 1e-9, "{x},{y}");
    }

    /// A moveto's trailing pairs are linetos, not more movetos.
    #[test]
    fn extra_pairs_after_a_moveto_are_implicit_linetos() {
        let (x, y) = endpoint("M15 9.5 21 6");
        assert!((x - 21.0).abs() < 1e-9 && (y - 6.0).abs() < 1e-9, "{x},{y}");
    }

    /// Two half-arcs are how every circle in the set is written; the pen has to
    /// come back to where it started.
    #[test]
    fn a_circle_written_as_two_arcs_closes_on_itself() {
        let (x, y) = endpoint("M6 12a6 6 0 1 0 12 0a6 6 0 1 0-12 0");
        assert!((x - 6.0).abs() < 1e-6 && (y - 12.0).abs() < 1e-6, "{x},{y}");
    }

    /// Every icon has to parse to *something*: an unsupported command aborts the
    /// walk, so a typo would otherwise show up only as a shape nobody drew.
    #[test]
    fn every_icon_path_is_consumed_to_its_end() {
        use Icon::*;
        for k in [
            Mirror,
            GameMode,
            Camera,
            Mic,
            Recording,
            Caffeine,
            Night,
            AiUsage,
            Weather,
            Wifi,
            Ethernet,
            Disconnected,
            Bluetooth,
            BluetoothOff,
            BluetoothConnected,
            VolumeLow,
            VolumeMid,
            VolumeHigh,
            VolumeMuted,
            Brightness,
            Battery,
            BatteryCharging,
            Bell,
            BellUnread,
            ChevronDown,
            Collapse,
            Power,
            TransportPrev,
            TransportPlay,
            TransportPause,
            TransportNext,
        ] {
            for p in spec(k).parts {
                let (x, y) = endpoint(p.d);
                assert!(x.is_finite() && y.is_finite(), "{k:?} left no current point: {}", p.d);
                // Nothing in the set strays outside the 24-unit grid; a number
                // misread as `0` would park the pen at the origin.
                assert!(
                    (-1.0..=25.0).contains(&x) && (-1.0..=25.0).contains(&y),
                    "{k:?} ends off-grid at {x},{y}: {}",
                    p.d
                );
            }
        }
    }
}
