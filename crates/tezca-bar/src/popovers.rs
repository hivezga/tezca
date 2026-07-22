//! Glass popovers — the expandable modules from the prototype's section 03.
//!
//! Each is a [`gtk4::Popover`] styled `.tz-popover` (obsidian glass, blurred by
//! the compositor's layerrule on the bar namespace). Content that reflects live
//! state is rebuilt in `connect_show`, so opening one always shows current data:
//!   * clock  → calendar
//!   * audio  → per-sink/source mixer
//!   * network → SSID + connection detail
//! Plus the Tezca "mirror" system menu.

use crate::sysinfo::{self, Net, Throughput};
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
