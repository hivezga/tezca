//! The bar surfaces + the manager that drives them.
//!
//! One [`Surface`] per monitor: a layer-shell `Window` (namespace `tezca-bar`)
//! whose child is a `.bar` CenterBox laid out left · centre · right, matching the
//! prototype. The ultrawide primary shows the full cluster; a monitor narrower
//! than `compact_width` drops the per-app label and tightens (per-monitor
//! adaptive). [`Bar`] owns every surface, the live palette + CSS, and the poll
//! timers that push CPU/MEM/net/audio/clock/notification state into the widgets.
//!
//! Data all comes from std/shell-out readers (see `hypr`, `sysinfo`,
//! `nowplaying`, `notify`); this file is purely the GTK4 widget tree + wiring.

use crate::config::{Clutter, Config, Mod, Numerals, Region, Shape, Slot};
use crate::draw::{self, SharedPalette, Sparkline};
use crate::icon::{self, Icon};
use crate::sysinfo::{self, CpuMeter, Net, NetMeter, Throughput};
use crate::theme::{CssStack, Palette};
use crate::{
    ai, bluetooth, camera, custom, hypr, mic, notify, nowplaying, osd, popovers, session, tray,
    weather,
};
use gtk4::gdk;
use gtk4::glib::{self, ControlFlow};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, CenterBox, Image, Label, Orientation, Popover, Window};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

// ===========================================================================
// Manager
// ===========================================================================

/// How long to let the output list settle before re-placing the bars. Waking a
/// multi-monitor desk emits several add/remove events, and re-anchoring against
/// a half-assembled list only means doing it again.
const MONITOR_SETTLE_MS: u64 = 400;

/// The outputs that can actually hold a surface right now, as
/// `(connector, monitor)` — the connector being the identity that outlives the
/// `GdkMonitor` across a power cycle.
///
/// A monitor already invalidated, or still reporting a zero-width geometry, is
/// on its way in or out; anchoring to one puts the bar nowhere. Skipping it here
/// is safe because leaving the list is itself an event, so we come back.
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

pub struct Bar {
    /// One surface per live output. Mutable because the monitor list is: a
    /// display that sleeps hard enough to drop its link takes its output with
    /// it, and a surface has to be built for the one that comes back. See
    /// [`Bar::sync_monitors`].
    surfaces: RefCell<Vec<Rc<Surface>>>,
    /// The debounce for [`Bar::schedule_sync`]: the pending pass, and whether
    /// anything in the current burst asked for a forced re-anchor.
    sync_pending: RefCell<Option<glib::SourceId>>,
    sync_force: Cell<bool>,
    /// Whether SIGUSR1 has hidden the bars, so a surface coming back from a
    /// sleeping monitor is restored to the state the user actually chose.
    hidden: Cell<bool>,
    /// Held so a surface can be built after startup, when a monitor appears.
    app: gtk4::Application,
    /// Same, for the two collections `Surface::build` needs.
    customs: Vec<custom::CustomModule>,
    shared_state: Shared,
    cfg: Config,
    palette: SharedPalette,
    css: CssStack,
    cpu: RefCell<CpuMeter>,
    netmeter: RefCell<NetMeter>,
    throughput: Rc<RefCell<Throughput>>,
    tray_cmd: async_channel::Sender<tray::TrayCmd>,
    tray_items: RefCell<Vec<tray::TrayItemView>>,
    tray_menus: RefCell<HashMap<String, tray::MenuNode>>,
    /// Latest AI usage snapshot, shared with every surface's popover so they
    /// all render the same poll without re-fetching.
    ai: Rc<RefCell<ai::Snapshot>>,
    /// The weather module's latest reading, shared with its popover so opening
    /// one never triggers a fetch.
    weather: Rc<RefCell<weather::Snapshot>>,
    battery_history: Rc<RefCell<VecDeque<f64>>>,
    /// Down/up Mb/s per controls tick, for the network popover's throughput
    /// chart. Both directions are kept so the chart can scale them together —
    /// an upload spike has to read as small next to a download that dwarfs it.
    net_history: Rc<RefCell<VecDeque<(f64, f64)>>>,
    /// The moves the last compaction pass dispatched — if the same plan recurs
    /// (a window that wouldn't move), we skip it rather than loop forever.
    last_compaction: RefCell<Vec<(i32, i32)>>,
    /// Last (volume, muted) the OSD reacted to — so a sink event that didn't
    /// actually change the master volume (e.g. an app stream) doesn't flash it.
    last_audio: RefCell<Option<(u32, bool)>>,
    /// Last backlight brightness percent the OSD reacted to. `None` when there's
    /// no backlight (a desktop) — the fast brightness poll never starts there.
    last_brightness: RefCell<Option<u32>>,
    /// True when the camera indicator is in some layout region — gates the
    /// `/proc` scan so the poll costs nothing when the module isn't shown.
    has_camera: bool,
    /// True when the mic indicator is in some layout region — gates the `pactl`
    /// query the same way.
    has_mic: bool,
    /// True when the Bluetooth module is placed — gates the `bluetoothctl` poll,
    /// which is the most expensive of the three (it spawns a process per call).
    has_bluetooth: bool,
    /// Session-state modules (keep-awake / night light / recording). Each reads
    /// a small file, so they share one gate per module.
    has_caffeine: bool,
    has_recording: bool,
    /// The night-light state the schedule last settled on. `tezca night apply`
    /// runs only when this changes, so a configured schedule costs one process at
    /// the boundary rather than one every tick.
    last_night_active: RefCell<Option<bool>>,
}

impl Bar {
    pub fn build(
        app: &gtk4::Application,
        cfg: Config,
        palette: Palette,
        css: CssStack,
        tray_cmd: async_channel::Sender<tray::TrayCmd>,
        customs: &[custom::CustomModule],
    ) -> Rc<Bar> {
        let display = gdk::Display::default().expect("no display");
        let shared: SharedPalette = Rc::new(RefCell::new(palette));
        let throughput = Rc::new(RefCell::new(Throughput { down_mbps: 0.0, up_mbps: 0.0 }));
        let ai_state: Rc<RefCell<ai::Snapshot>> = Rc::new(RefCell::new(ai::Snapshot::default()));
        let weather_state: Rc<RefCell<weather::Snapshot>> =
            Rc::new(RefCell::new(weather::Snapshot::default()));
        let battery_history: Rc<RefCell<VecDeque<f64>>> = Rc::new(RefCell::new(VecDeque::new()));
        let net_history: Rc<RefCell<VecDeque<(f64, f64)>>> = Rc::new(RefCell::new(VecDeque::new()));
        let surface_state = Shared {
            throughput: throughput.clone(),
            ai: ai_state.clone(),
            weather: weather_state.clone(),
            battery_history: battery_history.clone(),
            net_history: net_history.clone(),
        };

        let mut surfaces = Vec::new();
        for (_, monitor) in live_monitors(&display) {
            let s = Surface::build(app, &monitor, &cfg, &shared, &surface_state, customs);
            surfaces.push(s);
        }

        let has_camera = cfg.uses_mod(Mod::Camera);
        let has_mic = cfg.uses_mod(Mod::Microphone);
        let has_bluetooth = cfg.uses_mod(Mod::Bluetooth);
        let has_caffeine = cfg.uses_mod(Mod::Caffeine);
        // No `has_night`: the night state is read every tick regardless of
        // placement, because the schedule must fire whether or not the glyph is
        // on the bar. An unplaced widget is never parented, so it cannot show.
        let has_recording = cfg.uses_mod(Mod::Recording);
        // Seed the OSD's baseline with the current volume so the user's very
        // first volume change flashes it. `pactl subscribe` emits nothing on
        // connect, so there's no spurious launch event to swallow.
        let a0 = sysinfo::audio();
        // Seed the brightness baseline too — Some only on a machine with a
        // backlight, which is what gates the fast brightness-OSD poll below.
        let b0 = sysinfo::brightness();
        let bar = Rc::new(Bar {
            surfaces: RefCell::new(surfaces),
            sync_pending: RefCell::new(None),
            sync_force: Cell::new(false),
            hidden: Cell::new(false),
            app: app.clone(),
            customs: customs.to_vec(),
            shared_state: surface_state,
            cfg,
            palette: shared,
            css,
            cpu: RefCell::new(CpuMeter::default()),
            netmeter: RefCell::new(NetMeter::default()),
            throughput,
            tray_cmd,
            tray_items: RefCell::new(Vec::new()),
            tray_menus: RefCell::new(HashMap::new()),
            ai: ai_state,
            weather: weather_state,
            battery_history,
            net_history,
            last_compaction: RefCell::new(Vec::new()),
            last_audio: RefCell::new(Some((a0.volume, a0.muted))),
            last_brightness: RefCell::new(b0),
            has_camera,
            has_mic,
            has_bluetooth,
            has_caffeine,
            has_recording,
            last_night_active: RefCell::new(None),
        });

        bar.reseed();
        let all = bar.surfaces.borrow().clone();
        bar.wire_layout_freeze(&all);
        Bar::wire_center_sizing(&all);
        bar.watch_monitors();
        bar.start_timers();
        bar
    }

    /// Fill every surface with the state we already hold, from scratch.
    ///
    /// Runs at startup and again whenever a surface is added, because a
    /// freshly built one is blank: its clock reads nothing, its meters are at
    /// zero, and the poll threads will not speak again until their next
    /// interval. Everything here is a pure re-render of state the bar already
    /// has, so calling it spuriously costs a few reads and no correctness.
    fn reseed(self: &Rc<Self>) {
        self.refresh_hypr();
        self.tick_clock();
        self.tick_cpu();
        self.tick_mem();
        self.tick_gpu();
        self.tick_controls();
        self.tick_bluetooth();
        self.tick_session();
        self.rebuild_tray();
        // The AI and weather snapshots live behind the same handles the
        // surfaces read, so replaying them is how a new surface learns what the
        // poll threads last said. (Custom modules keep no snapshot; they redraw
        // on their own next tick.)
        let ai = self.ai.borrow().clone();
        self.apply_ai(ai);
        let weather = self.weather.borrow().clone();
        self.apply_weather(weather);
    }

