//! The drag-to-arrange display canvas.
//!
//! Monitors are drawn to scale and dragged into place, with their edges snapping
//! to one another — the interaction wdisplays, wlay and nwg-displays all settle
//! on, because typing coordinates into two spin buttons is a bad way to describe
//! "this screen is to the left of that one".
//!
//! ## Why a canvas and not the number boxes
//!
//! The Displays page had `Position (x, y)` inside an expander and one
//! `Right of <other>` button. That is enough to *express* any layout and no help
//! at all in *checking* one: this machine had DP-2 persisted at `0x3440`, which
//! reads like "beside the 3440-wide ultrawide" and actually means 2000 px below
//! its bottom edge, with a dead gap in between. Nothing in the old UI showed the
//! gap. A canvas does, before you apply it.
//!
//! ## Coordinate spaces
//!
//! Everything here is in Hyprland's **logical** space — physical pixels divided
//! by scale, with 90°/270° rotations swapping width and height. That is the
//! space monitor positions are expressed in, so it is the only one where "these
//! two edges touch" is a true statement. Screen coordinates appear solely inside
//! [`View`], which maps logical → widget pixels for drawing and hit-testing.

use gtk4::cairo::{FontSlant, FontWeight};
use gtk4::gdk::RGBA;
use gtk4::prelude::*;
use gtk4::{Align, Box, Button, DrawingArea, EventControllerKey, GestureDrag, Orientation};
use std::cell::RefCell;
use std::rc::Rc;

use crate::backend;

/// How close two edges must come, in *widget* pixels, before one grabs the
/// other. Kept in screen space so the pull feels the same however far the view
/// is zoomed out — a logical-space threshold would be unusable on a 3-monitor
/// span and twitchy on one screen.
const SNAP_SCREEN_PX: f64 = 12.0;

/// Canvas height. Wide-and-short suits the arrangements people actually have
/// (monitors side by side) without eating the page.
const CANVAS_H: i32 = 230;

/// Breathing room around the arrangement, in widget pixels.
const PAD: f64 = 16.0;

/// Called with every monitor whose position changed, as `(name, x, y)`.
///
/// One call for the whole layout rather than one per drag: applying each drop
/// separately would stack a confirm-or-revert dialog per monitor, and a layout
/// is one decision.
pub type Commit = Rc<dyn Fn(Vec<(String, i32, i32)>)>;

/// One monitor in logical coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub name: String,
    pub detail: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub disabled: bool,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.w
    }

    fn bottom(&self) -> i32 {
        self.y + self.h
    }

    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

/// logical → widget-pixel transform for the current allocation.
#[derive(Clone, Copy, Debug)]
struct View {
    scale: f64,
    ox: f64,
    oy: f64,
}

impl Default for View {
    fn default() -> Self {
        View { scale: 1.0, ox: 0.0, oy: 0.0 }
    }
}

impl View {
    fn sx(&self, x: i32) -> f64 {
        x as f64 * self.scale + self.ox
    }

    fn sy(&self, y: i32) -> f64 {
        y as f64 * self.scale + self.oy
    }

    /// Widget pixels back to logical, for hit-testing a click.
    fn lx(&self, x: f64) -> i32 {
        ((x - self.ox) / self.scale).round() as i32
    }

    fn ly(&self, y: f64) -> i32 {
        ((y - self.oy) / self.scale).round() as i32
    }
}

struct State {
    rects: Vec<Rect>,
    /// Positions as first read, so "what changed" is answerable at Apply time
    /// and Revert does not need to re-query the compositor.
    original: Vec<(i32, i32)>,
    selected: Option<usize>,
    /// `(index, logical x, logical y)` captured when a drag began. Deltas are
    /// applied to this rather than accumulated, so a drag that snaps and then
    /// pulls away does not drift.
    dragging: Option<(usize, i32, i32)>,
    view: View,
}

impl State {
    fn changed(&self) -> Vec<(String, i32, i32)> {
        self.rects
            .iter()
            .zip(&self.original)
            .filter(|(r, (ox, oy))| r.x != *ox || r.y != *oy)
            .map(|(r, _)| (r.name.clone(), r.x, r.y))
            .collect()
    }
}

