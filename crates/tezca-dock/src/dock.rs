//! The dock surfaces + autohide state machine.
//!
//! Each monitor gets a [`DockSurface`]: an Overlay, bottom-anchored, centered
//! layer-shell window (namespace `tezca-dock`) that is mapped only while revealed
//! — so when hidden it captures no input and windows stay fully usable. Reveal is
//! driven by [`Dock::poll_cursor`], which polls the global pointer against each
//! monitor's bottom edge (a thin always-mapped hotspot layer surface was tried
//! first, but GTK4 won't reliably make one thin or deliver its input — see the
//! notes in `DockSurface::build`). Reveal/hide is a fade+slide eased on the frame
//! clock, with a short re-arm block after hiding so leaving the dock downward
//! doesn't instantly re-reveal it.
//!
//! [`Dock`] is the manager: it owns one surface per monitor and the shared model
//! (config, palette, live item list), fanning updates and signals out to each.

use crate::apps::{self, DockItem};
use crate::config::Config;
use crate::magnifier::Magnifier;
use crate::theme::Palette;
use gtk4::gdk;
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4::{Application, IconTheme, Window};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::RefCell;
use std::process::Command;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Reveal/hide easing duration, seconds.
const ANIM_SECS: f64 = 0.16;
/// After hiding, ignore the reveal strip this long to avoid an instant re-reveal.
const REARM_MS: u64 = 260;
/// Cursor poll interval for reveal detection, ms (~30 Hz).
const POLL_MS: u64 = 33;

/// Activation-relevant metadata, parallel to the visual item list (identical
/// across monitors, so the manager keeps one copy).
struct Meta {
    addresses: Vec<String>,
    launch_id: Option<String>,
}

// ===========================================================================
// Manager
// ===========================================================================

pub struct Dock {
    surfaces: RefCell<Vec<Rc<DockSurface>>>,
    theme: IconTheme,
    cfg: RefCell<Config>,
    palette: RefCell<Palette>,
    meta: RefCell<Vec<Meta>>,
}

impl Dock {
    pub fn build(app: &Application, cfg: Config, palette: Palette) -> Rc<Dock> {
        let display = gdk::Display::default().expect("no display");
        let theme = IconTheme::for_display(&display);

        // Transparent window chrome — the glass is all self-drawn.
        let css = gtk4::CssProvider::new();
        css.load_from_data("window { background: transparent; }");
        gtk4::style_context_add_provider_for_display(
            &display,
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let dock = Rc::new(Dock {
            surfaces: RefCell::new(Vec::new()),
            theme,
            cfg: RefCell::new(cfg),
            palette: RefCell::new(palette),
            meta: RefCell::new(Vec::new()),
        });

        // One surface per monitor. (Hotplug is a follow-up; the target rig is a
        // static dual-head setup.)
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            let Some(obj) = monitors.item(i) else { continue };
            let Ok(monitor) = obj.downcast::<gdk::Monitor>() else { continue };
            let surface = DockSurface::build(app, &monitor, &dock);
            dock.surfaces.borrow_mut().push(surface);
        }

        dock.rebuild();

        // Keep the app alive even while every dock window is hidden (unmapped),
        // and poll the cursor to drive reveal.
        let _hold = app.hold();
        let weak = Rc::downgrade(&dock);
        glib::timeout_add_local(std::time::Duration::from_millis(POLL_MS), move || {
            let _keep = &_hold;
            match weak.upgrade() {
                Some(d) => {
                    d.poll_cursor();
                    ControlFlow::Continue
                }
                None => ControlFlow::Break,
            }
        });
        dock
    }

    /// Reveal poll: read the global cursor position and offer it to each surface.
    fn poll_cursor(&self) {
        let Some((cx, cy)) = crate::hypr::cursor_pos() else { return };
        for s in self.surfaces.borrow().iter() {
            s.consider_reveal(cx, cy);
        }
    }

    /// Rebuild the shared item list from live Hyprland state, fan out to surfaces.
    pub fn rebuild(&self) {
        let cfg = self.cfg.borrow().clone();
        let items = apps::build(&cfg, &self.theme);
        let meta = items
            .iter()
            .map(|it: &DockItem| Meta {
                addresses: it.addresses.clone(),
                launch_id: it.launch_id.clone(),
            })
            .collect();
        *self.meta.borrow_mut() = meta;
        for s in self.surfaces.borrow().iter() {
            s.set_items(items.clone());
        }
    }