    /// Install the recurring poll timers, each holding a weak ref so they stop if
    /// the bar is ever torn down.
    fn start_timers(self: &Rc<Self>) {
        let every = |secs: u32, f: Box<dyn Fn(&Bar)>, me: &Rc<Bar>| {
            let weak = Rc::downgrade(me);
            glib::timeout_add_seconds_local(secs.max(1), move || match weak.upgrade() {
                Some(b) => {
                    f(&b);
                    ControlFlow::Continue
                }
                None => ControlFlow::Break,
            });
        };
        // Clock ticks every second for a live minute rollover.
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
            Some(b) => {
                b.tick_clock();
                ControlFlow::Continue
            }
            None => ControlFlow::Break,
        });
        every(self.cfg.cpu_interval, Box::new(|b| b.tick_cpu()), self);
        every(self.cfg.mem_interval, Box::new(|b| b.tick_mem()), self);
        every(self.cfg.gpu_interval, Box::new(|b| b.tick_gpu()), self);
        // Controls (audio/net/battery/brightness/bell/gamemode/now-playing).
        every(2, Box::new(|b| b.tick_controls()), self);
        // Session state (keep-awake / night light / recording): small file reads,
        // and the night-light schedule's clock.
        every(2, Box::new(|b| b.tick_session()), self);
        // Bluetooth gets its own, slower timer: each poll spawns bluetoothctl
        // (twice more when something is connected), which is far too heavy for
        // the 2-second cluster. Not started at all when the module is unplaced.
        if self.has_bluetooth {
            every(5, Box::new(|b| b.tick_bluetooth()), self);
        }

        // Brightness OSD: there's no event source for a backlight the way `pactl
        // subscribe` gives one for volume, so poll the (one tiny) sysfs file
        // quickly and flash the pill on a change. Only started when a backlight
        // actually exists (Some at seed time) and the OSD is enabled — so this
        // costs literally nothing on a desktop (DDC monitors, no sysfs backlight).
        if self.cfg.osd_enabled && self.last_brightness.borrow().is_some() {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                match weak.upgrade() {
                    Some(b) => {
                        b.tick_brightness_osd();
                        ControlFlow::Continue
                    }
                    None => ControlFlow::Break,
                }
            });
        }
    }

    /// Fast backlight poll: if the brightness moved since last look, flash the
    /// OSD on every monitor. Seeded at startup so the first change flashes.
    fn tick_brightness_osd(&self) {
        let Some(pct) = sysinfo::brightness() else { return };
        if self.last_brightness.borrow().map(|prev| prev == pct).unwrap_or(false) {
            return; // unchanged
        }
        *self.last_brightness.borrow_mut() = Some(pct);
        for s in self.surfaces.borrow().iter() {
            s.osd.show_brightness(pct);
        }
    }

    /// Refresh workspaces + the per-app label from live Hyprland state.
    pub fn refresh_hypr(&self) {
        let snap = hypr::snapshot();
        if self.cfg.compact {
            self.compact_workspaces(&snap);
        }
        for s in self.surfaces.borrow().iter() {
            s.set_workspaces(&snap);
            s.set_app(&snap.active.class);
        }
    }

    /// Per-monitor gap-compaction: within each assigned workspace set, pull
    /// occupied workspaces down to the lowest slots, preserving order and never
    /// moving the monitor's visible workspace (so nothing shifts under you).
    /// Only queries windows + dispatches moves when a gap actually exists.
    fn compact_workspaces(&self, snap: &hypr::Snapshot) {
        let occ: HashSet<i32> =
            snap.workspaces.iter().filter(|w| w.windows > 0).map(|w| w.id).collect();
        let mut moves = Vec::new();
        for (output, set) in &self.cfg.ws_assign {
            let visible = hypr::active_ws_for(&snap.monitors, output);
            moves.extend(plan_compaction(set, visible, |id| occ.contains(&id)));
        }
        if moves.is_empty() {
            self.last_compaction.borrow_mut().clear();
            return;
        }
        // A repeat of the exact plan means the previous moves didn't take (an
        // immovable window) — bail instead of dispatching in a tight loop.
        if *self.last_compaction.borrow() == moves {
            return;
        }
        *self.last_compaction.borrow_mut() = moves.clone();
        hypr::apply_moves(&moves, &hypr::clients_by_workspace());
    }

    /// A submap change (empty = default submap).
    pub fn set_submap(&self, name: &str) {
        for s in self.surfaces.borrow().iter() {
            s.set_submap(name);
        }
    }

    fn tick_clock(&self) {
        let now = glib::DateTime::now_local().ok();
        let text = now
            .and_then(|d| d.format(&self.cfg.clock_format).ok())
            .map(|g| g.to_string())
            .unwrap_or_default();
        let (date, time) = split_clock(&text);
        for s in self.surfaces.borrow().iter() {
            s.clock_label.set_text(date);
            s.clock_time.set_text(time);
            s.clock_time.set_visible(!time.is_empty());
            // Piggy-backed on the one timer that already runs every second. The
            // side clusters change width when a module appears, a tray icon
            // registers or the clock string itself gets longer, and the centre's
            // budget moves with them; recomputing is two `measure` calls and a
            // comparison, and lands on the same answer (and so does nothing)
            // every tick but the ones where the layout actually changed.
            s.retarget_center();
        }
    }

    fn tick_cpu(&self) {
        let frac = self.cpu.borrow_mut().sample();
        let pct = (frac * 100.0).round() as u32;
        // The package temp is one small sysfs read, and it is the number that
        // explains a load figure — 90% at 55° and 90% at 95° are different
        // situations. Absent (no sensor) simply hides the sub-label.
        let temp =
            sysinfo::cpu_temp().map(|c| format!("{}°", c.round() as i64)).unwrap_or_default();
        for s in self.surfaces.borrow().iter() {
            s.cpu_spark.push(frac);
            s.cpu_val.set_text(&pct_text(pct));
            set_sub(&s.cpu_sub, &temp);
            s.sys_parts.borrow_mut().0 = format!("{pct}%");
            s.apply_clusters();
        }
    }

    fn tick_mem(&self) {
        // `mem_detail` reads the same /proc/meminfo as `mem` and costs the same,
        // but keeps the absolute figures — "18.4G of 32" answers "can I open
        // another one of these" in a way that "57%" does not.
        let d = sysinfo::mem_detail();
        let frac = if d.total_kb > 0.0 { (d.used_kb / d.total_kb).clamp(0.0, 1.0) } else { 0.0 };
        let used = format!("{:.1}G", d.used_kb / 1024.0 / 1024.0);
        let total = format!("/{:.0}", d.total_kb / 1024.0 / 1024.0);
        for s in self.surfaces.borrow().iter() {
            s.mem_spark.push(frac);
            s.mem_val.set_text(&used);
            set_sub(&s.mem_sub, &total);
            s.sys_parts.borrow_mut().1 = used.clone();
            s.apply_clusters();
        }
    }

    fn tick_gpu(&self) {
        // One batched read rather than a utilisation call now and a telemetry
        // call when the popover opens: on NVIDIA both are `nvidia-smi`, so
        // asking for every field at once costs exactly what asking for one did.
        let detail = sysinfo::gpu_detail();
        let frac = detail
            .as_ref()
            .and_then(|d| d.util_pct)
            .map(|p| (p / 100.0).clamp(0.0, 1.0))
            .or_else(sysinfo::gpu);
        let Some(frac) = frac else {
            for s in self.surfaces.borrow().iter() {
                s.show(&s.gpu_metric, false);
                s.sys_parts.borrow_mut().2.clear();
                s.apply_clusters();
            }
            return;
        };
        let pct = (frac * 100.0).round() as u32;
        let sub = detail.as_ref().map(gpu_sub_text).unwrap_or_default();
        for s in self.surfaces.borrow().iter() {
            s.gpu_spark.push(frac);
            s.gpu_val.set_text(&pct_text(pct));
            set_sub(&s.gpu_sub, &sub);
            s.show(&s.gpu_metric, true);
            s.sys_parts.borrow_mut().2 =
                if sub.is_empty() { format!("{pct}%") } else { format!("{pct}% {sub}") };
            s.apply_clusters();
        }
    }

    /// The 2-second cluster: audio, network, battery, brightness, bell, gamemode,
    /// now-playing — plus the throughput sample that feeds the network popover.
    fn tick_controls(&self) {
        let audio = sysinfo::audio();
        let net = sysinfo::net();
        let battery = sysinfo::battery();
        let brightness = sysinfo::brightness();
        let bell = notify::state();
        let game = sysinfo::gamemode_on();
        let np = nowplaying::current();
        // Privacy indicators — only probe when the module is actually shown.
        let cam = self.has_camera.then(camera::poll).unwrap_or_default();
        let microphone = self.has_mic.then(mic::poll).unwrap_or_default();

        *self.throughput.borrow_mut() = self.netmeter.borrow_mut().sample(2.0);
        {
            let t = self.throughput.borrow();
            let mut h = self.net_history.borrow_mut();
            h.push_back((t.down_mbps, t.up_mbps));
            while h.len() > NET_POINTS {
                h.pop_front();
            }
        }
        // One trace point per tick, capped — the popover shows this session
        // only, which is what it says on the label.
        if let Some(b) = &battery {
            let mut h = self.battery_history.borrow_mut();
            h.push_back(b.percent as f64 / 100.0);
            while h.len() > BATTERY_POINTS {
                h.pop_front();
            }
        }

        for s in self.surfaces.borrow().iter() {
            s.set_audio(&audio);
            s.set_net(&net, &self.throughput.borrow());
            s.set_battery(&battery);
            s.set_brightness(brightness);
            s.set_bell(&bell);
            s.set_gamemode(game);
            s.set_camera(&cam);
            s.set_mic(&microphone);
            s.set_nowplaying(np.as_ref());
        }
    }

    /// Keep-awake / night light / recording — three small local-state reads,
    /// each skipped when its module is not placed.
    ///
    /// This is also where the night-light *schedule* is enforced: hyprsunset has
    /// no scheduler of its own, so the saved window is evaluated here and
    /// `tezca night apply` runs only when the answer changes.
    fn tick_session(&self) {
        let caffeine = self.has_caffeine && session::caffeine_on();
        let rec = if self.has_recording { session::recording() } else { Default::default() };
        // Read unconditionally, unlike the other two: the *schedule* has to be
        // enforced whether or not the glyph is on the bar. Requiring the module
        // to be placed before a schedule worked would be a trap.
        let night = session::night(session::minutes_now());

        if night.configured {
            let mut last = self.last_night_active.borrow_mut();
            if *last != Some(night.active) {
                // Skip the very first observation: at startup autostart.lua has
                // already applied the right state, and re-applying would restart
                // hyprsunset for nothing.
                if last.is_some() {
                    session::night_apply();
                }
                *last = Some(night.active);
            }
        }

        for s in self.surfaces.borrow().iter() {
            s.set_caffeine(caffeine);
            s.set_recording(&rec);
            s.set_night(&night);
        }
    }

    /// The 5-second Bluetooth poll. Only runs when the module is placed.
    fn tick_bluetooth(&self) {
        if !self.has_bluetooth {
            return;
        }
        let bt = bluetooth::poll();
        for s in self.surfaces.borrow().iter() {
            s.set_bluetooth(&bt);
        }
    }

    /// A default-sink change came in from `osd::subscribe`. Re-read the real
    /// volume and, if the master level or mute state actually changed, flash the
    /// OSD on every monitor. Comparing against the last value keeps unrelated
    /// sink events (and the burst some servers emit) from flashing it spuriously.
    pub fn show_osd(&self) {
        if !self.cfg.osd_enabled {
            return;
        }
        let a = sysinfo::audio();
        let now = (a.volume, a.muted);
        // The baseline is seeded at startup (see Bar::build), so a sink event
        // that didn't actually move the master volume/mute — an app stream, a
        // routing change — compares equal and is ignored; only a real change
        // flashes the pill.
        if self.last_audio.borrow().map(|prev| prev == now).unwrap_or(false) {
            return;
        }
        *self.last_audio.borrow_mut() = Some(now);
        for s in self.surfaces.borrow().iter() {
            s.osd.show(a.volume, a.muted);
        }
    }

    /// SIGUSR2 — re-read colors.css (CSS + parsed palette) and repaint.
    pub fn reload_palette(&self) {
        *self.palette.borrow_mut() = Palette::load();
        self.css.reload();
        for s in self.surfaces.borrow().iter() {
            s.repaint_drawn();
        }
    }

    /// SIGUSR1 — toggle every bar's visibility (parity with bar-toggle.sh).
    ///
    /// The wanted state is tracked rather than read back off the windows: a
    /// surface parked because its monitor is asleep is already invisible, and
    /// asking each window what it is doing would let one toggle flip the bars
    /// out of step with each other.
    pub fn toggle_visibility(&self) {
        let hide = !self.hidden.get();
        self.hidden.set(hide);
        for s in self.surfaces.borrow().iter() {
            if !s.detached.get() {
                s.window.set_visible(!hide);
            }
        }
    }

    /// Apply a weather reading from the poll thread. The module hides itself
    /// whenever there is no temperature to show — unconfigured, or a failed
    /// first fetch — rather than parking a dash on the bar.
    pub fn apply_weather(&self, snap: weather::Snapshot) {
        *self.weather.borrow_mut() = snap;
        let snap = self.weather.borrow();
        for s in self.surfaces.borrow().iter() {
            s.set_weather(&snap);
        }
    }

    /// Apply an AI usage snapshot from the poll thread. The module shows the
    /// single highest window utilisation across every provider — the number
    /// that actually constrains you — and colours itself at the configured
    /// warn/critical thresholds. It hides entirely when nothing is configured
    /// or no provider's tooling is installed.
    pub fn apply_ai(&self, snap: ai::Snapshot) {
        let pct = snap.peak_pct();
        let resets_at = snap.peak_resets_at();
        let empty = snap.is_empty();
        let (warn, crit) = (self.cfg.ai.warn, self.cfg.ai.critical);
        *self.ai.borrow_mut() = snap;
        for s in self.surfaces.borrow().iter() {
            s.set_ai(pct, resets_at, empty, warn, crit);
        }
    }

    /// Apply one custom-module poll result to every surface that hosts it.
    pub fn apply_custom(&self, out: custom::Output) {
        for s in self.surfaces.borrow().iter() {
            s.set_custom(&out);
        }
    }

    /// Apply a tray update from the D-Bus thread, then repaint every bar's tray.
    pub fn apply_tray(self: &Rc<Self>, update: tray::TrayUpdate) {
        match update {
            tray::TrayUpdate::Items(items) => *self.tray_items.borrow_mut() = items,
            tray::TrayUpdate::Menu { key, root } => {
                self.tray_menus.borrow_mut().insert(key, root);
            }
        }
        self.rebuild_tray();
    }

    /// Watch the display's output list and re-place the bars when it changes.
    ///
    /// This is what makes a monitor power-cycle survivable. A display that
    /// sleeps deeply enough drops its link, and the compositor destroys that
    /// output — but a layer surface anchored to it is not destroyed with it.
    /// Hyprland re-homes the orphan onto a surviving monitor, which is why
    /// every bar ends up stacked on the first screen. Nothing moves them back
    /// when the output returns: the surface is still perfectly happy, just on
    /// the wrong monitor, so the correction has to be ours.
    fn watch_monitors(self: &Rc<Self>) {
        let Some(display) = gdk::Display::default() else { return };
        let weak = Rc::downgrade(self);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(b) = weak.upgrade() {
                b.schedule_sync(false);
            }
        });
    }

    /// Queue a [`Bar::sync_monitors`] for once the output list stops moving.
    ///
    /// Waking a desk emits a burst — a removal and an addition per output, from
    /// two independent sources (GDK's list and Hyprland's event socket), and two
    /// screens rarely come back in the same instant. Coalescing buys one
    /// re-anchor at the end instead of one per event, and since the sync
    /// reconciles the whole list rather than applying a delta, only the last
    /// pass has to be right. A `force` asked for by any event in the burst
    /// carries through to that pass.
    pub fn schedule_sync(self: &Rc<Self>, force: bool) {
        if force {
            self.sync_force.set(true);
        }
        if let Some(id) = self.sync_pending.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(MONITOR_SETTLE_MS),
            move || {
                let Some(b) = weak.upgrade() else { return };
                *b.sync_pending.borrow_mut() = None;
                b.sync_monitors(b.sync_force.replace(false));
            },
        );
        *self.sync_pending.borrow_mut() = Some(id);
    }

    /// Reconcile the surfaces against the live outputs. Idempotent.
    ///
    /// Keyed on the connector name (`DP-1`), which is the only identity that
    /// survives an output being destroyed and recreated — the `GdkMonitor` is a
    /// fresh object each time, so holding one would be holding a corpse.
    ///
    /// A surface whose output has gone is unmapped and kept, not destroyed.
    /// Destroying a layer-shell window after its output is already gone takes
    /// GDK down with it (`gdk_surface_get_display` asserts, then the process
    /// dies), and a parked surface is the cheaper answer anyway: the monitor
    /// that just went to sleep is overwhelmingly likely to come back, and
    /// re-anchoring it costs nothing next to rebuilding every widget.
    ///
    /// `force` re-anchors even when the monitor object looks unchanged. The
    /// display-side signal never fires for a compositor that shuffles layer
    /// surfaces without dropping the output, which is what the Hyprland
    /// `monitoradded`/`monitorremoved` events are routed here to cover.
    fn sync_monitors(self: &Rc<Self>, force: bool) {
        let Some(display) = gdk::Display::default() else { return };
        let live = live_monitors(&display);
        // Every output down at once (both asleep, or mid-transition) is not a
        // reason to touch anything — the next event brings them back.
        if live.is_empty() {
            return;
        }

        // Park the surfaces whose output has gone. Left mapped they are what the
        // user actually sees go wrong: the compositor re-homes an orphaned layer
        // surface onto a surviving monitor, so its bar appears stacked under the
        // one that belongs there.
        for s in self.surfaces.borrow().iter() {
            if !live.iter().any(|(name, _)| *name == s.output) && !s.detached.get() {
                s.detached.set(true);
                s.window.set_visible(false);
                s.osd.park();
            }
        }

        // Re-anchor what is live, and build a surface for any output that hasn't
        // got one. Setting the monitor on a layer surface re-maps it onto that
        // output — the move back the compositor never makes.
        let mut fresh: Vec<Rc<Surface>> = Vec::new();
        let mut reattached = false;
        for (name, monitor) in &live {
            let existing = self.surfaces.borrow().iter().find(|s| s.output == *name).cloned();
            let Some(s) = existing else {
                fresh.push(Surface::build(
                    &self.app,
                    monitor,
                    &self.cfg,
                    &self.palette,
                    &self.shared_state,
                    &self.customs,
                ));
                continue;
            };
            // A surface already on this exact monitor object is where it
            // belongs; re-mapping it would flash a bar that never moved.
            let placed = s.window.monitor().is_some_and(|cur| cur.is_valid() && cur == *monitor);
            if force || !placed || s.detached.get() {
                s.window.set_monitor(Some(monitor));
                s.osd.set_output(monitor);
            }
            if s.detached.replace(false) {
                // Back on its own monitor. Respect a bar the user has hidden
                // with SIGUSR1 — a monitor waking up is not a reason to undo it.
                s.window.set_visible(!self.hidden.get());
                reattached = true;
            }
        }

        if fresh.is_empty() && !reattached {
            return;
        }
        self.wire_layout_freeze(&fresh);
        Bar::wire_center_sizing(&fresh);
        self.surfaces.borrow_mut().extend(fresh);
        self.reseed();
    }

    /// Hold the bar's layout still for as long as a popover is open.
    ///
    /// GTK keeps a popover glued to the widget it is anchored to, so anything
    /// that re-flows the cluster while one is open slides that anchor sideways
    /// and takes the popover with it — a privacy dot appearing, a tray icon
    /// arriving, the GPU group vanishing. Freezing on `show` and thawing on
    /// `closed` leaves a popover the user is reading exactly where they clicked.
    ///
    /// Both handlers hold weak refs: the popovers are owned by the surface, so
    /// strong ones would be a cycle that never drops.
    ///
    /// Takes the surfaces explicitly rather than reading `self.surfaces`, so a
    /// monitor arriving late wires only its own popovers — running over the
    /// whole list again would give every existing popover a second handler.
    /// Give each surface its first centre measurement.
    ///
    /// [`Surface::retarget_center`] needs a real allocation to measure, and a
    /// freshly presented layer surface has none — so ride the frame clock until
    /// the width is non-zero, size the centre once, and stop. From then on the
    /// per-second clock tick keeps it current. Without this the pill would spend
    /// its first second at whatever width it was built with.
    fn wire_center_sizing(surfaces: &[Rc<Surface>]) {
        for s in surfaces {
            let sw = Rc::downgrade(s);
            s.bar_box.add_tick_callback(move |b, _| {
                let Some(s) = sw.upgrade() else { return ControlFlow::Break };
                if b.width() <= 0 {
                    return ControlFlow::Continue;
                }
                s.retarget_center();
                ControlFlow::Break
            });
        }
    }

    fn wire_layout_freeze(self: &Rc<Self>, surfaces: &[Rc<Surface>]) {
        for s in surfaces {
            for pop in &s.popovers {
                let sw = Rc::downgrade(s);
                pop.connect_show(move |_| {
                    if let Some(s) = sw.upgrade() {
                        s.freeze();
                    }
                });
                let sw = Rc::downgrade(s);
                let bw = Rc::downgrade(self);
                pop.connect_closed(move |_| {
                    let (Some(s), Some(b)) = (sw.upgrade(), bw.upgrade()) else { return };
                    // `thaw` reports whether a tray rebuild was skipped; the item
                    // state is still in `tray_items`, so replaying it is enough.
                    if s.thaw() {
                        b.rebuild_tray();
                    }
                });
            }
        }
    }

    /// Rebuild each surface's tray cluster from the current item + menu state.
    fn rebuild_tray(self: &Rc<Self>) {
        let items = self.tray_items.borrow();
        for s in self.surfaces.borrow().iter() {
            // A rebuild destroys and recreates every icon, so a tray menu open
            // over one would lose the very widget it is anchored to — the icon
            // an app refreshes precisely when you are about to click it. Defer;
            // nothing is lost, because the state lives in `tray_items`.
            if s.layout_frozen() {
                s.tray_dirty.set(true);
                continue;
            }
            while let Some(c) = s.tray_box.first_child() {
                s.tray_box.remove(&c);
            }
            for item in items.iter() {
                s.tray_box.append(&self.tray_item(item));
            }
            if !items.is_empty() {
                s.tray_box.append(&sep());
            }
            s.tray_box.set_visible(!items.is_empty());
        }
    }

    /// One tray icon as a clickable box (a plain `Button`'s built-in primary
    /// gesture swallows secondary/middle clicks, so — like the metric groups —
    /// we drive every button off one `GestureClick` and branch on the button:
    /// left = Activate, middle = SecondaryActivate, right = our rendered
    /// DBusMenu popover (or ContextMenu when the app exposes no usable menu).
    fn tray_item(self: &Rc<Self>, item: &tray::TrayItemView) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 0);
        row.add_css_class("tray-item");
        row.set_valign(Align::Center);
        row.append(&tray_icon_widget(&item.icon));
        if !item.tooltip.is_empty() {
            row.set_tooltip_text(Some(&item.tooltip));
        }

        let menu = self.tray_menus.borrow().get(&item.key).cloned();
        let pop =
            menu.map(|root| popovers::tray_menu(&row, &root, &item.key, self.tray_cmd.clone()));

        let click = gtk4::GestureClick::new();
        click.set_button(0); // every button; branch in the handler
        let (cmd, key) = (self.tray_cmd.clone(), item.key.clone());
        click.connect_released(move |g, _, _, _| match g.current_button() {
            gdk::BUTTON_PRIMARY => {
                let _ = cmd.send_blocking(tray::TrayCmd::Activate(key.clone()));
            }
            gdk::BUTTON_MIDDLE => {
                let _ = cmd.send_blocking(tray::TrayCmd::SecondaryActivate(key.clone()));
            }
            gdk::BUTTON_SECONDARY => match &pop {
                Some(p) => p.popup(),
                None => {
                    let _ = cmd.send_blocking(tray::TrayCmd::ContextMenu(key.clone()));
                }
            },
            _ => {}
        });
        row.add_controller(click);
        row
    }
}

