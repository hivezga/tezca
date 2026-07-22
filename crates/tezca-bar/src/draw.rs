//! Self-drawn bits — the pieces a config bar can't render, painted with cairo
//! from the live [`Palette`]: the Tezca "mirror" glyph, the CPU/MEM sparklines,
//! and the now-playing equaliser. Each is a [`gtk4::DrawingArea`]; the sparkline
//! and equaliser own a little state (history buffer / animation phase). All read
//! a shared `Rc<RefCell<Palette>>` so a theme reload repaints them.

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

/// The rotated obsidian-mirror square with an accent gradient + glow. `edge` is
/// the square's side length in px; the area is sized generously so the glow room
/// isn't clipped.
pub fn mirror_glyph(pal: &SharedPalette, edge: f64) -> DrawingArea {
    let area = DrawingArea::new();
    let box_side = (edge * 2.0).ceil() as i32;
    area.set_content_width(box_side);
    area.set_content_height(box_side);
    let pal = pal.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let p = pal.borrow();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);

        // Soft glow behind the square.
        let glow = gtk4::cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, edge * 0.95);
        let a = p.accent;
        glow.add_color_stop_rgba(0.0, a.red() as f64, a.green() as f64, a.blue() as f64, 0.55);
        glow.add_color_stop_rgba(1.0, a.red() as f64, a.green() as f64, a.blue() as f64, 0.0);
        let _ = cr.set_source(&glow);
        cr.arc(cx, cy, edge * 0.95, 0.0, 2.0 * PI);
        let _ = cr.fill();

        // Rotated gradient square (135° accent → accent_dim).
        cr.save().ok();
        cr.translate(cx, cy);
        cr.rotate(PI / 4.0);
        let half = edge / 2.0;
        let lin = gtk4::cairo::LinearGradient::new(-half, -half, half, half);
        let ad = p.accent_dim;
        lin.add_color_stop_rgba(0.0, a.red() as f64, a.green() as f64, a.blue() as f64, 1.0);
        lin.add_color_stop_rgba(1.0, ad.red() as f64, ad.green() as f64, ad.blue() as f64, 1.0);
        let _ = cr.set_source(&lin);
        rounded_rect(cr, -half, -half, edge, edge, 3.0);
        let _ = cr.fill();
        cr.restore().ok();
    });
    area
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
    area.set_content_width(34);
    area.set_content_height(14);
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

        // Filled area under the line.
        cr.move_to(0.0, h);
        for (i, &v) in hist.iter().enumerate() {
            let (x, y) = xy(i, v);
            cr.line_to(x, y);
        }
        cr.line_to(w, h);
        cr.close_path();
        set_src(cr, col, 0.14);
        let _ = cr.fill();

        // Stroke on top.
        for (i, &v) in hist.iter().enumerate() {
            let (x, y) = xy(i, v);
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        set_src(cr, col, 1.0);
        cr.set_line_width(1.4);
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

/// The 4-bar now-playing equaliser — self-animating on the frame clock while
/// mapped. Heights breathe 4↔13px on staggered phases, exactly like the mock.
pub fn equalizer(pal: &SharedPalette) -> DrawingArea {
    let area = DrawingArea::new();
    area.set_content_width(18);
    area.set_content_height(14);
    area.set_valign(gtk4::Align::Center);

    let phase = Rc::new(RefCell::new(0.0_f64));
    let pal_c = pal.clone();
    let phase_c = phase.clone();
    area.set_draw_func(move |_, cr, _w, h| {
        let p = pal_c.borrow();
        let t = *phase_c.borrow();
        let h = h as f64;
        let bar_w = 2.5;
        let gap = 2.5;
        let offsets = [0.0, 0.2, 0.45, 0.65];
        set_src(cr, p.accent, 1.0);
        for (i, off) in offsets.iter().enumerate() {
            // 0.9s period; map sine to a 4..13px height.
            let s = ((t / 0.9 + off) * 2.0 * PI).sin() * 0.5 + 0.5;
            let bh = 4.0 + s * 9.0;
            let x = i as f64 * (bar_w + gap);
            let y = h - bh;
            rounded_rect(cr, x, y, bar_w, bh, 1.0);
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