    /// Re-read the palette (after `tezca theme` repoints current/).
    pub fn reload_palette(&self) {
        let pal = Palette::load();
        *self.palette.borrow_mut() = pal.clone();
        for s in self.surfaces.borrow().iter() {
            s.set_palette(pal.clone());
        }
    }

    /// SIGUSR1 — pin every dock open (autohide suspended) or release them.
    pub fn toggle_pin(&self) {
        for s in self.surfaces.borrow().iter() {
            s.toggle_pin();
        }
    }

    /// Click on item `i` of any surface: focus/cycle a running app, else launch.
    fn activate(&self, i: usize) {
        let (addresses, launch_id) = {
            let meta = self.meta.borrow();
            let Some(m) = meta.get(i) else { return };
            (m.addresses.clone(), m.launch_id.clone())
        };

        if !addresses.is_empty() {
            let active = crate::hypr::active_address();
            let start = active
                .and_then(|a| addresses.iter().position(|x| *x == a))
                .map(|p| (p + 1) % addresses.len())
                .unwrap_or(0);
            crate::hypr::focus(&addresses[start]);
        } else if let Some(id) = launch_id {
            let _ = Command::new("uwsm").args(["app", "--", &id]).spawn();
        }

        for s in self.surfaces.borrow().iter() {
            s.hide_after_activate();
        }
    }
}

// ===========================================================================
// One monitor's surface pair
// ===========================================================================

struct SurfState {
    shown: bool,
    pinned_open: bool,
    target: f64,
    animating: bool,
    last_frame: i64,
    hide_source: Option<glib::SourceId>,
    rearm_block: bool,
}

pub struct DockSurface {
    dock_win: Window,
    mag: Magnifier,
    cfg: Config,
    /// This surface's monitor geometry (x, y, width, height) in layout coords —
    /// used to test whether the polled cursor is in the bottom reveal strip.
    mon: (i32, i32, i32, i32),
    st: RefCell<SurfState>,
    manager: Weak<Dock>,
    me: RefCell<Weak<DockSurface>>,
}

impl DockSurface {
    fn build(app: &Application, monitor: &gdk::Monitor, manager: &Rc<Dock>) -> Rc<DockSurface> {
        let cfg = manager.cfg.borrow().clone();
        let palette = manager.palette.borrow().clone();

        let mag = Magnifier::new();
        mag.set_config(cfg.clone());
        mag.set_palette(palette);
        mag.set_reveal(0.0);

        // --- dock window (mapped only while revealed) ---
        let dock_win = Window::builder().application(app).child(&mag).build();
        dock_win.init_layer_shell();
        dock_win.set_monitor(Some(monitor));
        dock_win.set_layer(Layer::Overlay);
        dock_win.set_namespace(Some("tezca-dock"));
        dock_win.set_anchor(Edge::Bottom, true);
        dock_win.set_margin(Edge::Bottom, cfg.margin_bottom);
        dock_win.set_exclusive_zone(0);

        // Reveal is driven by polling the cursor against this monitor's bottom
        // edge (see Dock::poll_cursor). We tried an always-mapped thin hotspot
        // layer surface, but GTK4 floors a layer toplevel's free axis at ~200px
        // and won't reliably deliver pointer events to an off-screen-trimmed
        // surface — polling is simpler and robust, with no input dead zone.
        let g = monitor.geometry();
        let mon = (g.x(), g.y(), g.width(), g.height());

        let surface = Rc::new(DockSurface {
            dock_win,
            mag,
            cfg,
            mon,
            st: RefCell::new(SurfState {
                shown: false,
                pinned_open: false,
                target: 0.0,
                animating: false,
                last_frame: 0,
                hide_source: None,
                rearm_block: false,
            }),
            manager: Rc::downgrade(manager),
            me: RefCell::new(Weak::new()),
        });
        *surface.me.borrow_mut() = Rc::downgrade(&surface);
        surface.wire();
        surface
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);

        let we = weak.clone();
        self.mag.connect_pointer_enter(move || {
            if let Some(s) = we.upgrade() {
                s.cancel_hide();
            }
        });
        let wl = weak.clone();
        self.mag.connect_pointer_leave(move || {
            if let Some(s) = wl.upgrade() {
                s.arm_hide();
            }
        });