/// Build the GTK image for a tray icon (themed name or raw ARGB pixmap).
fn tray_icon_widget(icon: &tray::TrayIcon) -> Image {
    let img = match icon {
        tray::TrayIcon::Named { name, theme_path } => {
            if let (Some(path), Some(display)) = (theme_path, gdk::Display::default()) {
                let theme = gtk4::IconTheme::for_display(&display);
                if !theme.search_path().iter().any(|p| p.to_str() == Some(path.as_str())) {
                    theme.add_search_path(path);
                }
            }
            Image::from_icon_name(name)
        }
        tray::TrayIcon::Pixmap { width, height, argb } => {
            let bytes = glib::Bytes::from(argb);
            let texture = gdk::MemoryTexture::new(
                *width,
                *height,
                gdk::MemoryFormat::A8r8g8b8,
                &bytes,
                (*width * 4) as usize,
            );
            Image::from_paintable(Some(&texture))
        }
        tray::TrayIcon::None => Image::from_icon_name("application-x-executable"),
    };
    img.set_pixel_size(18);
    img
}

// ===========================================================================
// One monitor's surface
// ===========================================================================

struct Surface {
    window: Window,
    output: String,
    compact: bool,
    /// True while this surface's output is gone (a monitor asleep or unplugged).
    /// It is unmapped rather than destroyed — tearing down a layer-shell window
    /// whose output has already been destroyed kills GDK — and re-anchored when
    /// the connector comes back. See [`Bar::sync_monitors`].
    detached: Cell<bool>,
    bar_box: CenterBox,
    /// The three regions, kept so [`Surface::retarget_center`] can measure the
    /// side clusters and size the centre to whatever room they leave.
    left: GtkBox,
    center: GtkBox,
    right: GtkBox,
    /// The last text width handed to `np_text`, so a recompute that lands on the
    /// same answer — which is almost all of them — does no work and queues no
    /// resize.
    np_text_w: Cell<i32>,
    /// Whether a player is reporting a track. Kept apart from the pill's
    /// visibility because that is now the *conjunction* of "something is playing"
    /// and "there is room to show it" — two facts arriving from two directions.
    np_playing: Cell<bool>,

    ws_box: GtkBox,
    /// Fixed workspace ids this output's bar always shows (from config), or None
    /// to mirror whatever Hyprland has placed on this monitor.
    ws_assigned: Option<Vec<i32>>,
    /// Hide empty workspaces — show only occupied + the focused one.
    hide_empty: bool,
    numerals: Numerals,
    app_label: Label,

    submap_box: GtkBox,
    submap_label: Label,

    np_box: GtkBox,
    /// The title/artist column. Its width is *set*, not measured — see
    /// [`Surface::retarget_center`].
    np_text: GtkBox,
    np_title: Label,
    np_artist: Label,
    np_art_pic: gtk4::Image,
    /// The art URL currently loaded, so a two-second tick on an unchanged track
    /// doesn't re-decode the same JPEG.
    np_art_url: RefCell<String>,

    cpu_spark: Sparkline,
    cpu_val: Label,
    cpu_sub: Label,
    mem_spark: Sparkline,
    mem_val: Label,
    mem_sub: Label,
    gpu_spark: Sparkline,
    gpu_val: Label,
    gpu_sub: Label,
    gpu_metric: GtkBox,

    net_ctl: Button,
    net_glyph: icon::IconArea,
    net_val: Label,
    net_sub: Label,
    bt_ctl: Button,
    bt_glyph: icon::IconArea,
    bt_val: Label,
    rec_box: GtkBox,
    caffeine_box: GtkBox,
    night_box: GtkBox,

    vol_glyph: icon::IconArea,
    vol_val: Label,
    vol_ctl: Button,

    bri_ctl: GtkBox,
    bri_val: Label,

    bat_ctl: GtkBox,
    bat_glyph: icon::IconArea,
    bat_val: Label,
    bat_sub: Label,

    bell_btn: Button,
    bell_glyph: icon::IconArea,
    bell_count: Label,

    clock_label: Label,
    clock_time: Label,

    gamemode_box: GtkBox,
    tray_box: GtkBox,

    /// Camera-in-use privacy indicator — hidden until an app opens the webcam.
    camera_box: GtkBox,
    /// Microphone-in-use privacy indicator — hidden until an app records.
    mic_box: GtkBox,

    ai_box: GtkBox,
    ai_val: Label,
    ai_sub: Label,

    weather_box: GtkBox,
    weather_val: Label,
    weather_sub: Label,

    /// The two collapsible runs: chip, its summary label, and the box the
    /// members live in. `grouped_*` is which of the pair is currently showing.
    priv_chip_rev: gtk4::Revealer,
    priv_box_rev: gtk4::Revealer,
    sys_chip_rev: gtk4::Revealer,
    sys_box_rev: gtk4::Revealer,
    priv_chip: GtkBox,
    priv_chip_val: Label,
    grouped_priv: Cell<bool>,
    /// Whether each privacy source is live right now, so the chip can say how
    /// many without asking the three modules what they are displaying.
    priv_live: Cell<(bool, bool, bool)>,
    sys_chip: GtkBox,
    sys_chip_val: Label,
    grouped_sys: Cell<bool>,
    /// `(cpu, mem, gpu)` as last rendered, for the collapsed summary. Each tick
    /// owns one field, so the chip is rebuilt from the three most recent.
    sys_parts: RefCell<(String, String, String)>,

    /// This monitor's volume on-screen display (a separate overlay surface).
    osd: Rc<osd::Osd>,

    mirror: gtk4::DrawingArea,

    /// Community/user exec modules, keyed by manifest name.
    custom: HashMap<String, CustomCell>,

    /// Every popover anchored to a module of this bar, kept so the layout freeze
    /// can be wired to all of them once the surface is built.
    popovers: Vec<Popover>,
    /// How many of `popovers` are currently open. See [`Surface::show`].
    open_popovers: Cell<u32>,
    /// Visibility changes held back while a popover is open — at most one entry
    /// per widget, the latest.
    pending_show: RefCell<Vec<(gtk4::Widget, bool)>>,
    /// A tray rebuild that was skipped because a popover was open.
    tray_dirty: Cell<bool>,
}

/// The readings every surface's popovers share with the poll threads.
///
/// One handle rather than three parameters: each is an `Rc<RefCell<_>>` the
/// bar owns and every monitor's popovers borrow, so they travel together and
/// always will.
#[derive(Clone)]
struct Shared {
    throughput: Rc<RefCell<Throughput>>,
    ai: Rc<RefCell<ai::Snapshot>>,
    weather: Rc<RefCell<weather::Snapshot>>,
    /// Charge fractions recorded since launch, for the battery popover's trace.
    battery_history: Rc<RefCell<VecDeque<f64>>>,
    /// Down/up Mb/s recorded since launch, for the network popover's chart.
    net_history: Rc<RefCell<VecDeque<(f64, f64)>>>,
}

