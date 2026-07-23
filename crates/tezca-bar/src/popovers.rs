//! Glass popovers — the expandable modules from the prototype's section 03.
//!
//! Each is a [`gtk4::Popover`] styled `.tz-popover` (obsidian glass, blurred by
//! the compositor's layerrule on the bar namespace). Content that reflects live
//! state is rebuilt in `connect_show`, so opening one always shows current data:
//!   * clock  → calendar
//!   * audio  → per-sink/source mixer
//!   * network → SSID + connection detail
//! Plus the Tezca "mirror" system menu.

use crate::ai;
use crate::sysinfo::{self, Net, Throughput};
use crate::tray;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Calendar, Label, LevelBar, Orientation, Popover};
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

/// A glass popover parented to `anchor`, with an empty content box to fill.
fn glass(anchor: &impl IsA<gtk4::Widget>) -> (Popover, GtkBox) {
    let pop = Popover::new();
    pop.add_css_class("tz-popover");
    pop.set_has_arrow(false);
    pop.set_parent(anchor);
    let content = GtkBox::new(Orientation::Vertical, 12);
    pop.set_child(Some(&content));
    (pop, content)
}

fn sh(cmd: &str) {
    let _ = Command::new("sh").arg("-c").arg(cmd).spawn();
}

// ── Tezca system menu ──────────────────────────────────────────────────────

/// The mirror-glyph menu: Settings / Lock / Sleep / Log Out.
pub fn tezca_menu(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(180);

    let title = Label::new(Some("Tezca"));
    title.add_css_class("pop-title");
    title.set_halign(Align::Start);
    content.append(&title);

    // (label, shell command) — plain sh so both uwsm-app and direct forms work.
    let items = [
        ("Settings", "uwsm app -- tezca-settings || tezca-settings"),
        ("Lock", "loginctl lock-session || hyprlock"),
        ("Sleep", "systemctl suspend"),
        ("Log Out", "uwsm stop || hyprctl dispatch exit"),
    ];
    for (label, cmd) in items {
        let b = Button::with_label(label);
        b.add_css_class("appmenu-item");
        b.set_halign(Align::Fill);
        if let Some(child) = b.child() {
            child.set_halign(Align::Start);
        }
        let pop_c = pop.clone();
        let cmd = cmd.to_string();
        b.connect_clicked(move |_| {
            sh(&cmd);
            pop_c.popdown();
        });
        content.append(&b);
    }
    pop
}

// ── Calendar (clock) ───────────────────────────────────────────────────────

pub fn calendar(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    let cal = Calendar::new();
    cal.add_css_class("tz-cal");
    content.append(&cal);
    pop
}

// ── Audio mixer ────────────────────────────────────────────────────────────

pub fn mixer(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(240);
    let content_c = content.clone();
    pop.connect_show(move |_| {
        while let Some(c) = content_c.first_child() {
            content_c.remove(&c);
        }
        let rows = [
            ("Output", "@DEFAULT_AUDIO_SINK@"),
            ("Input", "@DEFAULT_AUDIO_SOURCE@"),
        ];
        for (label, id) in rows {
            let a = sysinfo::audio_of(id);
            let (vol, muted) = a.map(|x| (x.volume, x.muted)).unwrap_or((0, true));
            content_c.append(&mix_row(label, vol, muted));
        }
    });
    pop
}

fn mix_row(label: &str, vol: u32, muted: bool) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 6);
    let head = GtkBox::new(Orientation::Horizontal, 8);
    let l = Label::new(Some(label));
    l.add_css_class("mix-label");
    l.set_halign(Align::Start);
    l.set_hexpand(true);
    let v = Label::new(Some(&if muted { "muted".to_string() } else { vol.to_string() }));
    v.add_css_class("mix-val");
    v.set_halign(Align::End);
    head.append(&l);
    head.append(&v);
    let bar = LevelBar::builder()
        .mode(gtk4::LevelBarMode::Continuous)
        .min_value(0.0)
        .max_value(100.0)
        .value(if muted { 0.0 } else { vol as f64 })
        .hexpand(true)
        .build();
    bar.add_css_class("mix");
    row.append(&head);
    row.append(&bar);
    row
}

