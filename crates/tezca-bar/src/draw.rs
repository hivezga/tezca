//! Self-drawn bits — the pieces a config bar can't render, painted with cairo
//! from the live [`Palette`]: the CPU/MEM sparklines, the now-playing
//! equaliser, and the Mayan workspace numerals. Each is a
//! [`gtk4::DrawingArea`]; the sparkline and equaliser own a little state
//! (history buffer / animation phase). All read a shared `Rc<RefCell<Palette>>`
//! so a theme reload repaints them.
//!
//! The status icons live in [`crate::icon`] instead — they are the design's own
//! path data rather than shapes composed here.

use crate::theme::Palette;
use gtk4::cairo::Context;
use gtk4::glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::DrawingArea;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::f64::consts::PI;
use std::rc::Rc;

/// Shared, hot-swappable palette handle.
pub type SharedPalette = Rc<RefCell<Palette>>;

fn set_src(cr: &Context, c: gtk4::gdk::RGBA, a: f64) {
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64 * a);
}

/// Which theme token a sparkline strokes with — one per metric so CPU, MEM, and
/// GPU read apart at a glance while all staying theme-driven.
#[derive(Clone, Copy)]
pub enum SparkColor {
    Accent,    // CPU
    Gold,      // MEM
    AccentDim, // GPU
}

/// A live sparkline. Returns the area and its history buffer; push a value in
/// [0,1] and call `area.queue_draw()` to advance it. `color` selects the stroke
/// token, matching CPU / MEM / GPU.
pub struct Sparkline {
    pub area: DrawingArea,
    pub history: Rc<RefCell<VecDeque<f64>>>,
}

const SPARK_POINTS: usize = 24;

pub fn sparkline(pal: &SharedPalette, color: SparkColor) -> Sparkline {
    let area = DrawingArea::new();
    area.set_content_width(26);
    area.set_content_height(13);
    area.set_valign(gtk4::Align::Center);
    let history: Rc<RefCell<VecDeque<f64>>> = Rc::new(RefCell::new(VecDeque::new()));

    let pal_c = pal.clone();
    let hist_c = history.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let hist = hist_c.borrow();
        if hist.len() < 2 {
            return;
        }
        let p = pal_c.borrow();
        let col = match color {
            SparkColor::Accent => p.accent,
            SparkColor::Gold => p.gold,
            SparkColor::AccentDim => p.accent_dim,
        };
        let (w, h) = (w as f64, h as f64);
        let n = hist.len();
        let dx = w / (n - 1) as f64;
        let xy = |i: usize, v: f64| (i as f64 * dx, h - v.clamp(0.0, 1.0) * (h - 1.0) - 0.5);

        // Stroke only — no area under it. The design's sparkline is a bare
        // `<polyline>` on an `fill="none"` svg, and the wash this used to lay
        // down did not read as a subtle tint at bar size: against the glass it
        // filled in as a solid block, so a steady metric showed up as a bar
        // rather than as the flat trace it actually is.
        for (i, &v) in hist.iter().enumerate() {
            let (x, y) = xy(i, v);
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        set_src(cr, col, 1.0);
        cr.set_line_width(1.3);
        cr.set_line_join(gtk4::cairo::LineJoin::Round);
        cr.set_line_cap(gtk4::cairo::LineCap::Round);
        let _ = cr.stroke();
    });

    Sparkline { area, history }
}

impl Sparkline {
    /// Append a sample and repaint.
    pub fn push(&self, v: f64) {
        let mut h = self.history.borrow_mut();
        h.push_back(v);
        while h.len() > SPARK_POINTS {
            h.pop_front();
        }
        drop(h);
        self.area.queue_draw();
    }
}

/// The design's `tzeq`: a 1.1s cycle between [`EQ_MIN_H`] and [`EQ_MAX_H`], the
/// four bars staggered a fifth of a period apart.
const EQ_PERIOD: f64 = 1.1;
const EQ_DELAYS: [f64; 4] = [0.0, 0.18, 0.36, 0.54];
const EQ_BAR_W: f64 = 2.0;
const EQ_GAP: f64 = 1.5;
const EQ_MIN_H: f64 = 3.0;
const EQ_MAX_H: f64 = 11.0;