/// Build the arrangement section: the canvas plus its actions.
pub fn canvas(mons: &[backend::Monitor], on_commit: Commit) -> Box {
    let rects = rects_of(mons);
    let original: Vec<(i32, i32)> = rects.iter().map(|r| (r.x, r.y)).collect();
    let state = Rc::new(RefCell::new(State {
        rects,
        original,
        selected: None,
        dragging: None,
        view: View::default(),
    }));

    let wrap = Box::new(Orientation::Vertical, 8);

    let area = DrawingArea::new();
    area.add_css_class("tz-arrange");
    area.set_content_height(CANVAS_H);
    area.set_hexpand(true);
    area.set_focusable(true);

    {
        let state = state.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let mut st = state.borrow_mut();
            st.view = view_for(&st.rects, w as f64, h as f64);
            let view = st.view;
            let colors = Palette::of(area);

            for (i, r) in st.rects.iter().enumerate() {
                let overlapping = st.rects.iter().enumerate().any(|(j, o)| j != i && r.overlaps(o));
                draw_rect(cr, r, &view, &colors, st.selected == Some(i), overlapping);
            }
        });
    }

    // --- drag ---------------------------------------------------------------
    let drag = GestureDrag::new();
    {
        let state = state.clone();
        let area_ref = area.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut st = state.borrow_mut();
            let view = st.view;
            let (lx, ly) = (view.lx(x), view.ly(y));
            // Topmost first: later rects are drawn over earlier ones, so the one
            // you can see is the one you grab.
            let hit = st
                .rects
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| lx >= r.x && lx < r.right() && ly >= r.y && ly < r.bottom())
                .map(|(i, r)| (i, r.x, r.y));
            st.selected = hit.map(|(i, _, _)| i);
            st.dragging = hit;
            drop(st);
            area_ref.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area_ref = area.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let mut st = state.borrow_mut();
            let Some((i, sx, sy)) = st.dragging else { return };
            let view = st.view;
            let want_x = sx + (dx / view.scale).round() as i32;
            let want_y = sy + (dy / view.scale).round() as i32;
            let tol = (SNAP_SCREEN_PX / view.scale).round() as i32;
            let (x, y) = snap(&st.rects, i, want_x, want_y, tol);
            st.rects[i].x = x;
            st.rects[i].y = y;
            drop(st);
            area_ref.queue_draw();
        });
    }
    {
        let state = state.clone();
        drag.connect_drag_end(move |_, _, _| {
            state.borrow_mut().dragging = None;
        });
    }
    area.add_controller(drag);

    // --- keyboard nudge -----------------------------------------------------
    // A drag is fine for "roughly there" and hopeless for "exactly 1 px left".
    let keys = EventControllerKey::new();
    {
        let state = state.clone();
        let area_ref = area.clone();
        keys.connect_key_pressed(move |_, key, _, modifier| {
            use gtk4::gdk::Key;
            let step = if modifier.contains(gtk4::gdk::ModifierType::SHIFT_MASK) { 10 } else { 1 };
            let (dx, dy) = match key {
                Key::Left => (-step, 0),
                Key::Right => (step, 0),
                Key::Up => (0, -step),
                Key::Down => (0, step),
                _ => return glib_propagate(),
            };
            let mut st = state.borrow_mut();
            let Some(i) = st.selected else { return glib_propagate() };
            st.rects[i].x += dx;
            st.rects[i].y += dy;
            drop(st);
            area_ref.queue_draw();
            gtk4::glib::Propagation::Stop
        });
    }
    area.add_controller(keys);

    wrap.append(&area);
    wrap.append(&hint_row());

    // --- actions ------------------------------------------------------------
    let actions = Box::new(Orientation::Horizontal, 6);
    actions.set_halign(Align::End);

    let tidy = Button::with_label("Tidy up");
    tidy.add_css_class("tz-small");
    {
        let state = state.clone();
        let area_ref = area.clone();
        tidy.connect_clicked(move |_| {
            let mut st = state.borrow_mut();
            tidy_up(&mut st.rects);
            drop(st);
            area_ref.queue_draw();
        });
    }

    let revert = Button::with_label("Revert");
    revert.add_css_class("tz-small");
    {
        let state = state.clone();
        let area_ref = area.clone();
        revert.connect_clicked(move |_| {
            let mut st = state.borrow_mut();
            let original = st.original.clone();
            for (r, (x, y)) in st.rects.iter_mut().zip(original) {
                r.x = x;
                r.y = y;
            }
            drop(st);
            area_ref.queue_draw();
        });
    }

    let apply = Button::with_label("Apply layout");
    apply.add_css_class("tz-action");
    {
        let state = state.clone();
        apply.connect_clicked(move |_| {
            let changed = state.borrow().changed();
            if !changed.is_empty() {
                on_commit(changed);
            }
        });
    }

    actions.append(&tidy);
    actions.append(&revert);
    actions.append(&apply);
    wrap.append(&actions);

    wrap
}