// ── Network detail ─────────────────────────────────────────────────────────

pub fn network(anchor: &impl IsA<gtk4::Widget>, tp: Rc<RefCell<Throughput>>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(230);
    let content_c = content.clone();
    pop.connect_show(move |_| {
        while let Some(c) = content_c.first_child() {
            content_c.remove(&c);
        }
        let t = tp.borrow();
        let (ssid, ip, signal, connected) = match sysinfo::net() {
            Net::Wifi { ssid, ip, signal } => (ssid, ip, Some(signal), true),
            Net::Ethernet { ip } => ("Wired".to_string(), ip, None, true),
            Net::Disconnected => ("Disconnected".to_string(), String::new(), None, false),
        };

        let head = GtkBox::new(Orientation::Horizontal, 8);
        let name = Label::new(Some(&ssid));
        name.add_css_class("pop-title");
        name.set_halign(Align::Start);
        name.set_hexpand(true);
        head.append(&name);
        if connected {
            let chip = Label::new(Some("connected"));
            chip.add_css_class("chip-connected");
            chip.set_halign(Align::End);
            head.append(&chip);
        }
        content_c.append(&head);

        let rows = GtkBox::new(Orientation::Vertical, 7);
        if !ip.is_empty() {
            rows.append(&mono_row("ipv4", &ip, false));
        }
        rows.append(&mono_row("down", &format!("\u{2193} {:.0} Mb/s", t.down_mbps), false));
        rows.append(&mono_row("up", &format!("\u{2191} {:.0} Mb/s", t.up_mbps), false));
        if let Some(s) = signal {
            rows.append(&mono_row("signal", &format!("{s}%"), true));
        }
        content_c.append(&rows);
    });
    pop
}

// ── Tray item menu (DBusMenu) ───────────────────────────────────────────────

/// A glass popover rendering an app's DBusMenu; leaf clicks dispatch a
/// `MenuClicked` back over the tray channel. Submenus nest as child popovers.
pub fn tray_menu(
    anchor: &impl IsA<gtk4::Widget>,
    root: &tray::MenuNode,
    key: &str,
    cmd: async_channel::Sender<tray::TrayCmd>,
) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(180);
    fill_menu(&content, root, key, &cmd, &pop);
    pop
}

fn fill_menu(
    content: &GtkBox,
    node: &tray::MenuNode,
    key: &str,
    cmd: &async_channel::Sender<tray::TrayCmd>,
    root: &Popover,
) {
    for child in node.children.iter().filter(|c| c.visible) {
        if child.separator {
            let line = GtkBox::new(Orientation::Horizontal, 0);
            line.add_css_class("sep");
            line.set_size_request(-1, 1);
            content.append(&line);
            continue;
        }

        let mark = match child.checked {
            Some(true) => "\u{2713} ",
            Some(false) => "  ",
            None => "",
        };
        let btn = Button::with_label(&format!("{mark}{}", child.label));
        btn.add_css_class("appmenu-item");
        btn.set_halign(Align::Fill);
        btn.set_sensitive(child.enabled);
        if let Some(c) = btn.child() {
            c.set_halign(Align::Start);
        }

        if child.children.iter().any(|c| c.visible) {
            // Submenu → open a nested glass popover anchored on this row.
            let sub = Popover::new();
            sub.add_css_class("tz-popover");
            sub.set_has_arrow(false);
            sub.set_position(gtk4::PositionType::Right);
            sub.set_parent(&btn);
            let sub_content = GtkBox::new(Orientation::Vertical, 12);
            sub.set_child(Some(&sub_content));
            fill_menu(&sub_content, child, key, cmd, root);
            let sub_c = sub.clone();
            btn.connect_clicked(move |_| sub_c.popup());
        } else {
            let (cmd, key, id, root) = (cmd.clone(), key.to_string(), child.id, root.clone());
            btn.connect_clicked(move |_| {
                let _ = cmd.send_blocking(tray::TrayCmd::MenuClicked { key: key.clone(), id });
                root.popdown();
            });
        }
        content.append(&btn);
    }
}