impl Surface {
    fn build(
        app: &gtk4::Application,
        monitor: &gdk::Monitor,
        cfg: &Config,
        pal: &SharedPalette,
        shared: &Shared,
        customs: &[custom::CustomModule],
    ) -> Rc<Surface> {
        let throughput = shared.throughput.clone();
        let ai_state = shared.ai.clone();
        let weather_state = shared.weather.clone();
        let battery_history = shared.battery_history.clone();
        let net_history = shared.net_history.clone();
        let output = monitor.connector().map(|s| s.to_string()).unwrap_or_default();
        let compact = monitor.geometry().width() < cfg.compact_width;
        let ws_assigned = cfg.ws_assign.get(&output).cloned();
        let hide_empty = cfg.hide_empty;
        let numerals = cfg.numerals;

        let bar_box = CenterBox::new();
        bar_box.add_css_class("bar");
        bar_box.set_hexpand(true);
        if cfg.shape == Shape::Edge {
            bar_box.add_css_class("edge");
        }
        bar_box.set_size_request(-1, cfg.height);

        // The three regions. Widgets below are all built unconditionally so the
        // update methods (which poke them by field) always have a live target;
        // which ones actually get *parented*, and in what order, is decided by
        // `place_region` from the configured layout at the end of `build`.
        let left = GtkBox::new(Orientation::Horizontal, 0);
        left.set_halign(Align::Start);
        let center = GtkBox::new(Orientation::Horizontal, 0);
        center.set_halign(Align::Center);
        let right = GtkBox::new(Orientation::Horizontal, 0);
        right.set_halign(Align::End);

        // Tezca mirror menu (drawn glyph inside a flat button).
        let mirror = icon::icon(pal, Icon::Mirror).area;
        mirror.set_valign(Align::Center);
        let mirror_btn = Button::new();
        mirror_btn.add_css_class("mirror");
        mirror_btn.set_child(Some(&mirror));
        let tezca_pop = popovers::tezca_menu(&mirror_btn);
        let p = tezca_pop.clone();
        mirror_btn.connect_clicked(move |_| p.popup());

        let app_label = Label::new(Some("Tezca"));
        app_label.add_css_class("appname");
        app_label.add_css_class("idle");

        let ws_box = GtkBox::new(Orientation::Horizontal, 0);
        ws_box.add_css_class("workspaces");

        // Submap indicator (hidden unless in a submap).
        let submap_box = GtkBox::new(Orientation::Horizontal, 0);
        let submap_label = Label::new(None);
        submap_label.add_css_class("submap-label");
        let submap_hint = Label::new(Some("hjkl / arrows · esc"));
        submap_hint.add_css_class("submap-hint");
        submap_box.append(&submap_label);
        submap_box.append(&submap_hint);
        submap_box.set_visible(false);

        // ── CENTER: now-playing ─────────────────────────────────────────
        let np_box = GtkBox::new(Orientation::Horizontal, 10);
        np_box.add_css_class("nowplaying");
        np_box.set_halign(Align::Center);
        // The thumbnail. Stays a gradient placeholder until the player names a
        // local cover file; the picture is swapped in on a track change rather
        // than every tick, which is what `np_art_url` remembers.
        let art = GtkBox::new(Orientation::Horizontal, 0);
        art.add_css_class("np-art");
        // Centred, not filled: the default `Fill` stretches the box to the
        // pill's full inner height, which turns the 26px square into a tall
        // rectangle with the gradient showing above and below the cover.
        art.set_valign(Align::Center);
        // Clip children to the box's own border-radius, so the cover gets the
        // rounded corners the gradient placeholder already has.
        art.set_overflow(gtk4::Overflow::Hidden);
        // A GtkImage rather than a GtkPicture: `pixel_size` makes it measure
        // exactly 26×26 whatever the texture's dimensions are, so cover art can
        // never drive the pill's size — and through it the bar's. A Picture
        // reports the paintable's own size as its natural size, and a
        // layer-shell surface sized to its content then grows to match.
        // `art_texture` hands over a square, so nothing is letterboxed.
        let np_art_pic = gtk4::Image::new();
        np_art_pic.set_pixel_size(26);
        np_art_pic.set_visible(false);
        art.append(&np_art_pic);
        // The text column. Its width comes from `retarget_center`, never from the
        // track: a label sized to its own text is what made the pill breathe on
        // every song change, and a pill that changes width changes where its
        // centre is. Both labels therefore ellipsize and are told to ask for
        // almost nothing (`max_width_chars(1)`), so the column measures exactly
        // the width it is given and no more.
        //
        // `halign: Fill` with `xalign: 0` rather than `halign: Start` — Start
        // would allocate each label only its own natural width, which after
        // capping the natural is a couple of characters, and the title would
        // ellipsize to nothing in a column with room to spare.
        let np_text = GtkBox::new(Orientation::Vertical, 0);
        let np_title = Label::new(None);
        np_title.add_css_class("np-title");
        let np_artist = Label::new(None);
        np_artist.add_css_class("np-artist");
        for l in [&np_title, &np_artist] {
            l.set_halign(Align::Fill);
            l.set_xalign(0.0);
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
        }
        np_text.append(&np_title);
        np_text.append(&np_artist);
        let eq = draw::equalizer(pal);
        np_box.append(&art);
        np_box.append(&np_text);
        np_box.append(&eq);
        np_box.set_visible(false);
        // Left click opens the transport popover, middle click keeps the
        // one-gesture play/pause the pill has always had, scroll seeks. One
        // all-buttons gesture rather than a Button, for the same reason the
        // tray items use one: a GtkButton's built-in primary gesture swallows
        // the secondary and middle ones.
        let np_pop = popovers::nowplaying_detail(&np_box, pal);
        let click = gtk4::GestureClick::new();
        click.set_button(0);
        {
            let pop = np_pop.clone();
            click.connect_released(move |g, _, _, _| match g.current_button() {
                gdk::BUTTON_MIDDLE => nowplaying::play_pause(),
                gdk::BUTTON_PRIMARY => pop.popup(),
                _ => {}
            });
        }
        np_box.add_controller(click);
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(|_, _, dy| {
            nowplaying::seek(if dy < 0.0 { 5 } else { -5 });
            glib::Propagation::Stop
        });
        np_box.add_controller(scroll);

        // ── RIGHT cluster widgets (parented by place_region below) ───────
        // Game mode (hidden unless on).
        let gamemode_box = GtkBox::new(Orientation::Horizontal, 0);
        gamemode_box.add_css_class("gamemode");
        let game_glyph = icon::glowing_icon(pal, Icon::GameMode);
        game_glyph.area.add_css_class("glyph");
        game_glyph.set_glow(true);
        gamemode_box.append(&game_glyph.area);
        gamemode_box.set_visible(false);

        // Camera-in-use privacy indicator — a lone webcam glyph, hidden until an
        // application opens `/dev/video*`. The tooltip names the holding app(s).
        let camera_box = GtkBox::new(Orientation::Horizontal, 0);
        camera_box.add_css_class("camera");
        camera_box.set_valign(Align::Center);
        let camera_glyph = icon::glowing_icon(pal, Icon::Camera);
        camera_glyph.area.add_css_class("glyph");
        camera_glyph.set_glow(true);
        camera_box.append(&camera_glyph.area);
        camera_box.set_visible(false);
        camera_box.set_has_tooltip(true);

        // Microphone-in-use privacy indicator — same treatment as the camera one,
        // driven by `pactl` recording streams (see mic.rs).
        let mic_box = GtkBox::new(Orientation::Horizontal, 0);
        mic_box.add_css_class("mic");
        mic_box.set_valign(Align::Center);
        let mic_glyph = icon::glowing_icon(pal, Icon::Mic);
        mic_glyph.area.add_css_class("glyph");
        mic_glyph.set_glow(true);
        mic_box.append(&mic_glyph.area);
        mic_box.set_visible(false);
        mic_box.set_has_tooltip(true);

        // Screen-recording indicator — the third privacy dot, beside camera and
        // microphone. Red, and shown for ANY recorder (see session.rs), not only
        // one that `tezca record` started.
        let rec_box = GtkBox::new(Orientation::Horizontal, 0);
        rec_box.add_css_class("recording");
        rec_box.set_valign(Align::Center);
        let rec_glyph = icon::glowing_icon(pal, Icon::Recording);
        rec_glyph.area.add_css_class("glyph");
        rec_glyph.set_glow(true);
        rec_box.append(&rec_glyph.area);
        rec_box.set_visible(false);
        rec_box.set_has_tooltip(true);

        // Keep awake ("caffeine") — a click toggles the systemd idle inhibitor
        // through the CLI, so the bar and `tezca idle inhibit` cannot disagree.
        let caffeine_box = GtkBox::new(Orientation::Horizontal, 0);
        caffeine_box.add_css_class("caffeine");
        caffeine_box.set_valign(Align::Center);
        let caffeine_glyph = icon::icon(pal, Icon::Caffeine);
        caffeine_glyph.area.add_css_class("glyph");
        caffeine_box.append(&caffeine_glyph.area);
        caffeine_box.set_visible(false);
        caffeine_box.set_has_tooltip(true);
        {
            // One all-buttons GestureClick on a plain Box, not a Button: a
            // GtkButton's built-in primary gesture swallows the others, which is
            // the bug the tray items hit.
            let click = gtk4::GestureClick::new();
            click.set_button(0);
            click.connect_pressed(|_, _, _, _| {
                let _ = std::process::Command::new("tezca")
                    .args(["idle", "inhibit", "toggle"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            });
            caffeine_box.add_controller(click);
        }

        // Night light — visible only while the filter is actually on (schedule
        // included), so it reads as "this is why the screen is warm".
        let night_box = GtkBox::new(Orientation::Horizontal, 0);
        night_box.add_css_class("night");
        night_box.set_valign(Align::Center);
        let night_glyph = icon::icon(pal, Icon::Night);
        night_glyph.area.add_css_class("glyph");
        night_box.append(&night_glyph.area);
        night_box.set_visible(false);
        night_box.set_has_tooltip(true);
        {
            let click = gtk4::GestureClick::new();
            click.set_button(0);
            click.connect_pressed(|_, _, _, _| {
                let _ = std::process::Command::new("tezca")
                    .args(["night", "toggle"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            });
            night_box.add_controller(click);
        }

        // AI provider usage — hidden until the poll thread reports something
        // worth showing (see ai.rs). Sits beside the tray because that is where
        // "ambient status from elsewhere" lives on this bar.
        let ai_box = GtkBox::new(Orientation::Horizontal, 7);
        ai_box.add_css_class("ai");
        ai_box.set_valign(Align::Center);
        let ai_glyph = icon::icon(pal, Icon::AiUsage);
        ai_glyph.area.add_css_class("glyph");
        let ai_val = Label::new(None);
        ai_val.add_css_class("ai-val");
        // How long until the window resets. "68%" alone does not tell you
        // whether to slow down; "68%, 4h 12m to go" does.
        let ai_sub = Label::new(None);
        ai_sub.add_css_class("control-sub");
        ai_sub.set_visible(false);
        ai_box.append(&ai_glyph.area);
        ai_box.append(&ai_val);
        ai_box.append(&ai_sub);
        ai_box.set_visible(false);
        let ai_pop = popovers::ai_detail(&ai_box, ai_state);
        attach_detail(&ai_box, ai_pop.clone());

        // Weather — like the AI module, hidden until its poll thread reports
        // something. Off entirely unless configured; see weather.rs.
        let weather_box = GtkBox::new(Orientation::Horizontal, 7);
        weather_box.add_css_class("weather");
        weather_box.set_valign(Align::Center);
        let weather_glyph = icon::icon(pal, Icon::Weather);
        weather_glyph.area.add_css_class("glyph");
        let weather_val = Label::new(None);
        weather_val.add_css_class("control-val");
        let weather_sub = Label::new(None);
        weather_sub.add_css_class("control-sub");
        weather_sub.set_visible(false);
        weather_box.append(&weather_glyph.area);
        weather_box.append(&weather_val);
        weather_box.append(&weather_sub);
        weather_box.set_visible(false);
        weather_box.set_has_tooltip(true);
        let weather_pop = popovers::weather_detail(&weather_box, weather_state);
        attach_detail(&weather_box, weather_pop.clone());

        // System tray (StatusNotifierItem icons) — filled live by the tray
        // thread; hidden until the first item registers.
        let tray_box = GtkBox::new(Orientation::Horizontal, 2);
        tray_box.add_css_class("tray");
        tray_box.set_valign(Align::Center);
        tray_box.set_visible(false);

        // Metrics: CPU + MEM sparklines.
        let cpu_spark = draw::sparkline(pal, draw::SparkColor::Accent);
        let cpu_val = Label::new(Some(&pct_text(0)));
        cpu_val.add_css_class("metric-val");
        let cpu_sub = Label::new(None);
        let cpu_metric = metric(G_CPU_LABEL, &cpu_spark.area, &cpu_val, &cpu_sub);

        let mem_spark = draw::sparkline(pal, draw::SparkColor::Gold);
        let mem_val = Label::new(Some(&pct_text(0)));
        mem_val.add_css_class("metric-val");
        let mem_sub = Label::new(None);
        let mem_metric = metric(G_MEM_LABEL, &mem_spark.area, &mem_val, &mem_sub);

        // GPU — hidden until the first successful read (absent on GPU-less rigs).
        let gpu_spark = draw::sparkline(pal, draw::SparkColor::AccentDim);
        let gpu_val = Label::new(Some(&pct_text(0)));
        gpu_val.add_css_class("metric-val");
        let gpu_sub = Label::new(None);
        let gpu_metric = metric(G_GPU_LABEL, &gpu_spark.area, &gpu_val, &gpu_sub);
        gpu_metric.set_visible(false);

        // Each metric group expands into a glass detail popover on click.
        let cpu_pop = popovers::cpu_detail(&cpu_metric);
        let mem_pop = popovers::mem_detail(&mem_metric);
        let gpu_pop = popovers::gpu_detail(&gpu_metric);
        attach_detail(&cpu_metric, cpu_pop.clone());
        attach_detail(&mem_metric, mem_pop.clone());
        attach_detail(&gpu_metric, gpu_pop.clone());

        // Controls: network (button → popover). Stacked, because the two facts
        // worth having — which network, and how fast it is moving — do not fit
        // side by side without pushing the rest of the cluster off a 2560px
        // monitor.
        let (net_ctl, net_glyph, net_val, net_sub) = control_button_stacked(pal, Icon::Wifi);
        let net_pop = popovers::network(&net_ctl, throughput.clone(), net_history);
        let p = net_pop.clone();
        net_ctl.connect_clicked(move |_| p.popup());

        // Bluetooth (button → device popover). Hidden until the first poll says
        // there is an adapter, so a machine without one shows nothing at all.
        let (bt_ctl, bt_glyph, bt_val) = control_button(pal, Icon::Bluetooth);
        bt_ctl.set_visible(false);
        let bt_pop = popovers::bluetooth(&bt_ctl);
        let p = bt_pop.clone();
        bt_ctl.connect_clicked(move |_| p.popup());

        // Volume (button → mixer popover).
        let (vol_ctl, vol_glyph, vol_val) = control_button(pal, Icon::VolumeHigh);
        let mix_pop = popovers::mixer(&vol_ctl);
        let p = mix_pop.clone();
        vol_ctl.connect_clicked(move |_| p.popup());

        // Brightness (display-only; hidden on desktops with no backlight).
        let bri_ctl = GtkBox::new(Orientation::Horizontal, 6);
        bri_ctl.add_css_class("control");
        bri_ctl.set_valign(Align::Center);
        let bri_glyph = icon::icon(pal, Icon::Brightness);
        bri_glyph.area.add_css_class("glyph");
        let bri_val = Label::new(None);
        bri_val.add_css_class("control-val");
        bri_ctl.append(&bri_glyph.area);
        bri_ctl.append(&bri_val);
        bri_ctl.set_visible(false);

        // Battery (hidden on desktops with no battery).
        let bat_ctl = GtkBox::new(Orientation::Horizontal, 7);
        bat_ctl.add_css_class("control");
        bat_ctl.set_valign(Align::Center);
        let bat_glyph = icon::icon(pal, Icon::Battery);
        bat_glyph.area.add_css_class("glyph");
        let bat_val = Label::new(None);
        bat_val.add_css_class("control-val");
        let bat_sub = Label::new(None);
        bat_sub.add_css_class("control-sub");
        bat_sub.set_visible(false);
        bat_ctl.append(&bat_glyph.area);
        bat_ctl.append(&bat_val);
        bat_ctl.append(&bat_sub);
        bat_ctl.set_visible(false);
        let bat_pop = popovers::battery_detail(&bat_ctl, battery_history);
        attach_detail(&bat_ctl, bat_pop.clone());

        // Notification bell. The badge carries the count rather than a bare
        // dot: "something happened" is a much weaker answer than "three things
        // happened", and the glyph already changes shape when it is non-zero.
        let bell_row = GtkBox::new(Orientation::Horizontal, 6);
        let bell_glyph = icon::icon(pal, Icon::Bell);
        bell_glyph.area.add_css_class("glyph");
        bell_row.append(&bell_glyph.area);
        let bell_count = Label::new(None);
        bell_count.add_css_class("notif-count");
        bell_count.set_visible(false);
        bell_row.append(&bell_count);
        let bell_btn = Button::new();
        bell_btn.add_css_class("bell");
        bell_btn.set_valign(Align::Center);
        bell_btn.set_child(Some(&bell_row));
        bell_btn.connect_clicked(|_| notify::toggle_panel());
        let bell_right = gtk4::GestureClick::new();
        bell_right.set_button(gdk::BUTTON_SECONDARY);
        bell_right.connect_released(|_, _, _, _| notify::toggle_dnd());
        bell_btn.add_controller(bell_right);

        // Clock (button → calendar popover).
        let clock_btn = Button::new();
        clock_btn.add_css_class("clock");
        clock_btn.set_valign(Align::Center);
        // Two labels, not one. The design sets the date in Inter and the time in
        // JetBrains Mono with tabular figures — the time is the part that
        // changes, and proportional digits make it shuffle sideways every
        // minute. `clock_format`'s shipped default separates the two with a run
        // of spaces, which is what `split_clock` looks for; a custom format
        // without one keeps the whole string in the date's font rather than
        // guessing where the time starts.
        let clock_row = GtkBox::new(Orientation::Horizontal, 9);
        clock_row.set_valign(Align::Center);
        let clock_label = Label::new(None);
        let clock_time = Label::new(None);
        clock_time.add_css_class("clock-time");
        clock_time.set_visible(false);
        clock_row.append(&clock_label);
        clock_row.append(&clock_time);
        clock_btn.set_child(Some(&clock_row));
        let cal_pop = popovers::calendar(&clock_btn, Rc::new(cfg.clock_zones.clone()));
        let p = cal_pop.clone();
        clock_btn.connect_clicked(move |_| p.popup());

        // Power → the session menu (which still offers wlogout as its last row).
        let power_btn = Button::new();
        power_btn.add_css_class("power");
        power_btn.set_child(Some(&icon::icon(pal, Icon::Power).area));
        let session_pop = popovers::session_menu(&power_btn);
        let p = session_pop.clone();
        power_btn.connect_clicked(move |_| p.popup());

        // ── Custom (community/user exec) modules ─────────────────────────
        // Build one widget per discovered manifest, keyed by name, so a
        // `custom:<name>` slot in the layout can resolve to it. Filled live by
        // the poll thread via `apply_custom`; each stays hidden until it first
        // prints something.
        let mut custom_cells: HashMap<String, CustomCell> = HashMap::new();
        for m in customs {
            custom_cells.insert(m.name.clone(), CustomCell::build(m));
        }

        // ── Collapsible clusters ─────────────────────────────────────────
        // Each is a chip that stands in for a run of modules, plus the box the
        // run actually lives in. Clicking either swaps which one is showing;
        // the members keep their own auto-hide logic untouched inside the box.
        let (priv_chip, priv_chip_val) = cluster_chip(pal, "", "priv-chip", 7);
        let priv_box = GtkBox::new(Orientation::Horizontal, 0);
        priv_box.add_css_class("cluster");
        let (sys_chip, sys_chip_val) = cluster_chip(pal, G_SYS_LABEL, "sys-chip", 9);
        let sys_box = GtkBox::new(Orientation::Horizontal, 0);
        sys_box.add_css_class("cluster");

        // Each half of a cluster lives in a revealer so expanding is a slide
        // rather than a jump. GTK4 cannot transition `max-width` the way the
        // design's CSS does, and a run of modules appearing between two
        // neighbours in one frame reads as the bar having been rebuilt; the
        // revealer animates the width it occupies, which is the same effect.
        //
        // The pair slides in opposite directions on purpose: the chip leaves
        // towards where its members are about to appear, so the two read as one
        // thing unfolding rather than two swapping.
        let rev = |child: &GtkBox, dir: gtk4::RevealerTransitionType| {
            // The revealer decides whether this is on screen now, so the child
            // must not also be hiding itself — `cluster_chip` builds its chip
            // invisible, which used to be undone by the `show` call that
            // `set_reveal_child` replaced.
            child.set_visible(true);
            let r = gtk4::Revealer::new();
            r.set_child(Some(child));
            r.set_transition_type(dir);
            r.set_transition_duration(CLUSTER_MS);
            r.set_valign(Align::Center);
            r
        };
        let priv_chip_rev = rev(&priv_chip, gtk4::RevealerTransitionType::SlideRight);
        let priv_box_rev = rev(&priv_box, gtk4::RevealerTransitionType::SlideLeft);
        let sys_chip_rev = rev(&sys_chip, gtk4::RevealerTransitionType::SlideRight);
        let sys_box_rev = rev(&sys_box, gtk4::RevealerTransitionType::SlideLeft);

        // ── Place modules per the configured layout ──────────────────────
        // Resolve a slot to the widget built above. Separators are handled by
        // `place_region`; every built-in maps to exactly one widget, so a
        // duplicate in the layout is ignored (a GTK widget has one parent).
        let resolve_slot = |slot: &Slot| -> Option<gtk4::Widget> {
            use gtk4::prelude::Cast;
            Some(match slot {
                Slot::Custom(name) => {
                    return custom_cells.get(name).map(|c| c.container.clone().upcast())
                }
                Slot::Mod(m) => match m {
                    Mod::Mirror => mirror_btn.clone().upcast(),
                    Mod::Appname => app_label.clone().upcast(),
                    Mod::Workspaces => ws_box.clone().upcast(),
                    Mod::Submap => submap_box.clone().upcast(),
                    Mod::NowPlaying => np_box.clone().upcast(),
                    Mod::GameMode => gamemode_box.clone().upcast(),
                    Mod::Camera => camera_box.clone().upcast(),
                    Mod::Microphone => mic_box.clone().upcast(),
                    Mod::Ai => ai_box.clone().upcast(),
                    Mod::Weather => weather_box.clone().upcast(),
                    Mod::Tray => tray_box.clone().upcast(),
                    Mod::Cpu => cpu_metric.clone().upcast(),
                    Mod::Mem => mem_metric.clone().upcast(),
                    Mod::Gpu => gpu_metric.clone().upcast(),
                    Mod::Network => net_ctl.clone().upcast(),
                    Mod::Bluetooth => bt_ctl.clone().upcast(),
                    Mod::Recording => rec_box.clone().upcast(),
                    Mod::Caffeine => caffeine_box.clone().upcast(),
                    Mod::NightLight => night_box.clone().upcast(),
                    Mod::Volume => vol_ctl.clone().upcast(),
                    Mod::Brightness => bri_ctl.clone().upcast(),
                    Mod::Battery => bat_ctl.clone().upcast(),
                    Mod::Bell => bell_btn.clone().upcast(),
                    Mod::Clock => clock_btn.clone().upcast(),
                    Mod::Power => power_btn.clone().upcast(),
                    Mod::Sep => return None,
                },
            })
        };

        // Under the hover strategy the modules you read on purpose keep full
        // weight and the ambient ones fade back until you approach the bar.
        // Tagged here rather than in each module's constructor so the whole
        // policy is one list (`Mod::is_ambient`) and one class.
        let fade_ambient = cfg.clutter == Clutter::Hover;
        let resolve = |slot: &Slot| -> Option<gtk4::Widget> {
            let w = resolve_slot(slot)?;
            if fade_ambient && matches!(slot, Slot::Mod(m) if m.is_ambient()) {
                w.add_css_class("ambient");
            }
            Some(w)
        };
        let drop_tier3 = cfg.clutter == Clutter::Tiers;
        // Only the right cluster is long enough to be worth collapsing, and it
        // is the only region the design groups.
        let clusters = ClusterSlots {
            privacy: (&priv_chip_rev, &priv_box_rev),
            system: (&sys_chip_rev, &sys_box_rev),
        };
        // One `placed` set across all three regions, not one per region. A GTK
        // widget has exactly one parent, so a module listed in two regions can
        // only ever render in the first — sharing the set makes the second
        // listing a no-op *and* says so, instead of leaving a module apparently
        // configured into a region it will never appear in. (That is not
        // hypothetical: the settings editor's "+ add" button used to drop
        // `nowplaying` into the left region while it was still in the centre,
        // which silently un-centred the bar.)
        let mut placed: HashSet<Slot> = HashSet::new();
        let regions = [
            (&left, Region::Left, None),
            (&center, Region::Center, None),
            (&right, Region::Right, Some(&clusters)),
        ];
        for (container, region, clusters) in regions {
            let slots = cfg.layout_for(region, &output);
            place_region(container, slots, compact, drop_tier3, clusters, &resolve, &mut placed);
        }

        // Grouping only exists where a run actually got placed — a layout with
        // no privacy modules must not grow a chip that stands for nothing.
        let has_priv = priv_box.first_child().is_some();
        let has_sys = sys_box.first_child().is_some();
        let grouped_priv = Cell::new(has_priv && cfg.clutter == Clutter::Grouped);
        let grouped_sys = Cell::new(has_sys && cfg.clutter == Clutter::Grouped);
        // The fold control only exists where folding is the chosen strategy.
        // Under `all` there is nothing to fold back into, so the glyph would be
        // an unexplained button offering a mode the user did not pick.
        let collapsible = cfg.clutter == Clutter::Grouped;
        let priv_collapse = collapse_button(pal);
        let sys_collapse = collapse_button(pal);
        if has_priv && collapsible {
            priv_box.append(&priv_collapse);
        }
        if has_sys && collapsible {
            sys_box.append(&sys_collapse);
        }
        if fade_ambient {
            bar_box.add_css_class("hover-reveal");
        }

        bar_box.set_start_widget(Some(&left));
        bar_box.set_center_widget(Some(&center));
        bar_box.set_end_widget(Some(&right));

        // ── window / layer-shell ────────────────────────────────────────
        let window = Window::builder().application(app).child(&bar_box).build();
        window.add_css_class("tezca-bar");
        window.init_layer_shell();
        window.set_monitor(Some(monitor));
        window.set_layer(Layer::Top);
        window.set_namespace(Some("tezca-bar"));
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        match cfg.shape {
            Shape::Floating => {
                window.set_margin(Edge::Top, cfg.margin_top);
                window.set_margin(Edge::Left, cfg.margin_side);
                window.set_margin(Edge::Right, cfg.margin_side);
                window.set_exclusive_zone(cfg.height + cfg.margin_top);
            }
            Shape::Edge => {
                window.set_exclusive_zone(cfg.height);
            }
        }
        window.present();

        // This monitor's volume OSD lives in its own overlay-layer surface so it
        // can float mid-screen independent of the bar strip.
        let osd = osd::Osd::build(app, monitor, pal, cfg.osd_timeout_ms);

        let surface = Rc::new(Surface {
            window,
            output,
            left,
            center: center.clone(),
            right,
            np_text_w: Cell::new(-1),
            np_playing: Cell::new(false),
            compact,
            detached: Cell::new(false),
            bar_box,
            ws_box,
            ws_assigned,
            hide_empty,
            numerals,
            app_label,
            submap_box,
            submap_label,
            np_box,
            np_text,
            np_title,
            np_artist,
            np_art_pic,
            np_art_url: RefCell::new(String::new()),
            cpu_spark,
            cpu_val,
            cpu_sub,
            mem_spark,
            mem_val,
            mem_sub,
            gpu_spark,
            gpu_val,
            gpu_sub,
            gpu_metric,
            net_ctl,
            net_glyph,
            net_val,
            net_sub,
            bt_ctl,
            bt_glyph,
            bt_val,
            rec_box,
            caffeine_box,
            night_box,
            vol_glyph,
            vol_val,
            vol_ctl,
            bri_ctl,
            bri_val,
            bat_ctl,
            bat_glyph,
            bat_val,
            bat_sub,
            bell_btn,
            bell_glyph,
            bell_count,
            clock_label,
            clock_time,
            gamemode_box,
            tray_box,
            camera_box,
            mic_box,
            ai_box,
            ai_val,
            ai_sub,
            weather_box,
            weather_val,
            weather_sub,
            priv_chip_rev,
            priv_box_rev,
            sys_chip_rev,
            sys_box_rev,
            priv_chip,
            priv_chip_val,
            grouped_priv,
            priv_live: Cell::new((false, false, false)),
            sys_chip,
            sys_chip_val,
            grouped_sys,
            sys_parts: RefCell::new(Default::default()),
            osd,
            mirror,
            custom: custom_cells,
            // Every popover on this bar, so `wire_layout_freeze` can hold the
            // layout still for whichever one the user opens.
            popovers: vec![
                tezca_pop,
                ai_pop,
                weather_pop,
                bat_pop,
                cpu_pop,
                mem_pop,
                gpu_pop,
                net_pop,
                bt_pop,
                mix_pop,
                cal_pop,
            ],
            open_popovers: Cell::new(0),
            pending_show: RefCell::new(Vec::new()),
            tray_dirty: Cell::new(false),
        });

        // Chip ↔ members. Wired after construction because both directions need
        // the surface to flip the flag on.
        for (chip, collapse, grouped) in
            [(&surface.priv_chip, &priv_collapse, true), (&surface.sys_chip, &sys_collapse, false)]
        {
            let click = gtk4::GestureClick::new();
            let sw = Rc::downgrade(&surface);
            click.connect_released(move |_, _, _, _| {
                if let Some(s) = sw.upgrade() {
                    s.set_grouped(grouped, false);
                }
            });
            chip.add_controller(click);

            let sw = Rc::downgrade(&surface);
            collapse.connect_clicked(move |_| {
                if let Some(s) = sw.upgrade() {
                    s.set_grouped(grouped, true);
                }
            });
        }
        surface.apply_clusters();
        surface
    }

    // ── updates ─────────────────────────────────────────────────────────

    /// Collapse or expand one cluster.
    fn set_grouped(&self, privacy: bool, grouped: bool) {
        if privacy {
            self.grouped_priv.set(grouped);
        } else {
            self.grouped_sys.set(grouped);
        }
        self.apply_clusters();
    }

    /// Show whichever of chip/members each cluster is currently in.
    ///
    /// The privacy chip additionally hides itself when nothing is capturing:
    /// a chip reading "0 capturing" is a permanent reminder of a thing that is
    /// not happening, and the three modules it stands for are all auto-hiding
    /// for exactly that reason.
    fn apply_clusters(&self) {
        let (cam, mic, rec) = self.priv_live.get();
        let live = u32::from(cam) + u32::from(mic) + u32::from(rec);
        let pg = self.grouped_priv.get();
        self.priv_chip_val.set_text(&format!("{live} capturing"));
        // `set_reveal_child`, not `set_visible`: hiding outright would skip the
        // transition the revealer exists for.
        self.priv_chip_rev.set_reveal_child(pg && live > 0);
        self.priv_box_rev.set_reveal_child(!pg);

        let sg = self.grouped_sys.get();
        let p = self.sys_parts.borrow();
        let summary: Vec<&str> = [p.0.as_str(), p.1.as_str(), p.2.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        self.sys_chip_val.set_text(&summary.join(" · "));
        drop(p);
        self.sys_chip_rev.set_reveal_child(sg);
        self.sys_box_rev.set_reveal_child(!sg);
    }

    fn set_workspaces(&self, snap: &hypr::Snapshot) {
        while let Some(c) = self.ws_box.first_child() {
            self.ws_box.remove(&c);
        }
        let active = hypr::active_ws_for(&snap.monitors, &self.output);
        let occupied = |id: i32| snap.workspaces.iter().any(|w| w.id == id && w.windows > 0);

        // A configured set enumerates this output's pills (in order); otherwise
        // mirror whatever Hyprland has placed on this monitor.
        let mut ids: Vec<i32> = match &self.ws_assigned {
            Some(list) => list.clone(),
            None => {
                let mut mine: Vec<i32> = snap
                    .workspaces
                    .iter()
                    .filter(|w| w.id > 0 && (w.monitor == self.output || self.output.is_empty()))
                    .map(|w| w.id)
                    .collect();
                mine.sort_unstable();
                mine
            }
        };
        // Optionally drop empty pills, keeping the focused one so the cluster
        // always shows where you are.
        if self.hide_empty {
            ids.retain(|id| occupied(*id) || *id == active);
        }
        let mayan = self.numerals == Numerals::Mayan;
        if ids.is_empty() {
            // Never show an empty cluster.
            self.ws_box.append(&ws_button(
                active,
                &ws_label(active, self.numerals),
                true,
                false,
                mayan,
            ));
            return;
        }
        for id in ids {
            let label = ws_label(id, self.numerals);
            self.ws_box.append(&ws_button(id, &label, id == active, occupied(id), mayan));
        }
    }

    fn set_app(&self, class: &str) {
        if self.compact {
            return;
        }
        if class.is_empty() {
            self.app_label.set_text("Tezca");
            self.app_label.add_css_class("idle");
        } else {
            self.app_label.set_text(&pretty(class));
            self.app_label.remove_css_class("idle");
        }
    }

    fn set_submap(&self, name: &str) {
        if name.is_empty() {
            self.submap_box.set_visible(false);
            self.bar_box.remove_css_class("submap");
        } else {
            self.submap_label.set_text(&format!("\u{25C6} {}", name.to_uppercase()));
            self.submap_box.set_visible(true);
            self.bar_box.add_css_class("submap");
        }
    }

    fn set_audio(&self, a: &sysinfo::Audio) {
        if a.muted {
            self.vol_glyph.set(Icon::VolumeMuted);
            set_pct(&self.vol_val, None);
            self.vol_ctl.add_css_class("muted");
        } else {
            let idx = match a.volume {
                0..=32 => 0,
                33..=66 => 1,
                _ => 2,
            };
            self.vol_glyph.set([Icon::VolumeLow, Icon::VolumeMid, Icon::VolumeHigh][idx]);
            set_pct(&self.vol_val, Some(a.volume));
            self.vol_ctl.remove_css_class("muted");
        }
    }

    /// The network control: which link, and how hard it is working.
    ///
    /// The name line is the SSID on Wi-Fi and the plain word on a wired link —
    /// signal strength moved to the sub-line's company in the popover, because
    /// "which network am I on" is asked far more often than "how many bars".
    fn set_net(&self, n: &Net, t: &sysinfo::Throughput) {
        self.net_ctl.remove_css_class("disconnected");
        match n {
            Net::Wifi { ssid, .. } => {
                self.net_glyph.set(Icon::Wifi);
                self.net_val.set_text(if ssid.is_empty() { "Wi-Fi" } else { ssid });
                set_sub(&self.net_sub, &rate_text(t));
            }
            Net::Ethernet { .. } => {
                self.net_glyph.set(Icon::Ethernet);
                self.net_val.set_text("Wired");
                set_sub(&self.net_sub, &rate_text(t));
            }
            Net::Disconnected => {
                self.net_glyph.set(Icon::Disconnected);
                self.net_val.set_text("Offline");
                set_sub(&self.net_sub, "");
                self.net_ctl.add_css_class("disconnected");
            }
        }
    }

    fn set_battery(&self, b: &Option<sysinfo::Battery>) {
        match b {
            Some(b) => {
                self.bat_glyph.set(if b.charging { Icon::BatteryCharging } else { Icon::Battery });
                self.bat_val.set_text(&pct_text(b.percent));
                // Time is the number you actually plan around; the percentage
                // only stands in for it. Absent while resting on AC.
                let left = b.secs_remaining.map(sysinfo::duration_short).unwrap_or_default();
                set_sub(&self.bat_sub, &left);
                self.show(&self.bat_ctl, true);
            }
            None => self.show(&self.bat_ctl, false),
        }
    }

    fn set_brightness(&self, b: Option<u32>) {
        match b {
            Some(p) => {
                self.bri_val.set_text(&pct_text(p));
                self.show(&self.bri_ctl, true);
            }
            None => self.show(&self.bri_ctl, false),
        }
    }

    fn set_bell(&self, s: &notify::BellState) {
        if s.unread > 0 {
            self.bell_glyph.set(Icon::BellUnread);
            self.bell_btn.add_css_class("unread");
            // Past two digits the badge would push the whole right cluster
            // along; at that point the exact number has stopped being useful.
            self.bell_count.set_text(&if s.unread > 99 {
                "99+".to_string()
            } else {
                s.unread.to_string()
            });
            self.bell_count.set_visible(true);
        } else {
            self.bell_glyph.set(Icon::Bell);
            self.bell_btn.remove_css_class("unread");
            self.bell_count.set_visible(false);
        }
    }

    /// Update the AI module: peak utilisation, visibility, and the threshold
    /// colour class. A provider that reports only local token counts has no
    /// percentage, so the module shows the glyph alone rather than inventing a
    /// number — the popover still carries the detail.
    fn set_ai(&self, pct: Option<f64>, resets_at: Option<i64>, empty: bool, warn: f64, crit: f64) {
        self.show(&self.ai_box, !empty);
        if empty {
            return;
        }
        match pct {
            Some(p) => set_pct(&self.ai_val, Some(p.round().max(0.0) as u32)),
            None => set_pct(&self.ai_val, None),
        }
        self.show(&self.ai_val, pct.is_some());
        // Only meaningful alongside a percentage — a reset time with no usage
        // figure beside it is a countdown to nothing.
        let resets = match (pct, resets_at) {
            (Some(_), Some(t)) => ai::until(t),
            _ => String::new(),
        };
        set_sub(&self.ai_sub, &resets);
        let p = pct.unwrap_or(0.0);
        self.ai_box.remove_css_class("warn");
        self.ai_box.remove_css_class("crit");
        if p >= crit {
            self.ai_box.add_css_class("crit");
        } else if p >= warn {
            self.ai_box.add_css_class("warn");
        }
    }

    /// The weather readout: temperature, with today's range beside it.
    fn set_weather(&self, s: &weather::Snapshot) {
        self.show(&self.weather_box, !s.is_empty());
        if s.is_empty() {
            return;
        }
        self.weather_val.set_text(&s.temp_text());
        set_sub(&self.weather_sub, &s.range_text());
        self.weather_box.set_tooltip_text(Some(&s.tooltip()));
    }

    fn set_custom(&self, out: &custom::Output) {
        if let Some(cell) = self.custom.get(&out.name) {
            cell.set(out);
        }
    }

    fn set_gamemode(&self, on: bool) {
        if on {
            self.show(&self.gamemode_box, true);
            self.gamemode_box.add_css_class("active");
        } else {
            self.show(&self.gamemode_box, false);
            self.gamemode_box.remove_css_class("active");
        }
    }

    /// Show/hide the camera privacy indicator and keep its tooltip current.
    fn set_camera(&self, c: &camera::CameraUse) {
        self.show(&self.camera_box, c.active);
        self.camera_box.set_tooltip_text(Some(&c.tooltip()));
        let (_, mic, rec) = self.priv_live.get();
        self.priv_live.set((c.active, mic, rec));
        self.apply_clusters();
    }

    /// Show/hide the microphone privacy indicator and keep its tooltip current.
    fn set_mic(&self, m: &mic::MicUse) {
        self.show(&self.mic_box, m.active);
        self.mic_box.set_tooltip_text(Some(&m.tooltip()));
        let (cam, _, rec) = self.priv_live.get();
        self.priv_live.set((cam, m.active, rec));
        self.apply_clusters();
    }

    /// True while at least one popover anchored to this bar is open.
    fn layout_frozen(&self) -> bool {
        self.open_popovers.get() > 0
    }

    /// Size the centre region to the room the side clusters leave, so that GTK's
    /// own centring is never the thing that has to give.
    ///
    /// `GtkCenterBox` centres its middle child only while
    /// `end ≤ (width − centre) / 2`. Past that it stops centring and pins the
    /// centre against the wider side — silently, and by however much it is over.
    /// The bar was over on the 2560px monitor, and because the pill used to be
    /// sized by the track title, *whether the bar centred depended on the name of
    /// the song*: measured, a 428px pill sat 74px left of centre and a 223px one
    /// sat dead centre.
    ///
    /// So rather than fight the clamp, keep the centre inside the budget that
    /// makes the clamp unreachable:
    ///
    /// ```text
    ///   centre_half  ≤  width/2 − max(left, right) − gap
    /// ```
    ///
    /// Two consequences worth naming. The pill's width is now a function of the
    /// *layout* — it changes when a module comes or goes, not when a song does —
    /// which is the property that was actually being asked for. And when the
    /// centre region holds more than the pill (weather to its left, say), the
    /// margins below pad the shorter side so the region's midpoint is the pill's
    /// midpoint: centring the region then centres the pill rather than the group
    /// the pill happens to sit in.
    fn retarget_center(&self) {
        let w = self.bar_box.width();
        if w <= 0 {
            return; // not allocated yet; the next tick catches it
        }
        // Resizing under an open popover would drag it off its anchor, which is
        // the whole point of the freeze — see `show`.
        if self.layout_frozen() {
            return;
        }
        let nat = |w: &GtkBox| w.measure(Orientation::Horizontal, -1).1;
        let side = nat(&self.left).max(nat(&self.right));

        // Anything else sharing the centre region, split by which side of the
        // pill it sits on. Hidden children measure 0 and are skipped anyway.
        let pill = self.np_box.clone().upcast::<gtk4::Widget>();
        let (mut lead, mut trail, mut past) = (0, 0, false);
        let mut child = self.center.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            if c == pill {
                past = true;
                continue;
            }
            if !c.is_visible() {
                continue;
            }
            let n = c.measure(Orientation::Horizontal, -1).1;
            if past {
                trail += n;
            } else {
                lead += n;
            }
        }
        // The pill is centred by padding the region out to symmetry around it.
        let sat = lead.max(trail);
        let budget = 2 * (w / 2 - side - sat - NP_GAP);
        let mut text = (budget - NP_CHROME).clamp(0, NP_TEXT_MAX);
        if text < NP_TEXT_MIN {
            // More ellipsis than title. Better hidden than shown as a stub.
            text = 0;
        }
        if self.np_text_w.replace(text) != text {
            self.np_text.set_size_request(text, -1);
            self.apply_np_visibility();
        }
        // Only pad while the pill is actually there to be centred. With nothing
        // playing there is no anchor, and holding its place would strand the
        // weather chip half a pill-width off centre beside a gap — which reads as
        // a bug, where a chip that recentres when the music stops reads as the
        // module appearing and disappearing, which is what happened.
        let (pad_start, pad_end) = if text > 0 && self.np_playing.get() {
            ((trail - lead).max(0), (lead - trail).max(0))
        } else {
            (0, 0)
        };
        self.center.set_margin_start(pad_start);
        self.center.set_margin_end(pad_end);
    }

    /// The pill shows when there is something playing *and* room to say so.
    fn apply_np_visibility(&self) {
        self.show(&self.np_box, self.np_playing.get() && self.np_text_w.get() > 0);
    }

    /// Show or hide a module — unless doing so would move an open popover.
    ///
    /// A module appearing or disappearing re-flows the whole cluster, and GTK
    /// keeps a popover glued to the widget it is anchored to, so the popover
    /// gets dragged sideways with it. Held-back changes are applied by
    /// [`Surface::thaw`] when the last popover closes; keeping only the latest
    /// value per widget means a module that flickered while you were reading
    /// settles on whatever it ended up as, not on a queue of stale toggles.
    fn show(&self, w: &impl IsA<gtk4::Widget>, visible: bool) {
        let w = w.as_ref();
        if w.is_visible() == visible {
            return; // already right — nothing would move
        }
        if !self.layout_frozen() {
            w.set_visible(visible);
            return;
        }
        let mut pending = self.pending_show.borrow_mut();
        pending.retain(|(p, _)| p != w);
        pending.push((w.clone(), visible));
    }

    fn freeze(&self) {
        self.open_popovers.set(self.open_popovers.get() + 1);
    }

    /// Drop one freeze. When it was the last, apply everything held back and
    /// report whether a tray rebuild was among the things skipped.
    fn thaw(&self) -> bool {
        let n = self.open_popovers.get().saturating_sub(1);
        self.open_popovers.set(n);
        if n > 0 {
            return false;
        }
        for (w, v) in self.pending_show.borrow_mut().drain(..) {
            w.set_visible(v);
        }
        self.tray_dirty.replace(false)
    }

    /// Add or remove a CSS class from a widget in one call.
    fn toggle_class(w: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
        if on {
            w.add_css_class(class);
        } else {
            w.remove_css_class(class);
        }
    }

    /// Keep-awake: shown only while the inhibitor is held.
    fn set_caffeine(&self, on: bool) {
        self.show(&self.caffeine_box, on);
        self.caffeine_box.set_tooltip_text(Some("Keeping the session awake — click to release"));
    }

    /// Recording: the third privacy dot. Says so when it is not ours, because
    /// clicking cannot stop a recorder this bar did not start.
    fn set_recording(&self, r: &session::RecordState) {
        self.show(&self.rec_box, r.active);
        self.rec_box.set_tooltip_text(Some(if r.foreign {
            "Screen recording in progress (started outside Tezca)"
        } else {
            "Recording the screen — `tezca record stop` to save"
        }));
        let (cam, mic, _) = self.priv_live.get();
        self.priv_live.set((cam, mic, r.active));
        self.apply_clusters();
    }

    /// Night light: shown only while the filter is actually on.
    fn set_night(&self, n: &session::NightState) {
        self.show(&self.night_box, n.active);
        self.night_box
            .set_tooltip_text(Some(&format!("Night light on at {} K — click to turn off", n.temp)));
    }

    /// Bluetooth: hidden with no adapter, dim when off, accent when connected.
    fn set_bluetooth(&self, b: &bluetooth::BtState) {
        self.show(&self.bt_ctl, b.present);
        if !b.present {
            return;
        }
        let connected = !b.connected.is_empty();
        self.bt_glyph.set(match (b.powered, connected) {
            (false, _) => Icon::BluetoothOff,
            (true, false) => Icon::Bluetooth,
            (true, true) => Icon::BluetoothConnected,
        });
        // The battery of a connected headset is the one number worth the space —
        // named, because with two devices paired "82%" alone leaves you guessing
        // which one is about to die.
        let badge = match (b.badge_name(), b.badge_pct()) {
            (Some(n), Some(p)) => format!("{n} {p}%"),
            (None, Some(p)) => format!("{p}%"),
            _ => String::new(),
        };
        self.bt_val.set_text(&badge);
        self.show(&self.bt_val, !badge.is_empty());
        Self::toggle_class(&self.bt_ctl, "active", connected);
        Self::toggle_class(&self.bt_ctl, "off", !b.powered);
        self.bt_ctl.set_has_tooltip(true);
        self.bt_ctl.set_tooltip_text(Some(&b.tooltip()));
    }

    fn set_nowplaying(&self, np: Option<&nowplaying::NowPlaying>) {
        match np {
            Some(t) => {
                self.np_title.set_text(&t.title);
                // The sub-line carries the elapsed time beside the artist, so
                // the pill answers "how far in am I" without opening anything.
                self.np_artist.set_text(&t.subtitle());
                if *self.np_art_url.borrow() != t.art_url {
                    self.np_art_url.replace(t.art_url.clone());
                    match nowplaying::art_texture(&t.art_url) {
                        Some(tex) => {
                            self.np_art_pic.set_paintable(Some(&tex));
                            self.np_art_pic.set_visible(true);
                        }
                        // Back to the gradient — a stale cover over a new track
                        // is worse than none.
                        None => {
                            self.np_art_pic.set_paintable(gtk4::gdk::Paintable::NONE);
                            self.np_art_pic.set_visible(false);
                        }
                    }
                }
                self.np_playing.set(true);
            }
            None => self.np_playing.set(false),
        }
        self.apply_np_visibility();
    }

    /// Repaint the cairo-drawn widgets after a palette reload.
    fn repaint_drawn(&self) {
        self.mirror.queue_draw();
        self.cpu_spark.area.queue_draw();
        self.mem_spark.area.queue_draw();
        self.gpu_spark.area.queue_draw();
    }
}

// ===========================================================================
// small widget helpers
// ===========================================================================

const G_CPU_LABEL: &str = "CPU";
const G_MEM_LABEL: &str = "MEM";
const G_GPU_LABEL: &str = "GPU";
const G_SYS_LABEL: &str = "SYS";
/// How long a cluster takes to fold or unfold, ms. The design's own module
/// transition is `.2s`; matching it keeps the expand in the same tempo as every
/// hover on the bar.
const CLUSTER_MS: u32 = 200;
/// How many charge samples the battery trace keeps. At the 2-second controls
/// tick that is about two minutes — enough to see a trend start, and small
/// enough that the strip stays legible at popover width.
const BATTERY_POINTS: usize = 60;
/// How many throughput samples the network chart keeps — the design's 24 bars.
/// At the 2-second controls tick that is the last ~48 seconds, which is the
/// window in which "is something downloading right now" is a live question.
const NET_POINTS: usize = 24;

/// A collapsed-cluster chip: an optional fixed label, the live summary, and a
/// chevron saying it opens. Returns the chip and the label to fill each tick.
///
/// `gap` is per-chip rather than shared: the SYS chip's summary is a run of
/// numbers separated by interpuncts and needs more air between them than the
/// privacy chip's dot-and-phrase does.
fn cluster_chip(pal: &SharedPalette, label: &str, class: &str, gap: i32) -> (GtkBox, Label) {
    let b = GtkBox::new(Orientation::Horizontal, gap);
    b.add_css_class("cluster-chip");
    b.add_css_class(class);
    b.add_css_class("clickable");
    b.set_valign(Align::Center);
    if !label.is_empty() {
        let l = Label::new(Some(label));
        l.add_css_class("metric-label");
        b.append(&l);
    }
    let val = Label::new(None);
    val.add_css_class("cluster-val");
    b.append(&val);
    let chev = icon::icon(pal, Icon::ChevronDown);
    chev.area.add_css_class("cluster-chev");
    b.append(&chev.area);
    b.set_visible(false);
    (b, val)
}

/// The control that folds an expanded run back into its chip.
fn collapse_button(pal: &SharedPalette) -> Button {
    let b = Button::new();
    b.add_css_class("cluster-collapse");
    b.set_child(Some(&icon::icon(pal, Icon::Collapse).area));
    b.set_valign(Align::Center);
    b.set_tooltip_text(Some("Group these"));
    b
}

/// A 1×18 hairline separator.
fn sep() -> GtkBox {
    let s = GtkBox::new(Orientation::Horizontal, 0);
    s.add_css_class("sep");
    s.set_size_request(1, 16);
    s.set_valign(Align::Center);
    s
}

/// A community/user exec module's widget: an optional static glyph plus a value
/// label the poll thread fills. Hidden until it first prints something; a
/// script-supplied `class` is swapped in on each update so CSS/themes can react.
struct CustomCell {
    container: GtkBox,
    value: Label,
    /// The class(es) the last output added, to remove before the next.
    last_class: RefCell<Option<String>>,
}

impl CustomCell {
    fn build(m: &custom::CustomModule) -> CustomCell {
        let container = GtkBox::new(Orientation::Horizontal, 6);
        container.add_css_class("custom");
        container.set_valign(Align::Center);
        if let Some(icon) = &m.icon {
            let g = Label::new(Some(icon));
            g.add_css_class("glyph");
            container.append(&g);
        }
        let value = Label::new(None);
        value.add_css_class("custom-val");
        container.append(&value);
        container.set_visible(false);

        // Left click → on_click, right click → on_right_click. One all-buttons
        // gesture on the box (a plain GtkButton's built-in primary gesture
        // swallows secondary/middle — the tray hit the same snag).
        let (lc, rc) = (m.on_click.clone(), m.on_right_click.clone());
        if lc.is_some() || rc.is_some() {
            let click = gtk4::GestureClick::new();
            click.set_button(0); // 0 = listen for every button
            click.connect_released(move |g, _, _, _| match g.current_button() {
                gdk::BUTTON_SECONDARY => {
                    if let Some(c) = &rc {
                        custom::run_action(c);
                    }
                }
                _ => {
                    if let Some(c) = &lc {
                        custom::run_action(c);
                    }
                }
            });
            container.add_controller(click);
            container.add_css_class("clickable");
        }

        CustomCell { container, value, last_class: RefCell::new(None) }
    }

    fn set(&self, out: &custom::Output) {
        // Always clear the previous run's class(es) first, including on the error
        // and hidden paths — otherwise a module that goes from `class: "cold"` to
        // failing keeps the stale class forever.
        if let Some(prev) = self.last_class.borrow_mut().take() {
            for c in prev.split_whitespace() {
                self.container.remove_css_class(c);
            }
        }

        // A failed poll stays *visible*. Hiding it (which is what an empty text used
        // to do) is indistinguishable from a module you never configured, so a typo
        // in a manifest looked like nothing at all. Show a marker and put the reason
        // in the tooltip.
        if let Some(err) = &out.error {
            self.container.set_visible(true);
            self.value.set_text(if out.text.is_empty() { "!" } else { &out.text });
            self.container.set_tooltip_text(Some(&match &out.tooltip {
                Some(t) => format!("{t}\n{err}"),
                None => err.clone(),
            }));
            self.container.add_css_class("error");
            *self.last_class.borrow_mut() = Some("error".to_string());
            return;
        }

        let show = !out.text.is_empty();
        self.container.set_visible(show);
        if !show {
            return;
        }
        self.value.set_text(&out.text);
        self.container.set_tooltip_text(out.tooltip.as_deref());
        if let Some(cls) = &out.class {
            for c in cls.split_whitespace() {
                self.container.add_css_class(c);
            }
            *self.last_class.borrow_mut() = Some(cls.clone());
        }
    }
}

/// A run of modules the bar can collapse behind one chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cluster {
    /// Camera / microphone / recording — "3 capturing".
    Privacy,
    /// CPU / MEM / GPU — "SYS 31% · 18.4G · 44%".
    System,
}

/// Which cluster a slot belongs to, if any.
fn cluster_of(slot: &Slot) -> Option<Cluster> {
    match slot {
        Slot::Mod(Mod::Camera | Mod::Microphone | Mod::Recording) => Some(Cluster::Privacy),
        Slot::Mod(Mod::Cpu | Mod::Mem | Mod::Gpu) => Some(Cluster::System),
        _ => None,
    }
}

/// The chip + members container for each collapsible run.
struct ClusterSlots<'a> {
    privacy: (&'a gtk4::Revealer, &'a gtk4::Revealer),
    system: (&'a gtk4::Revealer, &'a gtk4::Revealer),
}

impl ClusterSlots<'_> {
    fn get(&self, c: Cluster) -> (&gtk4::Revealer, &gtk4::Revealer) {
        match c {
            Cluster::Privacy => self.privacy,
            Cluster::System => self.system,
        }
    }
}

/// Append the configured modules to a region, in order.
///
/// `resolve` turns a non-separator module id into the widget built for it (or
/// `None` for `Sep`). We honour a couple of layout niceties so the built-in
/// defaults reproduce the old hardcoded look *and* hand-written layouts stay
/// tidy:
///   * on a compact monitor the `appname` module is dropped (as the old code
///     did — the ultrawide keeps it),
///   * separators are collapsed: leading, trailing, and runs of adjacent
///     `Sep`s render as at most one hairline (so a dropped module can't leave a
///     doubled or dangling divider),
///   * a widget already placed is skipped (a GTK widget can have only one
///     parent, so a duplicate id would otherwise fail on reparent). `placed` is
///     shared across all three regions by the caller, so this catches a module
///     listed twice in one region *and* the same module listed in two.
///
/// `clusters`, where given, additionally folds the collapsible runs into their
/// chip + members pair — see [`Cluster`].
fn place_region(
    container: &GtkBox,
    slots: &[Slot],
    compact: bool,
    drop_tier3: bool,
    clusters: Option<&ClusterSlots>,
    resolve: &dyn Fn(&Slot) -> Option<gtk4::Widget>,
    placed: &mut HashSet<Slot>,
) {
    // A cluster is a *contiguous run*. The first run of each kind gets the chip
    // and the members container; anything from that cluster appearing again
    // later in the region renders on its own. Collapsing two separated runs
    // behind one chip would move modules across the layout the user wrote,
    // which is a bigger surprise than the second run staying expanded.
    let mut open: Option<Cluster> = None;
    let mut used: Vec<Cluster> = Vec::new();

    for slot in plan_region(slots, compact, drop_tier3, placed) {
        if slot.is_sep() {
            open = None;
            // A fresh hairline per occurrence.
            container.append(&sep());
            continue;
        }
        // A custom slot with no matching manifest resolves to None → nothing
        // placed (the module just isn't installed), which is fine.
        let Some(w) = resolve(&slot) else { continue };

        let want = clusters.and_then(|_| cluster_of(&slot)).filter(|c| {
            // Either we are already inside this run, or it has not had one yet.
            open == Some(*c) || !used.contains(c)
        });
        match (want, clusters) {
            (Some(c), Some(cs)) => {
                let (chip, members) = cs.get(c);
                if open != Some(c) {
                    container.append(chip);
                    container.append(members);
                    used.push(c);
                    open = Some(c);
                }
                // Into the box the revealer wraps, not the revealer itself.
                if let Some(b) = members.child().and_downcast::<GtkBox>() {
                    b.append(&w);
                }
            }
            _ => {
                open = None;
                container.append(&w);
            }
        }
    }
}

/// Resolve a region's raw slot list into the ordered sequence to append (still
/// as `Slot`s, separators included but collapsed). Pure (no GTK) so the fiddly
/// rules are unit-testable:
///   * `appname` is dropped on a compact monitor,
///   * a non-separator slot already emitted is skipped (a GTK widget has one
///     parent, so a duplicate would panic on reparent),
///   * separators collapse — leading, trailing, and adjacent `Sep`s reduce to
///     at most one, so a dropped module never leaves a doubled or dangling one.
fn plan_region(
    slots: &[Slot],
    compact: bool,
    drop_tier3: bool,
    placed: &mut HashSet<Slot>,
) -> Vec<Slot> {
    let mut out: Vec<Slot> = Vec::new();
    let mut last_real = false; // suppress a leading separator
    let mut pending_sep = false; // only flushed when a real widget follows

    for slot in slots {
        if slot.is_sep() {
            if last_real {
                pending_sep = true;
            }
            continue;
        }
        if slot.is_appname() && compact {
            continue;
        }
        // The tiers strategy drops rather than hides: a module that is never
        // placed cannot be un-hidden by its own auto-show logic two seconds
        // later, and the separators around it collapse for free below.
        if drop_tier3 {
            if let Slot::Mod(m) = slot {
                if m.is_tier3() {
                    continue;
                }
            }
        }
        if !placed.insert(slot.clone()) {
            eprintln!(
                "tezca-bar: `{}` is listed more than once — it renders where it \
                 was reached first and is ignored here",
                slot.id()
            );
            continue;
        }
        if pending_sep {
            out.push(Slot::Mod(Mod::Sep));
            pending_sep = false;
        }
        out.push(slot.clone());
        last_real = true;
    }
    out
}

/// The now-playing pill's fixed furniture, in px: a 1px border and 9px of
/// padding on each side, the 26px cover, the 13px equaliser, and the two 10px
/// gaps its box puts between those three children. Everything except the text
/// column — so the pill measures exactly `NP_CHROME + text`, and sizing the text
/// sizes the pill.
const NP_CHROME: i32 = 2 * (1 + 9) + 26 + 13 + 2 * 10;
/// Widest the title/artist column is allowed to get, however much room there is.
/// Past this the pill stops reading as a label and starts reading as a third
/// cluster.
const NP_TEXT_MAX: i32 = 240;
/// Narrowest worth drawing. Below this a title is more ellipsis than title.
const NP_TEXT_MIN: i32 = 88;
/// Clearance kept between the centre region and each side cluster, so a centred
/// pill never looks welded to the module beside it.
const NP_GAP: i32 = 14;

/// Digits a percent readout is padded to — enough for the widest, `100`.
const PCT_DIGITS: usize = 3;

/// Render a percent so that every value is exactly the same width.
///
/// These readouts are the only labels on the bar whose *width* changes on a
/// tick: `9%` grows to `100%` and shrinks back. The right cluster is
/// right-aligned, so whenever one of them changes width every module to its left
/// slides sideways — constantly, since CPU, GPU and Wi-Fi signal all cross the
/// 10% and 100% boundaries on their own. That is what dragged an open popover
/// off the module it was anchored to.
///
/// The padding is U+2007 FIGURE SPACE, which is *defined* to be exactly as wide
/// as a digit — so `␣10%` and `100%` occupy identical space in a proportional
/// font, with no font change and no guessed pixel width. `GtkLabel::width_chars`
/// is the obvious alternative and does not work here: it reserves N *average*
/// character widths, and in Inter a digit is wider than the average character,
/// so `100%` still overflowed a four-character reservation and shifted the bar
/// by 4px. Measured, not assumed.
///
/// Paired with `font-feature-settings: "tnum"` in bar.css, which makes the
/// digits themselves equal-width; this handles the digit *count*, that handles
/// the digits.
fn pct_text(p: u32) -> String {
    let n = p.to_string();
    let pad = PCT_DIGITS.saturating_sub(n.chars().count());
    format!("{}{n}%", "\u{2007}".repeat(pad))
}

/// Set a percent readout, or clear it entirely.
///
/// Some are genuinely absent at times — no signal strength on Ethernet, no level
/// when the sink is muted — and holding a blank field open next to the glyph
/// would read as a rendering fault. Those are mode changes rather than per-tick
/// churn, so they cost one re-flow each, not one every two seconds.
fn set_pct(l: &Label, v: Option<u32>) {
    match v {
        Some(p) => l.set_text(&pct_text(p)),
        None => l.set_text(""),
    }
}

/// `LABEL  <spark>  val%` metric group.
/// A metric group: `CPU ~~~ 31% 62°`.
///
/// `sub` carries the second number the popover already computes — the temp
/// beside the load, the watts beside the utilisation. It starts hidden and
/// reveals itself only once a tick has something real to put in it, so a machine
/// with no temperature sensor shows no empty gap where one would be.
fn metric(label: &str, spark: &gtk4::DrawingArea, val: &Label, sub: &Label) -> GtkBox {
    let b = GtkBox::new(Orientation::Horizontal, 7);
    b.add_css_class("metric");
    b.set_valign(Align::Center);
    let l = Label::new(Some(label));
    l.add_css_class("metric-label");
    b.append(&l);
    b.append(spark);
    b.append(val);
    sub.add_css_class("metric-sub");
    sub.set_visible(false);
    b.append(sub);
    b
}

/// Set a secondary readout, hiding it when there is nothing to say.
fn set_sub(l: &Label, text: &str) {
    l.set_text(text);
    l.set_visible(!text.is_empty());
}

/// `↓12.4 ↑1.1 MB/s` — the throughput line under the network name.
///
/// Megabytes, not the megabits the meter samples in: MB/s is the unit a
/// download progress bar quotes, so it is the one that answers "is this as fast
/// as it should be". Blank while nothing is moving, so an idle link does not
/// park a row of zeroes on the bar.
/// Padded to a fixed width for the same reason [`pct_text`] is: this label sits
/// in the right-aligned cluster, so when `↓1.2` becomes `↓12.8` every module to
/// its left slides over. Measured on the 2560px monitor, that one digit moved the
/// cluster's edge by 48px — enough on its own to decide whether the centred
/// now-playing pill had room to exist.
fn rate_text(t: &sysinfo::Throughput) -> String {
    let (down, up) = (t.down_mbps / 8.0, t.up_mbps / 8.0);
    if down < 0.05 && up < 0.05 {
        return String::new();
    }
    format!("↓{} ↑{} MB/s", rate_num(down), rate_num(up))
}

/// One throughput figure, padded to `NN.N` with U+2007 FIGURE SPACE — the
/// character defined to be exactly one digit wide. Past 99.9 it simply grows: a
/// truncated number would be worse than a one-off re-flow.
fn rate_num(v: f64) -> String {
    let s = format!("{v:.1}");
    let int_digits = s.split('.').next().map(str::len).unwrap_or(1);
    format!("{}{s}", "\u{2007}".repeat(2usize.saturating_sub(int_digits)))
}

/// Split a formatted clock into its date and time halves.
///
/// The design draws them in different faces, so they have to be different
/// labels — but `clock_format` is one strftime string the user owns. The shipped
/// default (`%a %d %b   %H:%M`) puts a run of spaces between the two, and that
/// run is the only marker available that does not amount to parsing dates back
/// out of their own rendering. Without one, the whole string stays in the date's
/// face: one font is a smaller error than a split in the wrong place.
fn split_clock(text: &str) -> (&str, &str) {
    match text.rfind("  ") {
        Some(i) => (text[..i].trim_end(), text[i..].trim_start()),
        None => (text, ""),
    }
}

/// `71° 168W` — whichever of the two the driver actually reported.
fn gpu_sub_text(d: &sysinfo::GpuDetail) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = d.temp_c {
        parts.push(format!("{}°", t.round() as i64));
    }
    if let Some(w) = d.power_w {
        parts.push(format!("{}W", w.round() as i64));
    }
    parts.join(" ")
}

