//! The dock surfaces + autohide state machine.
//!
//! Each monitor gets a [`DockSurface`]: an Overlay, bottom-anchored, centered
//! layer-shell window (namespace `tezca-dock`) that is mapped only while revealed
//! — so when hidden it captures no input and windows stay fully usable. Both
//! halves of autohide are driven by [`Dock::poll_cursor`], which polls the global
//! pointer against each monitor (a thin always-mapped hotspot layer surface was
//! tried first for the reveal, but GTK4 won't reliably make one thin or deliver
//! its input — see the notes in `DockSurface::build`).
//!
//! The poll owns *both* directions on purpose. Reveal fires from a strip that
//! spans the whole bottom edge, while the dock is a few hundred pixels wide and
//! floats `margin_bottom` above that edge — the two regions do not overlap, so a
//! pointer that grazes the bottom corner of a screen reveals a dock it will never
//! touch. Hanging the hide on the widget's own pointer-leave (as this once did)
//! means that dock never gets a leave, and stays up until something else moves
//! it. The widget's crossing signals are still wired, as a faster local echo and
//! as the fallback for a poll that can't reach Hyprland.
//!
//! Reveal/hide is a fade+slide eased on the frame clock, with a short re-arm
//! block after hiding so leaving the dock downward doesn't instantly re-reveal
//! it, and a watchdog so a stalled frame clock can't wedge the state machine.
//!
//! [`Dock`] is the manager: it owns one surface per monitor and the shared model
//! (config, palette, live item list), fanning updates and signals out to each,
//! and re-anchors the surfaces when the output list moves under them.

use crate::apps::{self, DockItem};
use crate::config::Config;
use crate::magnifier::Magnifier;
use crate::theme::Palette;
use gtk4::gdk;
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4::{Application, IconTheme, Window};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Reveal/hide easing duration, seconds.
const ANIM_SECS: f64 = 0.16;
/// Hard stop for one reveal/hide, ms. The easing rides the widget's frame clock,
/// which is the compositor's to stop feeding — an output in direct scanout for a
/// fullscreen window sends no frame callbacks to anything else, and a tick that
/// never runs is a tick that never clears `animating`, which would leave every
/// later `animate_to` a no-op and the dock frozen wherever it stood.
const ANIM_WATCHDOG_MS: u64 = 500;
/// After hiding, ignore the reveal strip this long to avoid an instant re-reveal.
const REARM_MS: u64 = 260;
/// Cursor poll interval for reveal detection, ms (~30 Hz).
const POLL_MS: u64 = 33;
/// Slack around the dock window that still counts as being on it, px. The poll
/// is a coarser instrument than the widget's own crossing events; this keeps the
/// two from disagreeing along the edge and flickering the hide timer.
const KEEP_SLACK: i32 = 12;
/// How long the output list must hold still before the surfaces are re-anchored,
/// ms. Waking two screens emits a burst, and only the last pass has to be right.
const MONITOR_SETTLE_MS: u64 = 400;