// ── Hardware detail (CPU / MEM / GPU metric popovers) ───────────────────────

/// Clear a popover's content box (rebuild-on-show pattern).
fn clear(b: &GtkBox) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A titled section header for the top of a detail popover.
fn pop_title(text: &str) -> Label {
    let l = Label::new(Some(text));
    l.add_css_class("pop-title");
    l.set_halign(Align::Start);
    l.set_max_width_chars(24);
    l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    l
}

/// `label … value` over a continuous meter filled to `frac` (0..1).
fn meter_row(label: &str, value: &str, frac: f64) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 6);
    let head = GtkBox::new(Orientation::Horizontal, 8);
    let l = Label::new(Some(label));
    l.add_css_class("mix-label");
    l.set_halign(Align::Start);
    l.set_hexpand(true);
    let v = Label::new(Some(value));
    v.add_css_class("mix-val");
    v.set_halign(Align::End);
    head.append(&l);
    head.append(&v);
    let bar = LevelBar::builder()
        .mode(gtk4::LevelBarMode::Continuous)
        .min_value(0.0)
        .max_value(1.0)
        .value(frac.clamp(0.0, 1.0))
        .hexpand(true)
        .build();
    bar.add_css_class("mix");
    row.append(&head);
    row.append(&bar);
    row
}

/// CPU detail: model, temperature, clock, load average, thread count.
pub fn cpu_detail(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(250);
    let c = content.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let d = sysinfo::cpu_detail();
        c.append(&pop_title(&d.model));
        if let Some(t) = d.temp_c {
            c.append(&meter_row("Temperature", &format!("{t:.0} \u{00B0}C"), t / 100.0));
        }
        let rows = GtkBox::new(Orientation::Vertical, 7);
        if let Some(f) = d.freq_mhz {
            rows.append(&mono_row("clock", &format!("{:.2} GHz", f / 1000.0), false));
        }
        let (l1, l5, l15) = d.load;
        rows.append(&mono_row("load", &format!("{l1:.2} · {l5:.2} · {l15:.2}"), false));
        if d.threads > 0 {
            rows.append(&mono_row("threads", &d.threads.to_string(), false));
        }
        c.append(&rows);
    });
    pop
}

/// Memory detail: used / cached / buffers / swap breakdown + DIMM temp.
pub fn mem_detail(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(250);
    let c = content.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let d = sysinfo::mem_detail();
        let gib = |kb: f64| kb / (1024.0 * 1024.0);
        c.append(&pop_title("Memory"));
        let used_frac = if d.total_kb > 0.0 { d.used_kb / d.total_kb } else { 0.0 };
        c.append(&meter_row(
            "Used",
            &format!("{:.1} / {:.1} GiB", gib(d.used_kb), gib(d.total_kb)),
            used_frac,
        ));
        if d.swap_total_kb > 0.0 {
            c.append(&meter_row(
                "Swap",
                &format!("{:.1} / {:.1} GiB", gib(d.swap_used_kb), gib(d.swap_total_kb)),
                d.swap_used_kb / d.swap_total_kb,
            ));
        }
        let rows = GtkBox::new(Orientation::Vertical, 7);
        rows.append(&mono_row("available", &format!("{:.1} GiB", gib(d.available_kb)), false));
        rows.append(&mono_row("cached", &format!("{:.1} GiB", gib(d.cached_kb)), false));
        rows.append(&mono_row("buffers", &format!("{:.1} GiB", gib(d.buffers_kb)), false));
        if let Some(t) = d.dimm_temp_c {
            rows.append(&mono_row("dimm temp", &format!("{t:.0} \u{00B0}C"), false));
        }
        c.append(&rows);
    });
    pop
}

