//! The magnifier — a custom `gtk::Widget` that self-draws the whole dock.
//!
//! One `snapshot()` paints the obsidian-glass pill and every icon at a per-frame
//! scale computed from the pointer's distance (classic macOS cosine falloff),
//! laid out so the icon under the cursor stays put while its neighbours part.
//! Drawing everything ourselves (no per-icon widgets, no GTK CSS) gives full
//! geometric control and keeps the magnification buttery on 165 Hz. Colors come
//! from the parsed theme-engine palette; running/hover chrome is drawn inline.

use crate::apps::DockItem;
use crate::config::Config;
use crate::theme::{self, Palette};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use gtk4::{graphene, gsk};
use std::f64::consts::FRAC_PI_2;

/// Extra room before a group-divider (pinned | running-only).
const DIVIDER_GAP: f64 = 14.0;
/// Vertical distance the dock rises over on reveal.
const SLIDE: f64 = 22.0;
/// Reserved band above the icons for the hover label.
const LABEL_ZONE: f64 = 24.0;

mod imp {
    use super::*;
    use gtk4::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct Magnifier {
        pub items: RefCell<Vec<DockItem>>,
        pub palette: RefCell<Palette>,
        pub config: RefCell<Config>,
        pub pointer_x: Cell<Option<f64>>,
        pub reveal: Cell<f64>,
        pub on_activate: RefCell<Option<Box<dyn Fn(usize)>>>,
        pub on_enter: RefCell<Option<Box<dyn Fn()>>>,
        pub on_leave: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Magnifier {
        const NAME: &'static str = "TezcaMagnifier";
        type Type = super::Magnifier;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for Magnifier {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Pointer tracking → magnification + reveal/hide triggers.
            let motion = gtk4::EventControllerMotion::new();
            motion.connect_enter(glib::clone!(
                #[weak] obj,
                move |_, x, _| {
                    obj.imp().pointer_x.set(Some(x));
                    if let Some(cb) = obj.imp().on_enter.borrow().as_ref() { cb(); }
                    obj.queue_draw();
                }
            ));
            motion.connect_motion(glib::clone!(
                #[weak] obj,
                move |_, x, _| {
                    obj.imp().pointer_x.set(Some(x));
                    obj.queue_draw();
                }
            ));
            motion.connect_leave(glib::clone!(
                #[weak] obj,
                move |_| {
                    obj.imp().pointer_x.set(None);
                    if let Some(cb) = obj.imp().on_leave.borrow().as_ref() { cb(); }
                    obj.queue_draw();
                }
            ));
            obj.add_controller(motion);

            // Click → activate the item under the cursor (base-layout hit-test).
            let click = gtk4::GestureClick::new();
            click.set_button(gtk4::gdk::BUTTON_PRIMARY);
            click.connect_released(glib::clone!(
                #[weak] obj,
                move |_, _, x, _| {
                    if let Some(i) = obj.hit_test(x) {
                        if let Some(cb) = obj.imp().on_activate.borrow().as_ref() { cb(i); }
                    }
                }
            ));
            obj.add_controller(click);
        }
    }

    impl WidgetImpl for Magnifier {
        fn measure(&self, orientation: gtk4::Orientation, _for: i32) -> (i32, i32, i32, i32) {
            let obj = self.obj();
            let size = if orientation == gtk4::Orientation::Horizontal {
                obj.natural_width()
            } else {
                obj.natural_height()
            };
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct Magnifier(ObjectSubclass<imp::Magnifier>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Default for Magnifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-frame geometry of one icon.
struct Geom {
    left: f64,
    size: f64,
    scale: f64,
}

impl Magnifier {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // --- wiring from the dock controller ---------------------------------

    pub fn set_items(&self, items: Vec<DockItem>) {
        self.imp().items.replace(items);
        self.queue_resize();
        self.queue_draw();
    }

    pub fn set_palette(&self, p: Palette) {
        self.imp().palette.replace(p);
        self.queue_draw();
    }

    pub fn set_config(&self, c: Config) {
        self.imp().config.replace(c);
        self.queue_resize();
        self.queue_draw();
    }

    pub fn set_reveal(&self, r: f64) {
        self.imp().reveal.set(r.clamp(0.0, 1.0));
        self.queue_draw();
    }

    pub fn reveal(&self) -> f64 {
        self.imp().reveal.get()
    }

    pub fn connect_activate<F: Fn(usize) + 'static>(&self, f: F) {
        self.imp().on_activate.replace(Some(Box::new(f)));
    }
    pub fn connect_pointer_enter<F: Fn() + 'static>(&self, f: F) {
        self.imp().on_enter.replace(Some(Box::new(f)));
    }
    pub fn connect_pointer_leave<F: Fn() + 'static>(&self, f: F) {
        self.imp().on_leave.replace(Some(Box::new(f)));
    }

    // --- intrinsic size ---------------------------------------------------

    fn natural_width(&self) -> i32 {
        let cfg = self.imp().config.borrow();
        let items = self.imp().items.borrow();
        let n = items.len().max(1) as f64;
        let dividers = items.iter().filter(|i| i.divider_before).count() as f64;
        let base_row = n * cfg.icon_size + (n - 1.0) * cfg.gap + dividers * DIVIDER_GAP;
        let pill = base_row + 2.0 * cfg.pad_x;
        // Headroom each side so magnified icons never clip against the window edge.
        let headroom = cfg.icon_size * cfg.max_scale;
        (pill + 2.0 * headroom).ceil() as i32
    }

    fn natural_height(&self) -> i32 {
        let cfg = self.imp().config.borrow();
        let max_icon = cfg.icon_size * cfg.max_scale;
        (SLIDE + LABEL_ZONE + max_icon + cfg.pad_y).ceil() as i32
    }

    // --- layout -----------------------------------------------------------

    /// Base (unmagnified) icon centers for the current width, plus total row width.
    fn base_centers(&self, w: f64) -> (Vec<f64>, f64) {
        let cfg = self.imp().config.borrow();
        let items = self.imp().items.borrow();
        let n = items.len();
        let dividers = items.iter().filter(|i| i.divider_before).count() as f64;
        let total = n as f64 * cfg.icon_size
            + (n.saturating_sub(1)) as f64 * cfg.gap
            + dividers * DIVIDER_GAP;
        let left = (w - total) / 2.0;
        let mut x = left;
        let mut centers = Vec::with_capacity(n);
        for (i, it) in items.iter().enumerate() {
            if it.divider_before {
                x += DIVIDER_GAP;
            }
            centers.push(x + cfg.icon_size / 2.0);
            x += cfg.icon_size;
            if i + 1 < n {
                x += cfg.gap;
            }
        }
        (centers, total)
    }

    /// Full per-icon geometry given the (optional) pointer x and widget width.
    fn compute(&self, px: Option<f64>, w: f64) -> Vec<Geom> {
        let cfg = self.imp().config.borrow();
        let items = self.imp().items.borrow();
        let n = items.len();
        if n == 0 {
            return Vec::new();
        }
        let (centers, total_base) = self.base_centers(w);

        // Scale from pointer distance: cosine bump within `influence`.
        let scales: Vec<f64> = centers
            .iter()
            .map(|&c| match px {
                Some(p) => {
                    let d = (p - c).abs();
                    if d >= cfg.influence {
                        1.0
                    } else {
                        let f = (FRAC_PI_2 * (d / cfg.influence)).cos();
                        1.0 + (cfg.max_scale - 1.0) * f * f
                    }
                }
                None => 1.0,
            })
            .collect();

        // Sequential layout by magnified width (constant gaps), starting at 0.
        let mut seq_left = vec![0.0; n];
        let mut x = 0.0;
        for i in 0..n {
            if items[i].divider_before {
                x += DIVIDER_GAP;
            }
            seq_left[i] = x;
            x += cfg.icon_size * scales[i];
            if i + 1 < n {
                x += cfg.gap;
            }
        }

        // Anchor: keep the icon under the cursor under the cursor.
        let base_left = (w - total_base) / 2.0;
        let shift = match px {
            None => base_left,
            Some(p) => {
                let a = nearest_index(&centers, p);
                let base_left_a = centers[a] - cfg.icon_size / 2.0;
                let frac = ((p - base_left_a) / cfg.icon_size).clamp(0.0, 1.0);
                let seq_anchor = seq_left[a] + frac * cfg.icon_size * scales[a];
                p - seq_anchor
            }
        };

        (0..n)
            .map(|i| Geom {
                left: seq_left[i] + shift,
                size: cfg.icon_size * scales[i],
                scale: scales[i],
            })
            .collect()
    }

    /// Hit-test against the *base* layout (stable while icons magnify).
    fn hit_test(&self, x: f64) -> Option<usize> {
        let w = self.width() as f64;
        let cfg = self.imp().config.borrow();
        let (centers, _) = self.base_centers(w);
        let half = cfg.icon_size / 2.0 + cfg.gap / 2.0;
        centers
            .iter()
            .position(|&c| (x - c).abs() <= half)
    }

    // --- drawing ----------------------------------------------------------

    fn draw(&self, snapshot: &gtk4::Snapshot) {
        let w = self.width() as f64;
        let h = self.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let cfg = self.imp().config.borrow().clone();
        let pal = self.imp().palette.borrow().clone();
        let reveal = self.imp().reveal.get();
        let px = self.imp().pointer_x.get();
        let items_len = self.imp().items.borrow().len();
        if items_len == 0 {
            return;
        }

        // Clip to the widget, fade + slide up on reveal.
        let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
        snapshot.push_clip(&bounds);
        snapshot.push_opacity(reveal);
        snapshot.translate(&graphene::Point::new(0.0, ((1.0 - reveal) * SLIDE) as f32));

        // --- glass pill (static, centered on the base row) ---
        let (_, total_base) = self.base_centers(w);
        let pill_w = total_base + 2.0 * cfg.pad_x;
        let pill_h = cfg.icon_size + 2.0 * cfg.pad_y;
        let pill_x = (w - pill_w) / 2.0;
        let pill_y = h - pill_h;
        let radius = (pill_h / 2.0).min(20.0) as f32;
        let pill_rect = graphene::Rect::new(pill_x as f32, pill_y as f32, pill_w as f32, pill_h as f32);
        let rr = gsk::RoundedRect::from_rect(pill_rect.clone(), radius);
        snapshot.push_rounded_clip(&rr);
        snapshot.append_color(&theme::with_alpha(pal.base, 0.72), &pill_rect);
        snapshot.pop();
        let border_col = theme::with_alpha(pal.accent, 0.16);
        snapshot.append_border(&rr, &[1.0; 4], &[border_col; 4]);

        // --- icons + chrome ---
        let geoms = self.compute(px, w);
        let bottom = h - cfg.pad_y;
        let items = self.imp().items.borrow();

        // Group divider(s).
        for (i, it) in items.iter().enumerate() {
            if it.divider_before {
                let dx = (geoms[i].left - DIVIDER_GAP / 2.0) as f32;
                let dr = graphene::Rect::new(dx, (pill_y + 6.0) as f32, 1.0, (pill_h - 12.0) as f32);
                snapshot.append_color(&theme::with_alpha(pal.muted, 0.35), &dr);
            }
        }

        for (i, g) in geoms.iter().enumerate() {
            let top = bottom - g.size;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(g.left as f32, top as f32));
            items[i].icon.snapshot(snapshot, g.size, g.size);
            snapshot.restore();

            // Running indicator dot below the icon.
            if items[i].running {
                let r = 2.4_f64;
                let cx = g.left + g.size / 2.0;
                let cy = h - cfg.pad_y / 2.0;
                let dot = graphene::Rect::new(
                    (cx - r) as f32,
                    (cy - r) as f32,
                    (2.0 * r) as f32,
                    (2.0 * r) as f32,
                );
                let dr = gsk::RoundedRect::from_rect(dot.clone(), r as f32);
                snapshot.push_rounded_clip(&dr);
                snapshot.append_color(&pal.accent, &dot);
                snapshot.pop();
            }
        }

        // --- hover label above the focused icon ---
        if let Some(p) = px {
            let a = nearest_index_geom(&geoms);
            if geoms[a].scale > 1.15 {
                let _ = p;
                let g = &geoms[a];
                let top = bottom - g.size;
                self.draw_label(snapshot, &items[a].label, g.left + g.size / 2.0, top - 8.0, &pal, w);
            }
        }

        snapshot.pop(); // opacity
        snapshot.pop(); // clip
    }

    fn draw_label(&self, snapshot: &gtk4::Snapshot, text: &str, cx: f64, baseline_y: f64, pal: &Palette, w: f64) {
        let layout = self.create_pango_layout(Some(text));
        let desc = gtk4::pango::FontDescription::from_string("Inter 10");
        layout.set_font_description(Some(&desc));
        let (tw, th) = layout.pixel_size();
        let (tw, th) = (tw as f64, th as f64);
        let pad = 7.0;
        let bw = tw + 2.0 * pad;
        let bh = th + 2.0 * pad * 0.6;
        // Keep the label on-screen near the edges.
        let bx = (cx - bw / 2.0).clamp(2.0, (w - bw - 2.0).max(2.0));
        let by = baseline_y - bh;
        let rect = graphene::Rect::new(bx as f32, by as f32, bw as f32, bh as f32);
        let rr = gsk::RoundedRect::from_rect(rect.clone(), 8.0);
        snapshot.push_rounded_clip(&rr);
        snapshot.append_color(&theme::with_alpha(pal.base, 0.96), &rect);
        snapshot.pop();
        snapshot.append_border(&rr, &[1.0; 4], &[theme::with_alpha(pal.accent, 0.18); 4]);

        snapshot.save();
        snapshot.translate(&graphene::Point::new((bx + pad) as f32, (by + bh * 0.5 - th / 2.0) as f32));
        snapshot.append_layout(&layout, &pal.text);
        snapshot.restore();
    }
}

fn nearest_index(centers: &[f64], p: f64) -> usize {
    let mut best = 0;
    let mut best_d = f64::INFINITY;
    for (i, &c) in centers.iter().enumerate() {
        let d = (p - c).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

fn nearest_index_geom(geoms: &[Geom]) -> usize {
    let mut best = 0;
    let mut best_s = -1.0;
    for (i, g) in geoms.iter().enumerate() {
        if g.scale > best_s {
            best_s = g.scale;
            best = i;
        }
    }
    best
}