fn glib_propagate() -> gtk4::glib::Propagation {
    gtk4::glib::Propagation::Proceed
}

fn hint_row() -> gtk4::Label {
    let l = gtk4::Label::new(Some(
        "Drag a monitor to move it — edges snap to their neighbours. \
         Arrow keys nudge by 1 px, Shift+arrow by 10. Red means two screens overlap.",
    ));
    l.add_css_class("tz-hint");
    l.set_halign(Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_max_width_chars(72);
    l
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Logical rectangles for every monitor the compositor reported.
pub fn rects_of(mons: &[backend::Monitor]) -> Vec<Rect> {
    mons.iter().filter_map(rect_of).collect()
}

fn rect_of(m: &backend::Monitor) -> Option<Rect> {
    let (rw, rh) = m.res.split_once('x')?;
    let rw: f64 = rw.trim().parse().ok()?;
    let rh: f64 = rh.trim().parse().ok()?;
    let scale: f64 = m.scale.trim().parse().unwrap_or(1.0);
    let scale = if scale > 0.0 { scale } else { 1.0 };
    // Odd transforms are the 90°/270° ones, which swap the logical extents.
    let rotated = matches!(m.transform.trim(), "1" | "3" | "5" | "7");
    let (lw, lh) = if rotated { (rh / scale, rw / scale) } else { (rw / scale, rh / scale) };
    let (x, y) = parse_pos(&m.pos)?;
    Some(Rect {
        name: m.name.clone(),
        detail: format!("{} · {} Hz", m.res, trim_rate(&m.rate)),
        x,
        y,
        w: lw.round().max(1.0) as i32,
        h: lh.round().max(1.0) as i32,
        disabled: m.disabled,
    })
}

/// `"3440x0"`, and the negative forms Hyprland also accepts (`"-1920x0"`,
/// `"0x-1080"`). Splitting naively on `x` mangles a leading minus, so the sign
/// is peeled off first.
fn parse_pos(p: &str) -> Option<(i32, i32)> {
    let t = p.trim();
    let neg = t.starts_with('-');
    let body = t.strip_prefix('-').unwrap_or(t);
    let (a, b) = body.split_once('x')?;
    let x: i32 = a.trim().parse().ok()?;
    let y: i32 = b.trim().parse().ok()?;
    Some((if neg { -x } else { x }, y))
}

/// `165.00` → `165`, `143.97` → `143.97`. Whole rates are the common case and
/// the trailing zeros are noise on a small label.
fn trim_rate(r: &str) -> String {
    match r.trim().parse::<f64>() {
        Ok(v) if (v - v.round()).abs() < 0.005 => format!("{}", v.round() as i64),
        Ok(v) => format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string(),
        Err(_) => r.trim().to_string(),
    }
}

fn bounds(rects: &[Rect]) -> (i32, i32, i32, i32) {
    let minx = rects.iter().map(|r| r.x).min().unwrap_or(0);
    let miny = rects.iter().map(|r| r.y).min().unwrap_or(0);
    let maxx = rects.iter().map(Rect::right).max().unwrap_or(1);
    let maxy = rects.iter().map(Rect::bottom).max().unwrap_or(1);
    (minx, miny, maxx, maxy)
}

/// Fit the whole arrangement into the allocation and centre it.
fn view_for(rects: &[Rect], w: f64, h: f64) -> View {
    if rects.is_empty() || w <= 0.0 || h <= 0.0 {
        return View::default();
    }
    let (minx, miny, maxx, maxy) = bounds(rects);
    let ww = (maxx - minx).max(1) as f64;
    let wh = (maxy - miny).max(1) as f64;
    let scale = ((w - 2.0 * PAD) / ww).min((h - 2.0 * PAD) / wh).max(0.0001);
    View {
        scale,
        ox: (w - ww * scale) / 2.0 - minx as f64 * scale,
        oy: (h - wh * scale) / 2.0 - miny as f64 * scale,
    }
}

/// Pull `(x, y)` onto a neighbouring edge when it comes within `tol`.
///
/// Candidate lines are every other monitor's four edges plus the origin, and
/// each axis is resolved independently — so a monitor can snap flush on x while
/// staying free on y, which is what makes "line these two up" a single gesture.
/// Leading edge is tried before trailing edge, matching nwg-displays.
fn snap(rects: &[Rect], i: usize, x: i32, y: i32, tol: i32) -> (i32, i32) {
    if tol <= 0 {
        return (x, y);
    }
    let (w, h) = (rects[i].w, rects[i].h);

    let mut xs = vec![0];
    let mut ys = vec![0];
    for (j, r) in rects.iter().enumerate() {
        if j == i {
            continue;
        }
        xs.push(r.x);
        xs.push(r.right());
        ys.push(r.y);
        ys.push(r.bottom());
    }

    let nx = nearest(&xs, x, w, tol).unwrap_or(x);
    let ny = nearest(&ys, y, h, tol).unwrap_or(y);
    (nx, ny)
}

/// The snapped coordinate for one axis, or `None` if nothing is close enough.
fn nearest(lines: &[i32], v: i32, extent: i32, tol: i32) -> Option<i32> {
    // Leading edge against each line…
    let lead = lines.iter().filter(|c| (v - **c).abs() <= tol).min_by_key(|c| (v - **c).abs());
    if let Some(c) = lead {
        return Some(*c);
    }
    // …then the trailing edge, which lands the monitor flush on the far side.
    lines
        .iter()
        .filter(|c| (v + extent - **c).abs() <= tol)
        .min_by_key(|c| (v + extent - **c).abs())
        .map(|c| c - extent)
}

/// Pack the monitors left to right in their current horizontal order, vertically
/// centred, with the origin at `(0, 0)`.
///
/// Deliberately opinionated: it removes both overlaps and gaps at once, which is
/// what "my layout has gone wrong, just make it sensible" means in practice.
/// Only reachable from the Tidy up button, never automatic.
fn tidy_up(rects: &mut [Rect]) {
    if rects.is_empty() {
        return;
    }
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by_key(|i| (rects[*i].x, rects[*i].y));

    let tallest = rects.iter().map(|r| r.h).max().unwrap_or(0);
    let mut cursor = 0;
    for i in order {
        rects[i].x = cursor;
        rects[i].y = (tallest - rects[i].h) / 2;
        cursor += rects[i].w;
    }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

/// The theme tokens the canvas draws with, resolved from the live stylesheet so
/// a theme switch repaints correctly instead of baking obsidian's palette in.
struct Palette {
    surface: RGBA,
    text: RGBA,
    muted: RGBA,
    accent: RGBA,
    urgent: RGBA,
}

impl Palette {
    fn of(w: &DrawingArea) -> Palette {
        Palette {
            surface: token(w, "tz_surface", (0.08, 0.10, 0.11)),
            text: token(w, "tz_text", (0.91, 0.92, 0.93)),
            muted: token(w, "tz_muted", (0.55, 0.58, 0.60)),
            accent: token(w, "tz_accent", (0.25, 0.72, 0.69)),
            urgent: token(w, "tz_urgent", (0.88, 0.42, 0.46)),
        }
    }
}

/// Resolve one `@define-color` token by name.
///
/// `lookup_color` is deprecated and has no replacement that can read a
/// user-defined token: the themes in `~/.config/tezca/current/colors.css` are
/// exactly such tokens, and the alternative — hardcoding obsidian's hex values —
/// would leave the canvas the one widget in the app that ignores the theme.
#[allow(deprecated)]
fn token(w: &DrawingArea, name: &str, fallback: (f64, f64, f64)) -> RGBA {
    w.style_context()
        .lookup_color(name)
        .unwrap_or_else(|| RGBA::new(fallback.0 as f32, fallback.1 as f32, fallback.2 as f32, 1.0))
}

fn set_rgba(cr: &gtk4::cairo::Context, c: RGBA, alpha: f64) {
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, alpha);
}

fn draw_rect(
    cr: &gtk4::cairo::Context,
    r: &Rect,
    view: &View,
    p: &Palette,
    selected: bool,
    overlapping: bool,
) {
    let (x, y) = (view.sx(r.x), view.sy(r.y));
    let (w, h) = (r.w as f64 * view.scale, r.h as f64 * view.scale);

    // Body.
    set_rgba(cr, p.surface, if r.disabled { 0.35 } else { 0.92 });
    cr.rectangle(x, y, w, h);
    let _ = cr.fill();

    // Border carries the state: red for an overlap (almost always a mistake),
    // accent for the selection, muted otherwise.
    let (edge, width) = match (overlapping, selected) {
        (true, _) => (p.urgent, 2.0),
        (false, true) => (p.accent, 2.0),
        (false, false) => (p.muted, 1.0),
    };
    set_rgba(cr, edge, if r.disabled { 0.5 } else { 1.0 });
    cr.set_line_width(width);
    cr.rectangle(x + width / 2.0, y + width / 2.0, w - width, h - width);
    let _ = cr.stroke();

    // Labels, centred, and skipped entirely when the rect is too small to hold
    // them — a clipped half-word is worse than nothing.
    if w < 46.0 || h < 26.0 {
        return;
    }
    // Text is never allowed past the monitor's own edges — a detail line
    // overhanging into the neighbouring rect reads as belonging to it.
    let inner = w - 8.0;

    cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(13.0);
    set_rgba(cr, if r.disabled { p.muted } else { p.text }, 1.0);
    centre_text(cr, &r.name, x + w / 2.0, y + h / 2.0 - 3.0, inner);

    if h >= 44.0 {
        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(10.0);
        set_rgba(cr, p.muted, 1.0);
        let detail = if r.disabled { "off".to_string() } else { r.detail.clone() };
        // Falls back to just the resolution, then to nothing, rather than
        // spilling: on a narrow rect "2560x1440" alone is still worth having.
        let short = detail.split(" · ").next().unwrap_or("").to_string();
        for candidate in [detail, short] {
            if centre_text(cr, &candidate, x + w / 2.0, y + h / 2.0 + 13.0, inner) {
                break;
            }
        }
    }
}

/// Draw `text` centred on `(cx, cy)`, or draw nothing and return false if it
/// would be wider than `max_w`.
fn centre_text(cr: &gtk4::cairo::Context, text: &str, cx: f64, cy: f64, max_w: f64) -> bool {
    let Ok(e) = cr.text_extents(text) else { return false };
    if e.width() > max_w {
        return false;
    }
    cr.move_to(cx - e.width() / 2.0 - e.x_bearing(), cy);
    let _ = cr.show_text(text);
    true
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str, x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { name: name.into(), detail: String::new(), x, y, w, h, disabled: false }
    }

    fn mon(res: &str, pos: &str, scale: &str, transform: &str) -> backend::Monitor {
        backend::Monitor {
            name: "DP-1".into(),
            desc: String::new(),
            res: res.into(),
            rate: "165.00".into(),
            pos: pos.into(),
            scale: scale.into(),
            transform: transform.into(),
            vrr: String::new(),
            bitdepth: String::new(),
            mirror: String::new(),
            disabled: false,
            modes: vec![],
        }
    }

    #[test]
    fn logical_size_divides_by_scale_and_rotation_swaps_it() {
        let flat = rect_of(&mon("3440x1440", "0x0", "1", "0")).unwrap();
        assert_eq!((flat.w, flat.h), (3440, 1440));

        // A 3440-wide panel at 1.25 is 2752 logical pixels wide, which is what
        // its neighbour has to be positioned against.
        let scaled = rect_of(&mon("3440x1440", "0x0", "1.25", "0")).unwrap();
        assert_eq!((scaled.w, scaled.h), (2752, 1152));

        // transform 1 and 3 are the 90°/270° rotations.
        let portrait = rect_of(&mon("2560x1440", "0x0", "1", "1")).unwrap();
        assert_eq!((portrait.w, portrait.h), (1440, 2560));
        // …while 4-7 are the flips: 6 is flipped-180, which keeps the extents.
        let flipped = rect_of(&mon("2560x1440", "0x0", "1", "6")).unwrap();
        assert_eq!((flipped.w, flipped.h), (2560, 1440));
        let flipped_90 = rect_of(&mon("2560x1440", "0x0", "1", "5")).unwrap();
        assert_eq!((flipped_90.w, flipped_90.h), (1440, 2560));
    }

    #[test]
    fn positions_round_trip_including_the_negative_forms() {
        assert_eq!(parse_pos("3440x0"), Some((3440, 0)));
        assert_eq!(parse_pos("-1920x0"), Some((-1920, 0)));
        assert_eq!(parse_pos("0x-1080"), Some((0, -1080)));
        assert_eq!(parse_pos("-1920x-1080"), Some((-1920, -1080)));
        assert_eq!(parse_pos("garbage"), None);
    }

    #[test]
    fn a_near_miss_snaps_flush_against_its_neighbour() {
        // DP-1 occupies x 0..3440. Dropping DP-3 at 3436 should land it at 3440,
        // leaving no seam — this is the whole point of the canvas.
        let rects = vec![r("DP-1", 0, 0, 3440, 1440), r("DP-3", 3436, 0, 2560, 1440)];
        assert_eq!(snap(&rects, 1, 3436, 0, 20), (3440, 0));
    }

    #[test]
    fn the_trailing_edge_snaps_too_so_a_monitor_can_land_on_the_far_side() {
        // Placing DP-3 to the LEFT of DP-1 means its right edge meets x=0, so
        // its position must come out at -2560.
        let rects = vec![r("DP-1", 0, 0, 3440, 1440), r("DP-3", -2555, 0, 2560, 1440)];
        assert_eq!(snap(&rects, 1, -2555, 0, 20), (-2560, 0));
    }

    #[test]
    fn each_axis_snaps_on_its_own() {
        // Flush on x, deliberately offset on y: y must be left exactly alone.
        let rects = vec![r("DP-1", 0, 0, 3440, 1440), r("DP-3", 3438, 600, 2560, 1440)];
        assert_eq!(snap(&rects, 1, 3438, 600, 20), (3440, 600));
    }

    #[test]
    fn nothing_within_reach_leaves_the_position_untouched() {
        let rects = vec![r("DP-1", 0, 0, 3440, 1440), r("DP-3", 5000, 900, 2560, 1440)];
        assert_eq!(snap(&rects, 1, 5000, 900, 20), (5000, 900));
    }

    #[test]
    fn overlap_is_detected_but_touching_edges_are_not() {
        let a = r("DP-1", 0, 0, 3440, 1440);
        let flush = r("DP-3", 3440, 0, 2560, 1440);
        let over = r("DP-3", 3400, 0, 2560, 1440);
        assert!(!a.overlaps(&flush), "edge-to-edge monitors do not overlap");
        assert!(a.overlaps(&over));
    }

    #[test]
    fn tidy_up_closes_the_gap_this_machine_actually_had() {
        // DP-2 persisted at 0x3440: 2000 px below DP-1's bottom edge, which is
        // the layout that motivated the canvas.
        let mut rects = vec![
            r("DP-1", 0, 0, 3440, 1440),
            r("DP-2", 0, 3440, 2560, 1440),
            r("DP-3", 3440, 0, 2560, 1440),
        ];
        tidy_up(&mut rects);

        // Packed left to right in x order, no gaps, origin at 0,0.
        assert_eq!((rects[0].x, rects[0].y), (0, 0));
        let mut xs: Vec<i32> = rects.iter().map(|r| r.x).collect();
        xs.sort();
        assert_eq!(xs, vec![0, 3440, 6000]);
        // Nothing overlaps once tidied.
        for (i, a) in rects.iter().enumerate() {
            for (j, b) in rects.iter().enumerate() {
                assert!(i == j || !a.overlaps(b), "{} overlaps {}", a.name, b.name);
            }
        }
    }

    #[test]
    fn the_view_fits_the_whole_arrangement_inside_the_allocation() {
        let rects = vec![r("DP-1", 0, 0, 3440, 1440), r("DP-3", 3440, 0, 2560, 1440)];
        let v = view_for(&rects, 690.0, 230.0);
        let (minx, miny, maxx, maxy) = bounds(&rects);
        assert!(v.sx(minx) >= -0.01 && v.sx(maxx) <= 690.01);
        assert!(v.sy(miny) >= -0.01 && v.sy(maxy) <= 230.01);
        // …and the mapping is invertible, which hit-testing depends on.
        assert_eq!(v.lx(v.sx(3440)), 3440);
    }

    #[test]
    fn rate_labels_drop_noise_but_keep_a_real_fraction() {
        assert_eq!(trim_rate("165.00"), "165");
        assert_eq!(trim_rate("143.97"), "143.97");
        assert_eq!(trim_rate("59.94"), "59.94");
    }
}