/// GPU detail: utilization, temperature, power, VRAM, clocks, fan.
pub fn gpu_detail(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(250);
    let c = content.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let Some(d) = sysinfo::gpu_detail() else {
            c.append(&pop_title("GPU"));
            c.append(&mono_row("status", "no telemetry", false));
            return;
        };
        c.append(&pop_title(&d.name));
        if let Some(u) = d.util_pct {
            c.append(&meter_row("Utilization", &format!("{u:.0}%"), u / 100.0));
        }
        if let Some(t) = d.temp_c {
            c.append(&meter_row("Temperature", &format!("{t:.0} \u{00B0}C"), t / 100.0));
        }
        if let (Some(u), Some(t)) = (d.mem_used_mb, d.mem_total_mb) {
            let frac = if t > 0.0 { u / t } else { 0.0 };
            c.append(&meter_row(
                "VRAM",
                &format!("{:.1} / {:.1} GiB", u / 1024.0, t / 1024.0),
                frac,
            ));
        }
        if let (Some(p), Some(lim)) = (d.power_w, d.power_limit_w) {
            let frac = if lim > 0.0 { p / lim } else { 0.0 };
            c.append(&meter_row("Power", &format!("{p:.0} / {lim:.0} W"), frac));
        }
        let rows = GtkBox::new(Orientation::Vertical, 7);
        if d.power_limit_w.is_none() {
            if let Some(p) = d.power_w {
                rows.append(&mono_row("power", &format!("{p:.0} W"), false));
            }
        }
        if let Some(cl) = d.core_clock_mhz {
            rows.append(&mono_row("core clock", &format!("{cl:.0} MHz"), false));
        }
        if let Some(mc) = d.mem_clock_mhz {
            rows.append(&mono_row("mem clock", &format!("{mc:.0} MHz"), false));
        }
        if let Some(fan) = d.fan_pct {
            rows.append(&mono_row("fan", &format!("{fan:.0}%"), false));
        }
        if rows.first_child().is_some() {
            c.append(&rows);
        }
    });
    pop
}

/// AI usage detail: one section per provider — its rate-limit windows as
/// meters with reset countdowns, plus today's locally-computed token/cost
/// totals. Rebuilt from the shared snapshot on every show, so it always
/// reflects the last poll rather than the state at construction time.
///
/// Deliberately shows no account identifiers and no credential material — the
/// most this popover ever names is the plan tier the provider reported.
pub fn ai_detail(anchor: &impl IsA<gtk4::Widget>, state: Rc<RefCell<ai::Snapshot>>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(280);
    let c = content.clone();
    // Weak, so the sign-in button below can dismiss the popover it lives in
    // without the popover owning a reference cycle back to itself.
    let pw = pop.downgrade();
    pop.connect_show(move |_| {
        clear(&c);
        let snap = state.borrow();

        let shown: Vec<_> = snap.providers.iter().filter(|p| p.status.visible()).collect();
        if shown.is_empty() {
            c.append(&pop_title("AI usage"));
            c.append(&mono_row("status", "no provider configured", false));
            return;
        }

        for (i, p) in shown.iter().enumerate() {
            if i > 0 {
                c.append(&sep_row());
            }
            let title = match &p.plan {
                Some(plan) if !plan.is_empty() => format!("{}  ·  {}", p.name, plan),
                _ => p.name.to_string(),
            };
            c.append(&pop_title(&title));

            for w in &p.windows {
                c.append(&window_row(w));
            }
            if let Some(sp) = &p.spend {
                // Below the windows and captioned as money, because "20%" next
                // to a stack of rate limits otherwise reads as a fifth limit.
                let row = meter_row(
                    "Extra credits",
                    &format!("{} / {}", sp.money(sp.used), sp.money(sp.limit)),
                    sp.pct / 100.0,
                );
                row.append(&caption("pay-as-you-go, not a rate limit"));
                c.append(&row);
            }

            // Whatever went wrong is stated plainly rather than leaving an
            // empty section the user has to interpret.
            let note = match &p.status {
                ai::Status::RateLimited { until } if *until > 0 => {
                    Some(format!("rate limited · retry in {}", ai::until(*until)))
                }
                ai::Status::RateLimited { .. } => Some("rate limited".to_string()),
                ai::Status::NeedsLogin => Some("session expired".to_string()),
                ai::Status::Error(e) => Some(e.clone()),
                ai::Status::LocalOnly if p.windows.is_empty() => {
                    Some("local data only (offline)".to_string())
                }
                _ => None,
            };
            if let Some(note) = note {
                c.append(&mono_row("status", &note, false));
            }

            // A session about to lapse is worth flagging before it starts
            // returning 401s. One that's weeks out is just noise, so it's only
            // shown inside the last three days.
            if let Some(t) = p.session_expires {
                let left = t - ai::now_unix();
                if left > 0 && left < 3 * 86_400 {
                    c.append(&mono_row("session", &format!("expires in {}", ai::until(t)), false));
                }
            }

            // Tezca never writes the credential — it belongs to Claude Code. So
            // "sign in" means launching the real client, on the same AI
            // scratchpad that SUPER+ALT+SHIFT+A uses.
            if p.status == ai::Status::NeedsLogin {
                let b = Button::with_label("Sign in with claude");
                b.add_css_class("appmenu-item");
                b.set_halign(Align::Fill);
                if let Some(child) = b.child() {
                    child.set_halign(Align::Start);
                }
                let pw = pw.clone();
                b.connect_clicked(move |_| {
                    if let Some(pop) = pw.upgrade() {
                        pop.popdown();
                    }
                    sh("uwsm app -- kitty --class tezca-ai -e claude \
                        || kitty --class tezca-ai -e claude");
                });
                c.append(&b);
            }

            if let Some(l) = &p.local {
                let rows = GtkBox::new(Orientation::Vertical, 7);
                rows.append(&mono_row("today", &format!("{} tok", ai::compact_count(l.total_tokens())), false));
                if l.cost_usd > 0.0 {
                    // Named "API-equivalent" because on a subscription plan
                    // this is not money you are actually charged.
                    rows.append(&mono_row("api-equiv", &format!("${:.2}", l.cost_usd), true));
                }
                if l.messages > 0 {
                    rows.append(&mono_row("messages", &l.messages.to_string(), false));
                }
                c.append(&rows);
            }
        }

        // Freshness footer — the poll interval is minutes, so "when did this
        // last update" is real information, not decoration.
        if snap.updated > 0 {
            c.append(&sep_row());
            c.append(&mono_row("updated", &ai::ago(snap.updated), false));
        }
    });
    pop
}