/// Parent `pop` to `widget` and pop it up on click, marking the group hoverable.
fn attach_detail(widget: &impl IsA<gtk4::Widget>, pop: gtk4::Popover) {
    widget.add_css_class("clickable");
    let click = gtk4::GestureClick::new();
    click.connect_released(move |_, _, _, _| pop.popup());
    widget.add_controller(click);
}

/// A `.control` button holding an icon + value; returns handles to both.
fn control_button(pal: &SharedPalette, kind: Icon) -> (Button, icon::IconArea, Label) {
    let b = Button::new();
    b.add_css_class("control");
    b.set_valign(Align::Center);
    let inner = GtkBox::new(Orientation::Horizontal, 6);
    let glyph = icon::icon(pal, kind);
    glyph.area.add_css_class("glyph");
    let val = Label::new(None);
    val.add_css_class("control-val");
    inner.append(&glyph.area);
    inner.append(&val);
    b.set_child(Some(&inner));
    (b, glyph, val)
}

/// A `.control` button with its value stacked over a smaller second line.
/// Returns `(button, glyph, name, sub)`.
fn control_button_stacked(
    pal: &SharedPalette,
    kind: Icon,
) -> (Button, icon::IconArea, Label, Label) {
    let b = Button::new();
    b.add_css_class("control");
    b.set_valign(Align::Center);
    let inner = GtkBox::new(Orientation::Horizontal, 7);
    let glyph = icon::icon(pal, kind);
    glyph.area.add_css_class("glyph");

    let col = GtkBox::new(Orientation::Vertical, 0);
    col.set_valign(Align::Center);
    let name = Label::new(None);
    name.add_css_class("control-name");
    name.set_xalign(0.0);
    let sub = Label::new(None);
    sub.add_css_class("control-sub");
    sub.set_xalign(0.0);
    sub.set_visible(false);
    col.append(&name);
    col.append(&sub);

    inner.append(&glyph.area);
    inner.append(&col);
    b.set_child(Some(&inner));
    (b, glyph, name, sub)
}

