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

use crate::config::{Config, Numerals, Shape};
use crate::draw::{self, SharedPalette, Sparkline};
use crate::sysinfo::{self, CpuMeter, Net, NetMeter, Throughput};
use crate::theme::{CssStack, Palette};
use crate::{hypr, nowplaying, notify, popovers, tray};
use gtk4::gdk;
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, CenterBox, Image, Label, Orientation, Overlay, Window};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

// Nerd Font glyphs — the exact codepoints from config/waybar/config.jsonc, plus
// the redesign additions (brightness/battery/play-pause). See README §Assets.
const G_WIFI: &str = "\u{F05A9}";
const G_ETH: &str = "\u{F0200}";
const G_DISC: &str = "\u{F092D}";
const G_VOL: [&str; 3] = ["\u{F057F}", "\u{F0580}", "\u{F057E}"]; // low / mid / high
const G_MUTED: &str = "\u{F075F}";
const G_NOTIF: &str = "\u{F009A}";
const G_NOTIF_ON: &str = "\u{F0116}";
const G_POWER: &str = "\u{F0425}";
const G_GAME: &str = "\u{F02B4}";
const G_BRIGHT: &str = "\u{F00DF}";
const G_BATT: &str = "\u{F0079}";
const G_BATT_CHG: &str = "\u{F0084}";

// ===========================================================================
// Manager
// ===========================================================================

pub struct Bar {
    surfaces: Vec<Rc<Surface>>,
    cfg: Config,
    palette: SharedPalette,
    css: CssStack,
    cpu: RefCell<CpuMeter>,
    netmeter: RefCell<NetMeter>,
    throughput: Rc<RefCell<Throughput>>,
    tray_cmd: async_channel::Sender<tray::TrayCmd>,
    tray_items: RefCell<Vec<tray::TrayItemView>>,
    tray_menus: RefCell<HashMap<String, tray::MenuNode>>,
    /// The moves the last compaction pass dispatched — if the same plan recurs
    /// (a window that wouldn't move), we skip it rather than loop forever.
    last_compaction: RefCell<Vec<(i32, i32)>>,
}

impl Bar {
    pub fn build(
        app: &gtk4::Application,
        cfg: Config,
        palette: Palette,
        css: CssStack,
        tray_cmd: async_channel::Sender<tray::TrayCmd>,
    ) -> Rc<Bar> {
        let display = gdk::Display::default().expect("no display");
        let shared: SharedPalette = Rc::new(RefCell::new(palette));
        let throughput = Rc::new(RefCell::new(Throughput { down_mbps: 0.0, up_mbps: 0.0 }));

        let mut surfaces = Vec::new();
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            let Some(obj) = monitors.item(i) else { continue };
            let Ok(monitor) = obj.downcast::<gdk::Monitor>() else { continue };
            let s = Surface::build(app, &monitor, &cfg, &shared, throughput.clone());
            surfaces.push(s);
        }

        let bar = Rc::new(Bar {
            surfaces,
            cfg,
            palette: shared,
            css,
            cpu: RefCell::new(CpuMeter::default()),
            netmeter: RefCell::new(NetMeter::default()),
            throughput,
            tray_cmd,
            tray_items: RefCell::new(Vec::new()),
            tray_menus: RefCell::new(HashMap::new()),
            last_compaction: RefCell::new(Vec::new()),
        });