/// One rate-limit window: title, percent + countdown, meter, and a caption
/// spelling out what the number actually covers.
///
/// The caption is the point of this function. A bare `37%` is unreadable —
/// weekly or daily? this model or all of them? — and the account can carry
/// several overlapping windows at once, so each one has to say which it is.
fn window_row(w: &ai::Window) -> GtkBox {
    let value = match w.resets_at {
        Some(t) => format!("{:.0}%   {}", w.pct, ai::until(t)),
        None => format!("{:.0}%", w.pct),
    };
    let row = meter_row(&w.label, &value, w.pct / 100.0);
    // Of several limits only one is binding; saying so beats making the reader
    // compare percentages and guess.
    let scope = if w.active { format!("{}  ·  in use now", w.scope) } else { w.scope.clone() };
    row.append(&caption(&scope));
    row
}

/// Muted sub-line under a meter, explaining what it measures.
fn caption(text: &str) -> Label {
    let l = Label::new(Some(text));
    l.add_css_class("pop-sub");
    l.set_halign(Align::Start);
    l.set_max_width_chars(34);
    l.set_wrap(true);
    l
}

/// A hairline divider between provider sections.
fn sep_row() -> GtkBox {
    let s = GtkBox::new(Orientation::Horizontal, 0);
    s.add_css_class("pop-sep");
    s.set_size_request(-1, 1);
    s
}

fn mono_row(key: &str, val: &str, accent: bool) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    let k = Label::new(Some(key));
    k.add_css_class("pop-mono");
    k.set_halign(Align::Start);
    k.set_hexpand(true);
    let v = Label::new(Some(val));
    v.add_css_class(if accent { "chip-connected" } else { "pop-mono-val" });
    v.set_halign(Align::End);
    row.append(&k);
    row.append(&v);
    row
}