/// Live outputs, keyed by connector name — the only identity that survives an
/// output being destroyed and recreated, since the `GdkMonitor` is a fresh
/// object each time.
fn live_monitors(display: &gdk::Display) -> Vec<(String, gdk::Monitor)> {
    let list = display.monitors();
    let mut out = Vec::new();
    for i in 0..list.n_items() {
        let Some(obj) = list.item(i) else { continue };
        let Ok(monitor) = obj.downcast::<gdk::Monitor>() else { continue };
        if !monitor.is_valid() || monitor.geometry().width() == 0 {
            continue;
        }
        let name = monitor.connector().map(|s| s.to_string()).unwrap_or_default();
        out.push((name, monitor));
    }
    out
}

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
    /// One surface per output that has been seen. Mutable because the output
    /// list is: see [`Dock::sync_monitors`].
    surfaces: RefCell<Vec<Rc<DockSurface>>>,
    /// Kept so a monitor that appears later can be given a surface.
    app: Application,
    theme: IconTheme,
    cfg: RefCell<Config>,
    palette: RefCell<Palette>,
    meta: RefCell<Vec<Meta>>,
    /// Debounce for [`Dock::schedule_sync`]: the pending pass, and whether any
    /// event in the burst asked for a forced re-anchor.
    sync_pending: RefCell<Option<glib::SourceId>>,
    sync_force: Cell<bool>,
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
            app: app.clone(),
            theme,
            cfg: RefCell::new(cfg),
            palette: RefCell::new(palette),
            meta: RefCell::new(Vec::new()),
            sync_pending: RefCell::new(None),
            sync_force: Cell::new(false),
        });

        // One surface per live output, and keep that true afterwards.
        for (name, monitor) in live_monitors(&display) {
            let surface = DockSurface::build(app, name, &monitor, &dock);
            dock.surfaces.borrow_mut().push(surface);
        }
        dock.watch_monitors();

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

    /// Autohide poll: read the global cursor position and offer it to each
    /// surface, which reveals or arms its hide from it.
    fn poll_cursor(&self) {
        let Some((cx, cy)) = crate::hypr::cursor_pos() else { return };
        for s in self.surfaces.borrow().iter() {
            s.consider(cx, cy);
        }
    }

    // --- output topology --------------------------------------------------

    /// Watch the display's output list and re-anchor the docks when it moves.
    ///
    /// A display that sleeps deeply enough drops its link and the compositor
    /// destroys that output, but a layer surface anchored to it is not destroyed
    /// with it — Hyprland re-homes the orphan onto a surviving monitor and never
    /// moves it back. For an autohiding dock that is worse than a misplaced
    /// window: the orphan still reveals from *its* monitor's bottom edge, so it
    /// maps onto a screen the pointer isn't on, where nothing can ever hover it.
    fn watch_monitors(self: &Rc<Self>) {
        let Some(display) = gdk::Display::default() else { return };
        let weak = Rc::downgrade(self);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(d) = weak.upgrade() {
                d.schedule_sync(false);
            }
        });
    }

    /// Queue a [`Dock::sync_monitors`] for once the output list stops moving.
    /// A `force` asked for by any event in the burst carries through.
    pub fn schedule_sync(self: &Rc<Self>, force: bool) {
        if force {
            self.sync_force.set(true);
        }
        if let Some(id) = self.sync_pending.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id =
            glib::timeout_add_local_once(Duration::from_millis(MONITOR_SETTLE_MS), move || {
                let Some(d) = weak.upgrade() else { return };
                *d.sync_pending.borrow_mut() = None;
                d.sync_monitors(d.sync_force.replace(false));
            });
        *self.sync_pending.borrow_mut() = Some(id);
    }

    /// Reconcile the surfaces against the live outputs. Idempotent.
    ///
    /// A surface whose output has gone is parked (hidden, state reset) and kept,
    /// never destroyed: tearing down a layer-shell window after its output is
    /// already gone takes GDK down with it, and the monitor that just went to
    /// sleep is overwhelmingly likely to come back.
    ///
    /// `force` re-anchors even when the monitor object looks unchanged, for the
    /// compositor that shuffles layer surfaces without dropping the output —
    /// which the display-side signal never reports.
    fn sync_monitors(self: &Rc<Self>, force: bool) {
        let Some(display) = gdk::Display::default() else { return };
        let live = live_monitors(&display);
        // Every output down at once is mid-transition, not a topology.
        if live.is_empty() {
            return;
        }

        for s in self.surfaces.borrow().iter() {
            if !live.iter().any(|(name, _)| *name == s.output) {
                s.park();
            }
        }

        let mut fresh: Vec<Rc<DockSurface>> = Vec::new();
        for (name, monitor) in &live {
            let existing = self.surfaces.borrow().iter().find(|s| s.output == *name).cloned();
            let Some(s) = existing else {
                fresh.push(DockSurface::build(&self.app, name.clone(), monitor, self));
                continue;
            };
            let placed = s.dock_win.monitor().is_some_and(|cur| cur.is_valid() && cur == *monitor);
            if force || !placed || s.st.borrow().detached {
                // The dock is unmapped whenever it is hidden, so re-anchoring it
                // costs nothing visible: the new output takes effect at the next
                // reveal. A pinned-open dock is mapped and does move now.
                s.dock_win.set_monitor(Some(monitor));
            }
            s.reattach(monitor);
        }

        if fresh.is_empty() {
            return;
        }
        let cfg = self.cfg.borrow().clone();
        let items = apps::build(&cfg, &self.theme);
        for s in &fresh {
            s.set_items(items.clone());
        }
        self.surfaces.borrow_mut().extend(fresh);
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

    /// Files dropped on item `i`: open them with that app.
    ///
    /// `uwsm app -- <desktop-id> <paths…>` is the same launch path a click
    /// takes, with the files appended — so the app starts in the user's session
    /// scope exactly as it would otherwise, and a `.desktop` entry's own
    /// `%f`/`%U` handling does the rest. Items with no desktop id (a running
    /// app that was never pinned and has no entry) cannot be launch targets,
    /// so a drop on one is ignored rather than guessed at.
    fn drop_files(&self, i: usize, paths: Vec<String>) {
        let launch_id = {
            let meta = self.meta.borrow();
            let Some(m) = meta.get(i) else { return };
            let Some(id) = m.launch_id.clone() else { return };
            id
        };
        let mut args: Vec<String> = vec!["app".into(), "--".into(), launch_id];
        args.extend(paths);
        let _ = Command::new("uwsm").args(&args).spawn();
        for s in self.surfaces.borrow().iter() {
            s.hide_after_activate();
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
    /// Cleared when the watchdog or the tick lands an animation; bumping it
    /// orphans a tick callback whose frame clock stopped, so the next animation
    /// starts a live one instead of trusting a corpse.
    tick_gen: u64,
    watchdog: Option<glib::SourceId>,
    /// This surface's output is gone (asleep, unplugged). It stays parked until
    /// [`DockSurface::reattach`].
    detached: bool,
}

pub struct DockSurface {
    dock_win: Window,
    mag: Magnifier,
    cfg: Config,
    /// Connector name (`DP-1`) — the surface's identity across an output being
    /// destroyed and recreated.
    output: String,
    /// This surface's output. Read live for its geometry rather than cached: a
    /// mode change moves the bottom edge without touching the output list.
    monitor: RefCell<gdk::Monitor>,
    st: RefCell<SurfState>,
    manager: Weak<Dock>,
    me: RefCell<Weak<DockSurface>>,
}

impl DockSurface {
    fn build(
        app: &Application,
        output: String,
        monitor: &gdk::Monitor,
        manager: &Rc<Dock>,
    ) -> Rc<DockSurface> {
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
        let surface = Rc::new(DockSurface {
            dock_win,
            mag,
            cfg,
            output,
            monitor: RefCell::new(monitor.clone()),
            st: RefCell::new(SurfState {
                shown: false,
                pinned_open: false,
                target: 0.0,
                animating: false,
                last_frame: 0,
                hide_source: None,
                rearm_block: false,
                tick_gen: 0,
                watchdog: None,
                detached: false,
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

        let wd = weak.clone();
        self.mag.connect_drop(move |i, paths| {
            if let Some(s) = wd.upgrade() {
                if let Some(m) = s.manager.upgrade() {
                    m.drop_files(i, paths);
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

    /// This monitor's geometry in layout coords, or `None` while its output is
    /// gone — a parked surface must not reveal on someone else's screen.
    fn monitor_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let m = self.monitor.borrow();
        if !m.is_valid() {
            return None;
        }
        let g = m.geometry();
        (g.width() > 0 && g.height() > 0).then(|| (g.x(), g.y(), g.width(), g.height()))
    }

    /// Called from the cursor poll — the whole autohide decision for this
    /// surface. Reveal when the pointer enters the bottom strip; hide when it is
    /// neither on the dock nor on the strip that summoned it.
    ///
    /// The hide half cannot be left to the magnifier's pointer controller alone:
    /// the strip spans the full width of the screen and sits *below* the dock's
    /// bottom margin, so most reveals never put the pointer inside the window at
    /// all, and a widget that was never entered never fires a leave.
    fn consider(&self, cx: i32, cy: i32) {
        let (shown, pinned, blocked, detached) = {
            let st = self.st.borrow();
            (st.shown, st.pinned_open, st.rearm_block, st.detached)
        };
        if detached || pinned {
            return;
        }
        let Some(mon) = self.monitor_rect() else { return };
        let (mx, my, mw, mh) = mon;
        let in_strip =
            cx >= mx && cx < mx + mw && cy >= my + mh - self.cfg.hotspot_height && cy < my + mh;

        if !shown {
            if in_strip && !blocked {
                self.show();
            }
        } else if in_strip || self.over_dock(cx, cy, mon) {
            self.cancel_hide();
        } else {
            self.arm_hide();
        }
    }

    /// Is the pointer on this monitor's dock window (plus a little slack)?
    ///
    /// Derived from the monitor rather than asked of GTK: the window is bottom-
    /// anchored and centered by the compositor, and a layer surface only ever
    /// knows its own size, never where the layout put it.
    fn over_dock(&self, cx: i32, cy: i32, mon: (i32, i32, i32, i32)) -> bool {
        let (mx, my, mw, mh) = mon;
        let (w, h) = (self.dock_win.width(), self.dock_win.height());
        if w <= 0 || h <= 0 {
            return false;
        }
        let x0 = mx + (mw - w) / 2 - KEEP_SLACK;
        let y0 = my + mh - self.cfg.margin_bottom - h - KEEP_SLACK;
        // Down to the screen edge, not to the window's own bottom: the margin
        // beneath the dock is a gap the pointer crosses on its way in, and
        // hiding the dock out from under an approach is the one thing worse
        // than not hiding it.
        cx >= x0 && cx < x0 + w + 2 * KEEP_SLACK && cy >= y0 && cy < my + mh
    }

    /// The output went away: hide, drop any pending work, and stay down until
    /// [`DockSurface::reattach`]. Left mapped, the compositor re-homes the
    /// orphan onto a surviving screen and it becomes a dock nothing can dismiss.
    fn park(&self) {
        {
            let mut st = self.st.borrow_mut();
            if st.detached {
                return;
            }
            st.detached = true;
            st.shown = false;
            st.pinned_open = false;
            st.animating = false;
            st.tick_gen = st.tick_gen.wrapping_add(1);
        }
        self.cancel_hide();
        self.cancel_watchdog();
        self.mag.set_reveal(0.0);
        self.dock_win.set_visible(false);
    }

    /// The output is back (or never left): take the fresh monitor object, since
    /// the one we were holding is a corpse after a recreate.
    fn reattach(&self, monitor: &gdk::Monitor) {
        *self.monitor.borrow_mut() = monitor.clone();
        self.st.borrow_mut().detached = false;
    }

    fn show(&self) {
        self.cancel_hide();
        {
            let mut st = self.st.borrow_mut();
            if st.shown || st.detached {
                return;
            }
            st.shown = true;
        }
        self.dock_win.set_visible(true);
        self.animate_to(1.0);
    }

    /// Start the hide countdown, or leave a running one alone. Idempotent
    /// because the poll calls it at 30 Hz for as long as the pointer is away —
    /// re-arming each time would push the deadline forever and never hide.
    fn arm_hide(&self) {
        {
            let st = self.st.borrow();
            if st.pinned_open || st.hide_source.is_some() {
                return;
            }
        }
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
            if st.detached {
                return;
            }
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
        let start = {
            let mut st = self.st.borrow_mut();
            st.target = target;
            // An animation already in flight just changes course; only an idle
            // surface needs a new tick callback.
            let start = !st.animating;
            if start {
                st.animating = true;
                st.last_frame = 0;
                st.tick_gen = st.tick_gen.wrapping_add(1);
            }
            start.then_some(st.tick_gen)
        };

        // Arm (or re-arm) the watchdog for whatever the target now is. The frame
        // clock is the compositor's to stop; the state machine is not.
        self.cancel_watchdog();
        let ww = self.weak();
        let id = glib::timeout_add_local_once(Duration::from_millis(ANIM_WATCHDOG_MS), move || {
            if let Some(s) = ww.upgrade() {
                s.st.borrow_mut().watchdog = None;
                s.settle();
            }
        });
        self.st.borrow_mut().watchdog = Some(id);

        let Some(gen) = start else { return };
        let weak = self.weak();
        self.mag.add_tick_callback(move |_w, clock| {
            let Some(s) = weak.upgrade() else { return ControlFlow::Break };
            s.tick(clock, gen)
        });
    }

    fn tick(&self, clock: &gdk::FrameClock, gen: u64) -> ControlFlow {
        let now = clock.frame_time();
        let (target, dt) = {
            let mut st = self.st.borrow_mut();
            // A watchdog landed this animation and someone else owns the surface
            // now: this callback is the stalled one, waking up too late.
            if st.tick_gen != gen {
                return ControlFlow::Break;
            }
            let dt =
                if st.last_frame == 0 { 0.0 } else { (now - st.last_frame) as f64 / 1_000_000.0 };
            st.last_frame = now;
            (st.target, dt)
        };

        let cur = self.mag.reveal();
        let step = if dt > 0.0 { dt / ANIM_SECS } else { 0.0 };
        let next = if target > cur { (cur + step).min(target) } else { (cur - step).max(target) };
        self.mag.set_reveal(next);

        if (next - target).abs() < 0.001 {
            self.land(target);
            return ControlFlow::Break;
        }
        ControlFlow::Continue
    }

    /// The frame clock stopped feeding us (or never started). Put the surface
    /// where the animation was going and let go of the tick callback.
    fn settle(&self) {
        let target = {
            let st = self.st.borrow();
            if !st.animating {
                return;
            }
            st.target
        };
        {
            let mut st = self.st.borrow_mut();
            st.tick_gen = st.tick_gen.wrapping_add(1);
        }
        self.land(target);
    }

    /// Apply the end state of a reveal/hide, from whichever path got there.
    fn land(&self, target: f64) {
        self.cancel_watchdog();
        self.mag.set_reveal(target);
        self.st.borrow_mut().animating = false;
        if target <= 0.0 {
            self.dock_win.set_visible(false);
            self.block_rearm();
        }
    }

    fn cancel_watchdog(&self) {
        if let Some(id) = self.st.borrow_mut().watchdog.take() {
            id.remove();
        }
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