        let wa = weak.clone();
        self.mag.connect_activate(move |i| {
            if let Some(s) = wa.upgrade() {
                if let Some(m) = s.manager.upgrade() {
                    m.activate(i);
                }
            }
        });
    }

    fn set_items(&self, items: Vec<DockItem>) {
        self.mag.set_items(items);
    }
    fn set_palette(&self, p: Palette) {
        self.mag.set_palette(p);
    }

    fn weak(&self) -> Weak<DockSurface> {
        self.me.borrow().clone()
    }

    // --- reveal / hide ----------------------------------------------------

    /// Called from the cursor poll: reveal if the pointer is in this monitor's
    /// bottom reveal strip. (Leaving/hiding is handled by the magnifier's own
    /// pointer controller once the dock is up.)
    fn consider_reveal(&self, cx: i32, cy: i32) {
        let (mx, my, mw, mh) = self.mon;
        let in_x = cx >= mx && cx < mx + mw;
        let in_strip = cy >= my + mh - self.cfg.hotspot_height && cy < my + mh;
        if !(in_x && in_strip) {
            return;
        }
        let st = self.st.borrow();
        if st.shown || st.rearm_block {
            return;
        }
        drop(st);
        self.show();
    }

    fn show(&self) {
        self.cancel_hide();
        {
            let mut st = self.st.borrow_mut();
            if st.shown {
                return;
            }
            st.shown = true;
        }
        self.dock_win.set_visible(true);
        self.animate_to(1.0);
    }

    fn arm_hide(&self) {
        if self.st.borrow().pinned_open {
            return;
        }
        self.cancel_hide();
        let weak = self.weak();
        let id = glib::timeout_add_local_once(
            Duration::from_millis(self.cfg.hide_delay_ms),
            move || {
                if let Some(s) = weak.upgrade() {
                    s.st.borrow_mut().hide_source = None;
                    s.hide_now();
                }
            },
        );
        self.st.borrow_mut().hide_source = Some(id);
    }

    fn cancel_hide(&self) {
        if let Some(id) = self.st.borrow_mut().hide_source.take() {
            id.remove();
        }
    }

    fn hide_now(&self) {
        {
            let mut st = self.st.borrow_mut();
            if st.pinned_open || !st.shown {
                return;
            }
            st.shown = false;
        }
        self.animate_to(0.0);
    }

    fn hide_after_activate(&self) {
        if !self.st.borrow().pinned_open {
            self.hide_now();
        }
    }

    fn toggle_pin(&self) {
        let now = {
            let mut st = self.st.borrow_mut();
            st.pinned_open = !st.pinned_open;
            st.pinned_open
        };
        if now {
            self.show();
        } else {
            self.arm_hide();
        }
    }

    // --- animation --------------------------------------------------------

    fn animate_to(&self, target: f64) {
        {
            let mut st = self.st.borrow_mut();
            st.target = target;
            if st.animating {
                return;
            }
            st.animating = true;
            st.last_frame = 0;
        }
        let weak = self.weak();
        self.mag.add_tick_callback(move |_w, clock| {
            let Some(s) = weak.upgrade() else { return ControlFlow::Break };
            s.tick(clock)
        });
    }

    fn tick(&self, clock: &gdk::FrameClock) -> ControlFlow {
        let now = clock.frame_time();
        let (target, dt) = {
            let mut st = self.st.borrow_mut();
            let dt = if st.last_frame == 0 {
                0.0
            } else {
                (now - st.last_frame) as f64 / 1_000_000.0
            };
            st.last_frame = now;
            (st.target, dt)
        };

        let cur = self.mag.reveal();
        let step = if dt > 0.0 { dt / ANIM_SECS } else { 0.0 };
        let next = if target > cur {
            (cur + step).min(target)
        } else {
            (cur - step).max(target)
        };
        self.mag.set_reveal(next);

        if (next - target).abs() < 0.001 {
            self.mag.set_reveal(target);
            self.st.borrow_mut().animating = false;
            if target <= 0.0 {
                self.dock_win.set_visible(false);
                self.block_rearm();
            }
            return ControlFlow::Break;
        }
        ControlFlow::Continue
    }

    fn block_rearm(&self) {
        self.st.borrow_mut().rearm_block = true;
        let weak = self.weak();
        glib::timeout_add_local_once(Duration::from_millis(REARM_MS), move || {
            if let Some(s) = weak.upgrade() {
                s.st.borrow_mut().rearm_block = false;
            }
        });
    }
}