/// The 4-bar now-playing equaliser — self-animating on the frame clock while
/// mapped.
pub fn equalizer(pal: &SharedPalette) -> DrawingArea {
    let area = DrawingArea::new();
    // Four bars and the three gaps between them: 4·2 + 3·1.5.
    area.set_content_width((EQ_BAR_W * 4.0 + EQ_GAP * 3.0).ceil() as i32);
    area.set_content_height(EQ_MAX_H as i32);
    area.set_valign(gtk4::Align::Center);

    let phase = Rc::new(RefCell::new(0.0_f64));
    let pal_c = pal.clone();
    let phase_c = phase.clone();
    area.set_draw_func(move |_, cr, _w, h| {
        let p = pal_c.borrow();
        let t = *phase_c.borrow();
        let h = h as f64;
        set_src(cr, p.accent, 1.0);
        for (i, delay) in EQ_DELAYS.iter().enumerate() {
            // A raised cosine — 0 at the top of the cycle, 1 at the half —
            // which is the shape CSS traces alternating a `ease-in-out` between
            // two keyframes. The delay *subtracts* because a CSS
            // `animation-delay` puts a bar behind the one before it, so the
            // wave travels left to right the way the mock does.
            let s = (1.0 - (((t - delay) / EQ_PERIOD) * 2.0 * PI).cos()) * 0.5;
            let bh = EQ_MIN_H + s * (EQ_MAX_H - EQ_MIN_H);
            let x = i as f64 * (EQ_BAR_W + EQ_GAP);
            let y = h - bh;
            rounded_rect(cr, x, y, EQ_BAR_W, bh, 1.0);
            let _ = cr.fill();
        }
    });

    // Drive the animation from the frame clock (paused automatically when the
    // pill is hidden/unmapped).
    let phase_t = phase.clone();
    area.add_tick_callback(move |a, clock| {
        *phase_t.borrow_mut() = clock.frame_time() as f64 / 1_000_000.0;
        a.queue_draw();
        ControlFlow::Continue
    });
    area
}

// ---------------------------------------------------------------------------
// Mayan numerals
// ---------------------------------------------------------------------------

/// Widest a numeral gets: four dots at [`DOT`]px with [`DOT_GAP`]px between them.
const DOT: f64 = 3.0;
const DOT_GAP: f64 = 2.5;
const BAR_W: f64 = 13.0;
const BAR_H: f64 = 2.5;
const ROW_GAP: f64 = 2.0;
/// Fixed height, so every numeral centres on the same line; the *width* follows
/// the numeral, because the design's pill is sized by its content against a
/// 26px floor. A fixed 20px box made a one-dot pill 43px where the mock's is 26.
const NUMERAL_H: i32 = 18;

/// How wide `value` draws — the wider of its dot row and its bar row.
fn numeral_width(value: i32) -> f64 {
    let dots = value % 5;
    let dot_row = if dots > 0 { dots as f64 * DOT + DOT_GAP * (dots - 1) as f64 } else { 0.0 };
    let bar_row = if value / 5 > 0 { BAR_W } else { 0.0 };
    dot_row.max(bar_row)
}

/// The largest value that has a bar-and-dot form here. Mayan is vigesimal, so
/// past this a numeral becomes a stack of positional digits — three rows of
/// bars-and-dots for a single workspace, which is unreadable at 26px. Callers
/// fall back to the digit above this.
pub const MAYAN_MAX: i32 = 19;

/// A Mayan bar-and-dot numeral, drawn rather than typed.
///
/// The Unicode Mayan Numerals block (U+1D2E0…) needs Noto Sans Mayan Numerals
/// installed, and a workspace pill that renders as a tofu box on a machine
/// without it is worse than no feature at all. The glyphs are four rectangles
/// and some circles — drawing them costs less than depending on a font.
///
/// Colour comes from the widget's own CSS `color`, so the numeral follows the
/// pill through idle → occupied → active without knowing those states exist.
pub fn mayan_numeral(value: i32) -> DrawingArea {
    let area = DrawingArea::new();
    area.set_content_width(numeral_width(value).ceil() as i32);
    area.set_content_height(NUMERAL_H);
    area.set_valign(gtk4::Align::Center);
    area.set_halign(gtk4::Align::Center);

    area.set_draw_func(move |a, cr, w, h| {
        let (bars, dots) = (value / 5, value % 5);
        if !(1..=MAYAN_MAX).contains(&value) {
            return;
        }
        let c = a.color();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);

        // Dots ride above the bars, which is how the numerals are written.
        let rows = usize::from(dots > 0) + bars as usize;
        let total = if dots > 0 { DOT } else { 0.0 }
            + bars as f64 * BAR_H
            + ROW_GAP * rows.saturating_sub(1) as f64;
        let (cx, mut y) = (w as f64 / 2.0, (h as f64 - total) / 2.0);

        if dots > 0 {
            let span = dots as f64 * DOT + DOT_GAP * (dots - 1) as f64;
            let mut x = cx - span / 2.0;
            for _ in 0..dots {
                cr.arc(x + DOT / 2.0, y + DOT / 2.0, DOT / 2.0, 0.0, 2.0 * PI);
                let _ = cr.fill();
                x += DOT + DOT_GAP;
            }
            y += DOT + ROW_GAP;
        }
        for _ in 0..bars {
            rounded_rect(cr, cx - BAR_W / 2.0, y, BAR_W, BAR_H, BAR_H / 2.0);
            let _ = cr.fill();
            y += BAR_H + ROW_GAP;
        }
    });
    area
}

/// Trace a rounded rectangle path (cairo has no primitive).
fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}