        bar.refresh_hypr();
        bar.tick_clock();
        bar.tick_cpu();
        bar.tick_mem();
        bar.tick_gpu();
        bar.tick_controls();
        bar.start_timers();
        bar
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
    }

    /// Refresh workspaces + the per-app label from live Hyprland state.
    pub fn refresh_hypr(&self) {
        let snap = hypr::snapshot();
        if self.cfg.compact {
            self.compact_workspaces(&snap);
        }
        for s in &self.surfaces {
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
        for s in &self.surfaces {
            s.set_submap(name);
        }
    }

    fn tick_clock(&self) {
        let now = glib::DateTime::now_local().ok();
        let text = now
            .and_then(|d| d.format(&self.cfg.clock_format).ok())
            .map(|g| g.to_string())
            .unwrap_or_default();
        for s in &self.surfaces {
            s.clock_label.set_text(&text);
        }
    }

    fn tick_cpu(&self) {
        let frac = self.cpu.borrow_mut().sample();
        let pct = (frac * 100.0).round() as u32;
        for s in &self.surfaces {
            s.cpu_spark.push(frac);
            s.cpu_val.set_text(&format!("{pct}%"));
        }
    }

    fn tick_mem(&self) {
        let m = sysinfo::mem();
        let pct = (m.used_frac * 100.0).round() as u32;
        for s in &self.surfaces {
            s.mem_spark.push(m.used_frac);
            s.mem_val.set_text(&format!("{pct}%"));
        }
    }

    fn tick_gpu(&self) {
        match sysinfo::gpu() {
            Some(frac) => {
                let pct = (frac * 100.0).round() as u32;
                for s in &self.surfaces {
                    s.gpu_spark.push(frac);
                    s.gpu_val.set_text(&format!("{pct}%"));
                    s.gpu_metric.set_visible(true);
                }
            }
            None => {
                for s in &self.surfaces {
                    s.gpu_metric.set_visible(false);
                }
            }
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

        *self.throughput.borrow_mut() = self.netmeter.borrow_mut().sample(2.0);

        for s in &self.surfaces {
            s.set_audio(&audio);
            s.set_net(&net);
            s.set_battery(&battery);
            s.set_brightness(brightness);
            s.set_bell(&bell);
            s.set_gamemode(game);
            s.set_nowplaying(np.as_ref());
        }
    }

    /// SIGUSR2 — re-read colors.css (CSS + parsed palette) and repaint.
    pub fn reload_palette(&self) {
        *self.palette.borrow_mut() = Palette::load();
        self.css.reload();
        for s in &self.surfaces {
            s.repaint_drawn();
        }
    }

    /// SIGUSR1 — toggle every bar's visibility (parity with waybar-toggle.sh).
    pub fn toggle_visibility(&self) {
        for s in &self.surfaces {
            let vis = s.window.is_visible();
            s.window.set_visible(!vis);
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

    /// Rebuild each surface's tray cluster from the current item + menu state.
    fn rebuild_tray(self: &Rc<Self>) {
        let items = self.tray_items.borrow();
        for s in &self.surfaces {
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
        let pop = menu.map(|root| popovers::tray_menu(&row, &root, &item.key, self.tray_cmd.clone()));

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
    bar_box: CenterBox,

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
    np_title: Label,
    np_artist: Label,

    cpu_spark: Sparkline,
    cpu_val: Label,
    mem_spark: Sparkline,
    mem_val: Label,
    gpu_spark: Sparkline,
    gpu_val: Label,
    gpu_metric: GtkBox,

    net_ctl: Button,
    net_glyph: Label,
    net_val: Label,

    vol_glyph: Label,
    vol_val: Label,
    vol_ctl: Button,

    bri_ctl: GtkBox,
    bri_val: Label,

    bat_ctl: GtkBox,
    bat_glyph: Label,
    bat_val: Label,

    bell_btn: Button,
    bell_glyph: Label,
    bell_dot: GtkBox,

    clock_label: Label,

    gamemode_box: GtkBox,
    tray_box: GtkBox,

    mirror: gtk4::DrawingArea,
}

impl Surface {
    fn build(
        app: &gtk4::Application,
        monitor: &gdk::Monitor,
        cfg: &Config,
        pal: &SharedPalette,
        throughput: Rc<RefCell<Throughput>>,
    ) -> Rc<Surface> {
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

        // ── LEFT ────────────────────────────────────────────────────────
        let left = GtkBox::new(Orientation::Horizontal, 0);
        left.set_halign(Align::Start);

        // Tezca mirror menu (drawn glyph inside a flat button).
        let mirror = draw::mirror_glyph(pal, 16.0);
        mirror.set_valign(Align::Center);
        let mirror_btn = Button::new();
        mirror_btn.add_css_class("mirror");
        mirror_btn.set_child(Some(&mirror));
        let tezca_pop = popovers::tezca_menu(&mirror_btn);
        mirror_btn.connect_clicked(move |_| tezca_pop.popup());
        left.append(&mirror_btn);

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

        if compact {
            left.append(&sep());
            left.append(&ws_box);
        } else {
            left.append(&sep());
            left.append(&app_label);
            left.append(&sep());
            left.append(&ws_box);
        }
        left.append(&submap_box);

        // ── CENTER: now-playing ─────────────────────────────────────────
        let np_box = GtkBox::new(Orientation::Horizontal, 10);
        np_box.add_css_class("nowplaying");
        np_box.set_halign(Align::Center);
        let art = GtkBox::new(Orientation::Horizontal, 0);
        art.add_css_class("np-art");
        let np_text = GtkBox::new(Orientation::Vertical, 0);
        let np_title = Label::new(None);
        np_title.add_css_class("np-title");
        np_title.set_halign(Align::Start);
        let np_artist = Label::new(None);
        np_artist.add_css_class("np-artist");
        np_artist.set_halign(Align::Start);
        np_text.append(&np_title);
        np_text.append(&np_artist);
        let eq = draw::equalizer(pal);
        np_box.append(&art);
        np_box.append(&np_text);
        np_box.append(&eq);
        np_box.set_visible(false);
        // Click = play/pause; scroll = seek.
        let click = gtk4::GestureClick::new();
        click.connect_released(|_, _, _, _| nowplaying::play_pause());
        np_box.add_controller(click);
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(|_, _, dy| {
            nowplaying::seek(if dy < 0.0 { 5 } else { -5 });
            glib::Propagation::Stop
        });
        np_box.add_controller(scroll);

        // ── RIGHT ───────────────────────────────────────────────────────
        let right = GtkBox::new(Orientation::Horizontal, 0);
        right.set_halign(Align::End);

        // Game mode (hidden unless on).
        let gamemode_box = GtkBox::new(Orientation::Horizontal, 0);
        gamemode_box.add_css_class("gamemode");
        let game_glyph = Label::new(Some(G_GAME));
        game_glyph.add_css_class("glyph");
        gamemode_box.append(&game_glyph);
        gamemode_box.set_visible(false);
        right.append(&gamemode_box);

        // System tray (StatusNotifierItem icons) — filled live by the tray
        // thread; hidden until the first item registers.
        let tray_box = GtkBox::new(Orientation::Horizontal, 2);
        tray_box.add_css_class("tray");
        tray_box.set_valign(Align::Center);
        tray_box.set_visible(false);
        right.append(&tray_box);

        // Metrics: CPU + MEM sparklines.
        let cpu_spark = draw::sparkline(pal, draw::SparkColor::Accent);
        let cpu_val = Label::new(Some("0%"));
        cpu_val.add_css_class("metric-val");
        let cpu_metric = metric(G_CPU_LABEL, &cpu_spark.area, &cpu_val);

        let mem_spark = draw::sparkline(pal, draw::SparkColor::Gold);
        let mem_val = Label::new(Some("0%"));
        mem_val.add_css_class("metric-val");
        let mem_metric = metric(G_MEM_LABEL, &mem_spark.area, &mem_val);

        // GPU — hidden until the first successful read (absent on GPU-less rigs).
        let gpu_spark = draw::sparkline(pal, draw::SparkColor::AccentDim);
        let gpu_val = Label::new(Some("0%"));
        gpu_val.add_css_class("metric-val");
        let gpu_metric = metric(G_GPU_LABEL, &gpu_spark.area, &gpu_val);
        gpu_metric.set_visible(false);

        // Each metric group expands into a glass detail popover on click.
        attach_detail(&cpu_metric, popovers::cpu_detail(&cpu_metric));
        attach_detail(&mem_metric, popovers::mem_detail(&mem_metric));
        attach_detail(&gpu_metric, popovers::gpu_detail(&gpu_metric));

        right.append(&cpu_metric);
        right.append(&mem_metric);
        right.append(&gpu_metric);
        right.append(&sep());

        // Controls: network (button → popover).
        let (net_ctl, net_glyph, net_val) = control_button();
        net_glyph.set_text(G_WIFI);
        let net_pop = popovers::network(&net_ctl, throughput.clone());
        net_ctl.connect_clicked(move |_| net_pop.popup());
        right.append(&net_ctl);

        // Volume (button → mixer popover).
        let (vol_ctl, vol_glyph, vol_val) = control_button();
        vol_glyph.set_text(G_VOL[2]);
        let mix_pop = popovers::mixer(&vol_ctl);
        vol_ctl.connect_clicked(move |_| mix_pop.popup());
        right.append(&vol_ctl);

        // Brightness (display-only; hidden on desktops with no backlight).
        let bri_ctl = GtkBox::new(Orientation::Horizontal, 5);
        bri_ctl.add_css_class("control");
        let bri_glyph = Label::new(Some(G_BRIGHT));
        bri_glyph.add_css_class("glyph");
        let bri_val = Label::new(None);
        bri_val.add_css_class("control-val");
        bri_ctl.append(&bri_glyph);
        bri_ctl.append(&bri_val);
        bri_ctl.set_visible(false);
        right.append(&bri_ctl);

        // Battery (hidden on desktops with no battery).
        let bat_ctl = GtkBox::new(Orientation::Horizontal, 5);
        bat_ctl.add_css_class("control");
        let bat_glyph = Label::new(Some(G_BATT));
        bat_glyph.add_css_class("glyph");
        let bat_val = Label::new(None);
        bat_val.add_css_class("control-val");
        bat_ctl.append(&bat_glyph);
        bat_ctl.append(&bat_val);
        bat_ctl.set_visible(false);
        right.append(&bat_ctl);

        right.append(&sep());

        // Notification bell with an urgent dot badge.
        let bell_overlay = Overlay::new();
        let bell_glyph = Label::new(Some(G_NOTIF));
        bell_glyph.add_css_class("glyph");
        bell_overlay.set_child(Some(&bell_glyph));
        let bell_dot = GtkBox::new(Orientation::Horizontal, 0);
        bell_dot.add_css_class("notif-dot");
        bell_dot.set_halign(Align::End);
        bell_dot.set_valign(Align::Start);
        bell_dot.set_visible(false);
        bell_overlay.add_overlay(&bell_dot);
        let bell_btn = Button::new();
        bell_btn.add_css_class("bell");
        bell_btn.set_child(Some(&bell_overlay));
        bell_btn.connect_clicked(|_| notify::toggle_panel());
        let bell_right = gtk4::GestureClick::new();
        bell_right.set_button(gdk::BUTTON_SECONDARY);
        bell_right.connect_released(|_, _, _, _| notify::toggle_dnd());
        bell_btn.add_controller(bell_right);
        right.append(&bell_btn);

        // Clock (button → calendar popover).
        let clock_btn = Button::new();
        clock_btn.add_css_class("clock");
        let clock_label = Label::new(None);
        clock_btn.set_child(Some(&clock_label));
        let cal_pop = popovers::calendar(&clock_btn);
        clock_btn.connect_clicked(move |_| cal_pop.popup());
        right.append(&clock_btn);

        // Power → wlogout.
        let power_btn = Button::new();
        power_btn.add_css_class("power");
        power_btn.set_child(Some(&Label::new(Some(G_POWER))));
        power_btn.connect_clicked(|_| {
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg("uwsm app -- wlogout -b 4 || wlogout -b 4")
                .spawn();
        });
        right.append(&power_btn);

        bar_box.set_start_widget(Some(&left));
        bar_box.set_center_widget(Some(&np_box));
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

        Rc::new(Surface {
            window,
            output,
            compact,
            bar_box,
            ws_box,
            ws_assigned,
            hide_empty,
            numerals,
            app_label,
            submap_box,
            submap_label,
            np_box,
            np_title,
            np_artist,
            cpu_spark,
            cpu_val,
            mem_spark,
            mem_val,
            gpu_spark,
            gpu_val,
            gpu_metric,
            net_ctl,
            net_glyph,
            net_val,
            vol_glyph,
            vol_val,
            vol_ctl,
            bri_ctl,
            bri_val,
            bat_ctl,
            bat_glyph,
            bat_val,
            bell_btn,
            bell_glyph,
            bell_dot,
            clock_label,
            gamemode_box,
            tray_box,
            mirror,
        })
    }

    // ── updates ─────────────────────────────────────────────────────────

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
            self.ws_box.append(&ws_button(active, &ws_label(active, self.numerals), true, false, mayan));
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
            self.vol_glyph.set_text(G_MUTED);
            self.vol_val.set_text("");
            self.vol_ctl.add_css_class("muted");
        } else {
            let idx = match a.volume {
                0..=32 => 0,
                33..=66 => 1,
                _ => 2,
            };
            self.vol_glyph.set_text(G_VOL[idx]);
            self.vol_val.set_text(&format!("{}%", a.volume));
            self.vol_ctl.remove_css_class("muted");
        }
    }

    fn set_net(&self, n: &Net) {
        self.net_ctl.remove_css_class("disconnected");
        match n {
            Net::Wifi { signal, .. } => {
                self.net_glyph.set_text(G_WIFI);
                self.net_val.set_text(&format!("{signal}%"));
            }
            Net::Ethernet { .. } => {
                self.net_glyph.set_text(G_ETH);
                self.net_val.set_text("");
            }
            Net::Disconnected => {
                self.net_glyph.set_text(G_DISC);
                self.net_val.set_text("");
                self.net_ctl.add_css_class("disconnected");
            }
        }
    }

    fn set_battery(&self, b: &Option<sysinfo::Battery>) {
        match b {
            Some(b) => {
                self.bat_glyph.set_text(if b.charging { G_BATT_CHG } else { G_BATT });
                self.bat_val.set_text(&format!("{}%", b.percent));
                self.bat_ctl.set_visible(true);
            }
            None => self.bat_ctl.set_visible(false),
        }
    }

    fn set_brightness(&self, b: Option<u32>) {
        match b {
            Some(p) => {
                self.bri_val.set_text(&format!("{p}%"));
                self.bri_ctl.set_visible(true);
            }
            None => self.bri_ctl.set_visible(false),
        }
    }

    fn set_bell(&self, s: &notify::BellState) {
        if s.unread > 0 {
            self.bell_glyph.set_text(G_NOTIF_ON);
            self.bell_btn.add_css_class("unread");
            self.bell_dot.set_visible(true);
        } else {
            self.bell_glyph.set_text(G_NOTIF);
            self.bell_btn.remove_css_class("unread");
            self.bell_dot.set_visible(false);
        }
    }

    fn set_gamemode(&self, on: bool) {
        if on {
            self.gamemode_box.set_visible(true);
            self.gamemode_box.add_css_class("active");
        } else {
            self.gamemode_box.set_visible(false);
            self.gamemode_box.remove_css_class("active");
        }
    }

    fn set_nowplaying(&self, np: Option<&nowplaying::NowPlaying>) {
        match np {
            Some(t) => {
                self.np_title.set_text(&t.title);
                self.np_artist.set_text(&t.artist);
                self.np_box.set_visible(true);
            }
            None => self.np_box.set_visible(false),
        }
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

/// A 1×18 hairline separator.
fn sep() -> GtkBox {
    let s = GtkBox::new(Orientation::Horizontal, 0);
    s.add_css_class("sep");
    s.set_size_request(1, 18);
    s.set_valign(Align::Center);
    s
}

/// `LABEL  <spark>  val%` metric group.
fn metric(label: &str, spark: &gtk4::DrawingArea, val: &Label) -> GtkBox {
    let b = GtkBox::new(Orientation::Horizontal, 7);
    b.add_css_class("metric");
    let l = Label::new(Some(label));
    l.add_css_class("metric-label");
    b.append(&l);
    b.append(spark);
    b.append(val);
    b
}

/// Parent `pop` to `widget` and pop it up on click, marking the group hoverable.
fn attach_detail(widget: &impl IsA<gtk4::Widget>, pop: gtk4::Popover) {
    widget.add_css_class("clickable");
    let click = gtk4::GestureClick::new();
    click.connect_released(move |_, _, _, _| pop.popup());
    widget.add_controller(click);
}

/// A `.control` button holding a glyph + value; returns handles to both labels.
fn control_button() -> (Button, Label, Label) {
    let b = Button::new();
    b.add_css_class("control");
    let inner = GtkBox::new(Orientation::Horizontal, 5);
    let glyph = Label::new(None);
    glyph.add_css_class("glyph");
    let val = Label::new(None);
    val.add_css_class("control-val");
    inner.append(&glyph);
    inner.append(&val);
    b.set_child(Some(&inner));
    (b, glyph, val)
}

/// A workspace pill button showing `label`, switching to `id` on click.
/// `mayan` styles it for Mayan bar-and-dot numerals (covering font + sizing).
fn ws_button(id: i32, label: &str, active: bool, occupied: bool, mayan: bool) -> Button {
    let b = Button::with_label(label);
    b.add_css_class("ws");
    if mayan {
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

/// A workspace's pill label in the configured numeral system.
fn ws_label(id: i32, numerals: Numerals) -> String {
    match numerals {
        Numerals::Arabic => id.to_string(),
        Numerals::Mayan => mayan(id),
    }
}

/// Mayan numeral for `n` from the Unicode Mayan Numerals block
/// (U+1D2E0 ZERO … U+1D2F3 NINETEEN — bars-and-dots), digits beyond 19.
/// Needs a covering font (Noto Sans Mayan Numerals); see `button.ws.mayan`.
fn mayan(n: i32) -> String {
    match n {
        0..=19 => char::from_u32(0x1D2E0 + n as u32).map(String::from).unwrap_or_else(|| n.to_string()),
        _ => n.to_string(),
    }
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