/// A workspace pill button showing `label`, switching to `id` on click.
/// `mayan` asks for bar-and-dot numerals, which are drawn rather than typed.
fn ws_button(id: i32, label: &str, active: bool, occupied: bool, mayan: bool) -> Button {
    // Mayan numerals are drawn, not typed — see `draw::mayan_numeral`. Anything
    // outside the bar-and-dot range falls back to the digit, so a workspace 24
    // still labels itself rather than rendering blank.
    let drawn = mayan && (1..=draw::MAYAN_MAX).contains(&id);
    let b = if drawn {
        let b = Button::new();
        b.set_child(Some(&draw::mayan_numeral(id)));
        b
    } else {
        Button::with_label(label)
    };
    b.add_css_class("ws");
    // Centred, not filled: `min-height: 26px` in CSS is only a floor, and a
    // pill left to fill grows to the bar's full inner height — a rounded square
    // rather than the design's 26px pill.
    b.set_valign(Align::Center);
    if drawn {
        b.add_css_class("mayan");
    }
    if active {
        b.add_css_class("active");
    } else if occupied {
        b.add_css_class("occupied");
    }
    b.connect_clicked(move |_| hypr::goto_workspace(id));
    b
}

/// Plan the window moves that pack a monitor's ordered workspace `set` — the
/// occupied workspaces slide down to the lowest slots, order preserved — while
/// leaving `visible` (the monitor's shown workspace) fixed and never moving
/// content across it. Returns `(from, to)` pairs; empty when already compact.
fn plan_compaction(set: &[i32], visible: i32, occupied: impl Fn(i32) -> bool) -> Vec<(i32, i32)> {
    let mut moves = Vec::new();
    // Content stays on its side of the visible workspace, so pack each side
    // independently. When `visible` isn't in this set, the whole set is one part.
    let parts: Vec<&[i32]> = match set.iter().position(|&w| w == visible) {
        Some(i) => vec![&set[..i], &set[i + 1..]],
        None => vec![set],
    };
    for part in parts {
        let filled: Vec<i32> = part.iter().copied().filter(|&w| occupied(w)).collect();
        for (slot, &src) in filled.iter().enumerate() {
            if src != part[slot] {
                moves.push((src, part[slot]));
            }
        }
    }
    moves
}

/// A workspace's pill *text*.
///
/// Always the digit: in Mayan mode `ws_button` draws the bar-and-dot glyph
/// itself and ignores this, except above [`draw::MAYAN_MAX`] where the digit is
/// the fallback.
fn ws_label(id: i32, _numerals: Numerals) -> String {
    id.to_string()
}

// ---------------------------------------------------------------------------

/// `org.kde.dolphin` → `Dolphin`; `brave-browser` → `Brave`; `Code` → `Code`.
fn pretty(class: &str) -> String {
    let seg = class.rsplit('.').next().unwrap_or(class);
    let seg = seg.split(['-', '_']).next().unwrap_or(seg);
    let mut chars = seg.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => class.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn every_percent_readout_renders_to_the_same_width() {
        // The bug this exists for: the right cluster is right-aligned, so a
        // readout gaining a digit shoves every module left of it sideways — and
        // an open popover is anchored to one of those modules.
        let widths: Vec<usize> =
            [0, 5, 9, 10, 42, 99, 100].iter().map(|p| pct_text(*p).chars().count()).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}");

        // Padded with U+2007 FIGURE SPACE specifically — it is defined to be one
        // digit wide, which an ordinary space is not, so this is what makes the
        // widths equal on screen and not merely equal in character count.
        assert_eq!(pct_text(7), "\u{2007}\u{2007}7%");
        assert_eq!(pct_text(42), "\u{2007}42%");
        assert_eq!(pct_text(100), "100%");
        assert!(!pct_text(7).contains(' '), "an ASCII space is not digit-width");

        // Above the reserved field it simply grows rather than being truncated:
        // a wrong number would be worse than a one-off re-flow.
        assert_eq!(pct_text(1000), "1000%");
    }

    #[test]
    fn throughput_is_one_width_whatever_the_link_is_doing() {
        // The right cluster is right-aligned, so a label that widens by a digit
        // pushes every module left of it sideways. Same reasoning as `pct_text`,
        // same U+2007 padding — this one was simply missed.
        let w = |d: f64, u: f64| {
            rate_text(&sysinfo::Throughput { down_mbps: d * 8.0, up_mbps: u * 8.0 }).chars().count()
        };
        assert_eq!(w(1.2, 0.1), w(12.8, 0.1));
        assert_eq!(w(1.2, 0.1), w(99.9, 99.9));
        assert!(rate_num(1.2).starts_with('\u{2007}'), "one-digit values are padded");
        assert!(!rate_num(12.8).starts_with('\u{2007}'), "two-digit values are not");
        // Past the reserved field it grows rather than lying about the rate.
        assert_eq!(rate_num(123.4), "123.4");
    }

    fn m(x: Mod) -> Slot {
        Slot::Mod(x)
    }
    fn slots(xs: &[Mod]) -> Vec<Slot> {
        xs.iter().copied().map(Slot::Mod).collect()
    }
    /// One region in isolation. The real caller threads one `placed` set through
    /// all three so a module listed twice renders once; these cases are about
    /// what a single region does with its own list.
    fn plan(slots: &[Slot], compact: bool, drop_tier3: bool) -> Vec<Slot> {
        plan_region(slots, compact, drop_tier3, &mut HashSet::new())
    }

    #[test]
    fn default_left_full_reproduces_the_old_layout() {
        let plan = plan(&Config::default().layout_left, false, false);
        use Mod::*;
        assert_eq!(plan, slots(&[Mirror, Sep, Appname, Sep, Workspaces, Submap]));
    }

    #[test]
    fn compact_drops_appname_and_collapses_the_freed_separator() {
        // mirror, sep, appname, sep, workspaces, submap  →  mirror | workspaces submap
        let plan = plan(&Config::default().layout_left, true, false);
        use Mod::*;
        assert_eq!(plan, slots(&[Mirror, Sep, Workspaces, Submap]));
    }

    #[test]
    fn default_right_plan_matches_the_old_hardcoded_order() {
        // The default right layout has no dups/edge seps, so the plan is identical.
        let right = Config::default().layout_right;
        assert_eq!(plan(&right, false, false), right);
    }

    #[test]
    fn leading_trailing_and_doubled_separators_collapse() {
        use Mod::*;
        let plan = plan(&slots(&[Sep, Sep, Cpu, Sep, Sep, Mem, Sep, Sep]), false, false);
        assert_eq!(plan, slots(&[Cpu, Sep, Mem]));
    }

    #[test]
    fn duplicate_modules_are_dropped_and_their_orphaned_separator_too() {
        use Mod::*;
        // cpu, sep, cpu → the second cpu is a dup; the sep before it is then
        // trailing and disappears, leaving a single cpu.
        let plan = plan(&slots(&[Cpu, Sep, Cpu]), false, false);
        assert_eq!(plan, slots(&[Cpu]));
    }

    #[test]
    fn custom_slot_places_and_dedupes_like_a_builtin() {
        use Mod::*;
        let weather = Slot::Custom("weather".into());
        let input = vec![m(Cpu), m(Sep), weather.clone(), m(Sep), weather.clone()];
        // second weather is a dup; its leading sep becomes trailing and drops.
        assert_eq!(plan(&input, false, false), vec![m(Cpu), m(Sep), weather]);
    }

    #[test]
    fn tiers_drops_third_tier_modules_and_tidies_their_separators() {
        use Mod::*;
        // gpu and tray are tier-3; removing them must not leave the separators
        // that framed them dangling.
        let input = slots(&[Cpu, Sep, Gpu, Tray, Sep, Clock]);
        assert_eq!(plan(&input, false, true), slots(&[Cpu, Sep, Clock]));
        // The same layout keeps everything when the strategy is off.
        assert_eq!(plan(&input, false, false), input);
    }

    #[test]
    fn tiers_never_drops_a_custom_module() {
        // Tiering is a judgement about the built-ins we ship. A module someone
        // installed deliberately is not ours to rank.
        let weather = Slot::Custom("weather".into());
        let input = vec![m(Mod::Gpu), weather.clone()];
        assert_eq!(plan(&input, false, true), vec![weather]);
    }

    #[test]
    fn clusters_cover_the_privacy_and_metric_runs_only() {
        use Mod::*;
        for id in [Camera, Microphone, Recording] {
            assert_eq!(cluster_of(&m(id)), Some(Cluster::Privacy), "{id:?}");
        }
        for id in [Cpu, Mem, Gpu] {
            assert_eq!(cluster_of(&m(id)), Some(Cluster::System), "{id:?}");
        }
        for id in [Clock, Battery, Tray, Network] {
            assert_eq!(cluster_of(&m(id)), None, "{id:?}");
        }
        assert_eq!(cluster_of(&Slot::Custom("weather".into())), None);
    }

    #[test]
    fn the_clock_splits_on_the_shipped_format_and_leaves_others_whole() {
        // The shipped default separates date from time with a run of spaces.
        assert_eq!(split_clock("Tue 04 Aug   10:08"), ("Tue 04 Aug", "10:08"));
        // A format with no such run keeps one face rather than guessing.
        assert_eq!(split_clock("Tue 04 Aug 10:08"), ("Tue 04 Aug 10:08", ""));
        // The *last* run wins, so a padded date does not steal the split.
        assert_eq!(split_clock("Tue  04 Aug   10:08"), ("Tue  04 Aug", "10:08"));
        assert_eq!(split_clock(""), ("", ""));
    }

    #[test]
    fn every_tier3_module_is_also_ambient_except_the_gpu() {
        // The two lists are deliberately near-identical; this pins the one
        // difference so a later edit to either has to be a considered one.
        use Mod::*;
        for id in [Caffeine, NightLight, Tray, Brightness] {
            assert!(id.is_tier3() && id.is_ambient(), "{id:?}");
        }
        assert!(Gpu.is_tier3() && !Gpu.is_ambient());
        assert!(Bluetooth.is_ambient() && !Bluetooth.is_tier3());
    }
}
