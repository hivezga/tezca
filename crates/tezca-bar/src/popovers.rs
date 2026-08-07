//! Glass popovers — the expandable modules from the prototype's section 03.
//!
//! Each is a [`gtk4::Popover`] styled `.tz-popover` (obsidian glass, blurred by
//! the compositor's layerrule on the bar namespace). Content that reflects live
//! state is rebuilt in `connect_show`, so opening one always shows current data:
//!   * clock  → calendar
//!   * audio  → per-sink/source mixer
//!   * network → SSID + connection detail, radio toggle
//!   * bluetooth → connected devices + battery
//!
//! Plus the Tezca "mirror" system menu.

use crate::ai;
use crate::draw::SharedPalette;
use crate::icon::{self, Icon};
use crate::sysinfo::{self, Net, Throughput};
use crate::tray;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Calendar, Label, LevelBar, Orientation, Popover};
use std::cell::RefCell;
use std::collections::HashMap;
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
        // The fallback needs Lua dispatcher syntax: the Lua config manager
        // rejects the bare `dispatch exit` form outright rather than accepting
        // it for compatibility. `uwsm stop` is still tried first — upstream
        // advises uwsm sessions against `exit`, which pulls Hyprland out from
        // under its clients instead of shutting the session down in order.
        ("Log Out", "uwsm stop || hyprctl dispatch 'hl.dsp.exit()'"),
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

// ── Session menu (power) ───────────────────────────────────────────────────

/// The power glyph's menu: the five session actions, each with the chord that
/// reaches it without the mouse.
///
/// The button used to launch `wlogout` directly. It still can — that is the
/// last entry — but the four things you actually do from here now take one
/// click instead of two, and the popover names its own keybinds so the menu
/// teaches you out of needing it.
pub fn session_menu(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(196);

    // (label, chord, command). Empty chord = no binding, which is deliberate
    // for the three that reboot or power the machine off.
    let items = [
        ("Lock", "SUPER L", "loginctl lock-session || hyprlock"),
        ("Log out", "SUPER \u{21E7} Q", "uwsm stop || hyprctl dispatch 'hl.dsp.exit()'"),
        ("Suspend", "", "systemctl suspend"),
        ("Reboot", "", "systemctl reboot"),
        ("Power off", "", "systemctl poweroff"),
    ];
    for (label, chord, cmd) in items {
        let row = GtkBox::new(Orientation::Horizontal, 10);
        let l = Label::new(Some(label));
        l.set_halign(Align::Start);
        l.set_hexpand(true);
        let k = Label::new(Some(chord));
        k.add_css_class("pop-mono");
        k.set_halign(Align::End);
        row.append(&l);
        row.append(&k);

        let b = Button::new();
        b.add_css_class("appmenu-item");
        b.set_child(Some(&row));
        if label == "Power off" {
            b.add_css_class("danger");
        }
        let pop_c = pop.clone();
        let cmd = cmd.to_string();
        b.connect_clicked(move |_| {
            sh(&cmd);
            pop_c.popdown();
        });
        content.append(&b);
    }

    content.append(&sep_row());
    let full = Button::with_label("All options\u{2026}");
    full.add_css_class("appmenu-item");
    if let Some(child) = full.child() {
        child.set_halign(Align::Start);
    }
    let pop_c = pop.clone();
    full.connect_clicked(move |_| {
        sh("uwsm app -- wlogout -b 4 || wlogout -b 4");
        pop_c.popdown();
    });
    content.append(&full);
    pop
}

// ── Calendar (clock) ───────────────────────────────────────────────────────

/// The clock's popover: the month, the other zones you keep, and how long the
/// machine has been up.
///
/// `zones` comes from `clock_zones` in config.toml and is usually empty, in
/// which case that section is omitted rather than guessed at. There is no
/// agenda block: nothing on this system publishes one, and a hardcoded list of
/// fake meetings would be worse than no list at all.
pub fn calendar(anchor: &impl IsA<gtk4::Widget>, zones: Rc<Vec<(String, String)>>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(238);
    let c = content.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let now = glib::DateTime::now_local().ok();

        let head = GtkBox::new(Orientation::Horizontal, 8);
        let title = pop_title(
            &now.as_ref()
                .and_then(|d| d.format("%B %Y").ok())
                .map(|s| s.to_string())
                .unwrap_or_default(),
        );
        title.set_hexpand(true);
        head.append(&title);
        if let Some(w) = now.as_ref().and_then(|d| d.format("week %V").ok()) {
            let wk = Label::new(Some(&w));
            wk.add_css_class("pop-mono");
            wk.set_valign(Align::Baseline);
            head.append(&wk);
        }
        c.append(&head);

        let cal = Calendar::new();
        cal.add_css_class("tz-cal");
        c.append(&cal);

        if !zones.is_empty() {
            c.append(&sep_row());
            c.append(&caption("other zones"));
            let rows = GtkBox::new(Orientation::Vertical, 5);
            for (label, zone) in zones.iter() {
                if let Some((time, day)) = zone_time(zone) {
                    let val = if day.is_empty() { time } else { format!("{time} \u{00B7} {day}") };
                    rows.append(&mono_row(label, &val, false));
                }
            }
            c.append(&rows);
        }

        if let Some(up) = sysinfo::uptime_secs() {
            c.append(&sep_row());
            c.append(&mono_row("uptime", &uptime_long(up), false));
        }
    });
    pop
}

/// The wall clock in `zone`, plus how its date sits relative to ours.
///
/// `date` does the zone maths — the bar carries no tz database of its own, and
/// shelling out is how every other reading in this crate is taken. The relative
/// day is computed from the day-of-year pair rather than a string compare, so
/// "tomorrow" is still right across a year boundary.
fn zone_time(zone: &str) -> Option<(String, String)> {
    let out = Command::new("date")
        .env("TZ", zone)
        .arg("+%H:%M %j")
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (time, yday) = s.trim().split_once(' ')?;
    let there: i32 = yday.trim_start_matches('0').parse().ok()?;

    let here = Command::new("date")
        .arg("+%j")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout).trim().trim_start_matches('0').parse::<i32>().ok()
        })?;

    // A wrap at new year shows up as ±364-ish; clamp it back to ±1.
    let day = match there - here {
        0 => "today",
        d if d == 1 || d < -300 => "tomorrow",
        d if d == -1 || d > 300 => "yesterday",
        _ => "",
    };
    Some((time.to_string(), day.to_string()))
}

/// `3d 7h 12m` — uptime keeps its days, unlike [`sysinfo::duration_short`],
/// which drops anything past a day because a battery estimate that long is
/// noise. An uptime that long is just an uptime.
fn uptime_long(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

// ── Audio mixer ────────────────────────────────────────────────────────────

pub fn mixer(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(258);
    let c = content.clone();
    let pop_c = pop.clone();
    pop.connect_show(move |_| {
        clear(&c);

        let head = GtkBox::new(Orientation::Horizontal, 8);
        let title = pop_title("Audio");
        title.set_hexpand(true);
        head.append(&title);
        if let Some(rate) = sample_rate(&sysinfo::audio_server().spec) {
            let l = Label::new(Some(&rate));
            l.add_css_class("pop-mono");
            l.set_valign(Align::Baseline);
            head.append(&l);
        }
        c.append(&head);

        let rows = [("Output", "@DEFAULT_AUDIO_SINK@"), ("Input", "@DEFAULT_AUDIO_SOURCE@")];
        for (label, id) in rows {
            let a = sysinfo::audio_of(id);
            let (vol, muted) = a.map(|x| (x.volume, x.muted)).unwrap_or((0, true));
            c.append(&mix_row(label, vol, muted));
        }

        // Per-application streams. Only rendered when something is actually
        // playing — an empty "Apps" heading over nothing is worse than no
        // heading, and most of the time nothing is.
        let streams = sysinfo::streams();
        if !streams.is_empty() {
            c.append(&sep_row());
            c.append(&caption("apps"));
            for st in streams {
                c.append(&mix_row(&st.name, st.volume, st.muted));
            }
        }

        // Output routing. Skipped when there is only one sink: a picker with a
        // single entry is a label pretending to be a control.
        let sinks = sysinfo::sinks();
        if sinks.len() > 1 {
            c.append(&sep_row());
            c.append(&caption("output"));
            let list = GtkBox::new(Orientation::Vertical, 2);
            for s in sinks {
                let b = device_row(&s.name, s.default);
                let pop = pop_c.clone();
                let id = s.id;
                b.connect_clicked(move |_| {
                    sysinfo::set_default_sink(id);
                    pop.popdown();
                });
                list.append(&b);
            }
            c.append(&list);
        }

        if let Some(input) = sysinfo::sources().into_iter().find(|d| d.default) {
            c.append(&sep_row());
            c.append(&caption("input"));
            c.append(&mono_row(&input.name, "default", false));
        }
    });
    pop
}

/// `float32le 2ch 48000Hz` → `48 kHz`, `… 44100Hz` → `44.1 kHz`.
///
/// The bit format and channel count are the two parts of the spec nobody reads
/// off a bar; the rate is the one that tells you whether the device you think
/// you are on is the device you are on.
fn sample_rate(spec: &str) -> Option<String> {
    let hz: f64 = spec.split_whitespace().find_map(|t| t.strip_suffix("Hz"))?.parse().ok()?;
    let khz = hz / 1000.0;
    let text = format!("{khz:.1}");
    Some(format!("{} kHz", text.strip_suffix(".0").unwrap_or(&text)))
}

/// One selectable device in the output list — a filled dot when it is the one
/// currently in use, a hollow ring when clicking would switch to it.
fn device_row(name: &str, selected: bool) -> Button {
    let row = GtkBox::new(Orientation::Horizontal, 9);
    let dot = GtkBox::new(Orientation::Horizontal, 0);
    dot.add_css_class(if selected { "dot-on" } else { "dot-off" });
    dot.set_valign(Align::Center);
    let l = Label::new(Some(name));
    l.set_halign(Align::Start);
    l.set_hexpand(true);
    l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    l.set_max_width_chars(26);
    row.append(&dot);
    row.append(&l);

    let b = Button::new();
    b.add_css_class("appmenu-item");
    if selected {
        b.add_css_class("selected");
    }
    b.set_child(Some(&row));
    b
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

pub fn network(
    anchor: &impl IsA<gtk4::Widget>,
    tp: Rc<RefCell<Throughput>>,
    history: Rc<RefCell<std::collections::VecDeque<(f64, f64)>>>,
) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(230);
    let content_c = content.clone();
    let pop_c = pop.clone();
    pop.connect_show(move |_| {
        clear(&content_c);
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

        // The throughput trace, scaled to its own peak. Both directions share
        // one scale so an upload reads at its true size beside a download.
        if let Some((chart, peak)) = throughput_chart(&history.borrow()) {
            content_c.append(&caption(&format!("last 48 s \u{00B7} peak {peak}")));
            content_c.append(&chart);
        }

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

        // Actions. The popover deliberately stops short of joining a *new*
        // secured network: that needs a password field, a scan that can take
        // seconds, and somewhere to report "wrong password" — none of which
        // belong in a layer-shell popover with no keyboard focus. It offers the
        // two things that are one call away, and hands off for the rest.
        let actions = GtkBox::new(Orientation::Horizontal, 6);
        let radio_on = wifi_radio_on();
        let toggle = Button::with_label(if radio_on { "Wi-Fi off" } else { "Wi-Fi on" });
        toggle.add_css_class("pop-item");
        toggle.set_hexpand(true);
        {
            let pop = pop_c.clone();
            toggle.connect_clicked(move |_| {
                sh(if radio_on { "tezca net radio off" } else { "tezca net radio on" });
                pop.popdown();
            });
        }
        let manage = Button::with_label("Networks…");
        manage.add_css_class("pop-item");
        {
            let pop = pop_c.clone();
            manage.connect_clicked(move |_| {
                sh("tezca settings --page network");
                pop.popdown();
            });
        }
        actions.append(&toggle);
        actions.append(&manage);
        content_c.append(&actions);
    });
    pop
}

/// The throughput chart: one column per sample, download over upload, plus the
/// peak the columns are scaled against.
///
/// Returns `None` until there are two samples — a single column, or a flat run
/// of zeroes with no peak to scale against, reads as "the link is idle" when it
/// actually means "the bar has not been up long enough to say".
fn throughput_chart(history: &std::collections::VecDeque<(f64, f64)>) -> Option<(GtkBox, String)> {
    if history.len() < 2 {
        return None;
    }
    let peak = history.iter().fold(0.0f64, |m, (d, u)| m.max(*d).max(*u));
    if peak <= 0.0 {
        return None;
    }

    let strip = GtkBox::new(Orientation::Horizontal, 1);
    strip.set_homogeneous(true);
    for (down, up) in history.iter() {
        // Down above, up below, sharing the column and mirrored about the line
        // between them: each grows away from the centre, so the two directions
        // are read as one shape rather than two charts that happen to be
        // stacked. `inverted` is what anchors each half to that centre line.
        let col = GtkBox::new(Orientation::Vertical, 1);
        for (v, class, from_centre) in [(*down, "net-down", false), (*up, "net-up", true)] {
            let bar = LevelBar::builder()
                .mode(gtk4::LevelBarMode::Continuous)
                .min_value(0.0)
                .max_value(1.0)
                .value((v / peak).clamp(0.0, 1.0))
                .orientation(Orientation::Vertical)
                .inverted(from_centre)
                .hexpand(true)
                .build();
            bar.add_css_class("core");
            bar.add_css_class(class);
            bar.set_size_request(-1, 14);
            col.append(&bar);
        }
        strip.append(&col);
    }
    // Megabytes, matching the strip's own `↓12.4 ↑1.1 MB/s` — two units for one
    // measurement would make the chart look like it disagreed with the module.
    Some((strip, format!("{:.1} MB/s", peak / 8.0)))
}

/// Whether NetworkManager reports the Wi-Fi radio as enabled.
fn wifi_radio_on() -> bool {
    Command::new("nmcli")
        .args(["radio", "wifi"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false)
}

// ── Bluetooth ───────────────────────────────────────────────────────────────

/// Connected devices with battery, plus a power toggle and a way into Settings.
///
/// Deliberately not a device manager: pairing needs a scan, a list that changes
/// under you, and somewhere to report failure — all of which the Settings page
/// does properly. This popover answers "what am I connected to, and how much
/// battery does it have left", and hands off for anything more.
pub fn bluetooth(anchor: &impl IsA<gtk4::Widget>) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(230);
    let content_c = content.clone();
    let pop_c = pop.clone();
    pop.connect_show(move |_| {
        clear(&content_c);
        let state = crate::bluetooth::poll();

        let head = GtkBox::new(Orientation::Horizontal, 8);
        let title = pop_title("Bluetooth");
        title.set_hexpand(true);
        head.append(&title);
        if state.powered {
            let chip = Label::new(Some("on"));
            chip.add_css_class("chip-connected");
            head.append(&chip);
        }
        content_c.append(&head);

        if !state.powered {
            let l = Label::new(Some("Turn Bluetooth on to connect a device."));
            l.add_css_class("pop-caption");
            l.set_wrap(true);
            l.set_xalign(0.0);
            content_c.append(&l);
        } else if state.connected.is_empty() {
            let l = Label::new(Some("Nothing connected."));
            l.add_css_class("pop-caption");
            l.set_xalign(0.0);
            content_c.append(&l);
        } else {
            let rows = GtkBox::new(Orientation::Vertical, 7);
            for d in &state.connected {
                match d.battery {
                    Some(b) => rows.append(&meter_row(&d.name, &format!("{b}%"), b as f64 / 100.0)),
                    None => rows.append(&mono_row(&d.name, "connected", false)),
                }
            }
            content_c.append(&rows);
        }

        // Paired-but-idle devices and the adapter, both of which cost an extra
        // `bluetoothctl` round trip each — fine here, far too much on the tick.
        if state.powered {
            let d = crate::bluetooth::detail();
            if !d.paired.is_empty() {
                content_c.append(&sep_row());
                content_c.append(&caption("paired, not connected"));
                let rows = GtkBox::new(Orientation::Vertical, 4);
                for (name, kind) in &d.paired {
                    rows.append(&mono_row(name, kind, false));
                }
                content_c.append(&rows);
            }
            if let Some((alias, version)) = d.adapter {
                let val =
                    if version.is_empty() { alias } else { format!("{alias} \u{00B7} {version}") };
                content_c.append(&sep_row());
                content_c.append(&mono_row("adapter", &val, false));
            }
        }

        let actions = GtkBox::new(Orientation::Horizontal, 6);
        let toggle = Button::with_label(if state.powered { "Turn off" } else { "Turn on" });
        toggle.add_css_class("pop-item");
        toggle.set_hexpand(true);
        {
            let pop = pop_c.clone();
            let on = state.powered;
            toggle.connect_clicked(move |_| {
                // Through the CLI, so the bar and the Settings page drive the
                // same code path — and so this stays a one-liner.
                sh(if on { "tezca bt power off" } else { "tezca bt power on" });
                pop.popdown();
            });
        }
        let manage = Button::with_label("Devices…");
        manage.add_css_class("pop-item");
        {
            let pop = pop_c.clone();
            manage.connect_clicked(move |_| {
                sh("tezca settings --page network");
                pop.popdown();
            });
        }
        actions.append(&toggle);
        actions.append(&manage);
        content_c.append(&actions);
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
    content.set_width_request(268);
    let c = content.clone();
    // Per-core deltas need a previous sample, and this meter only advances
    // while a popover is open — so the very first open has nothing to diff.
    let cores = Rc::new(RefCell::new(sysinfo::CoreMeter::default()));
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

        // Prime the meter, then fill the grid a moment later once there is a
        // real delta. Sampling twice back to back would divide by a zero
        // interval and paint every core idle.
        //
        // The cells are built *now*, at their final size, and only their values
        // arrive late — see [`core_grid`] for why the popover must not grow
        // after it is open.
        cores.borrow_mut().sample();
        if let Some((grid, cells)) = core_grid(d.threads) {
            c.append(&caption("per core"));
            c.append(&grid);
            let cores_c = cores.clone();
            glib::timeout_add_local_once(SAMPLE_GAP, move || {
                for (cell, v) in cells.iter().zip(cores_c.borrow_mut().sample()) {
                    cell.set_value(v);
                }
            });
        }

        append_top_processes(&c, Rank::Cpu);
    });
    pop
}

/// How long to wait before the second sample of anything that needs a rate.
///
/// Long enough that the delta is not dominated by scheduling noise, short
/// enough that the popover does not visibly fill in twice.
const SAMPLE_GAP: std::time::Duration = std::time::Duration::from_millis(450);

/// Which figure the process list ranks and reports.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rank {
    Cpu,
    Mem,
}

/// Append a "top processes" block to `c`.
///
/// Memory is instantaneous, so it renders at once. CPU is a rate and needs two
/// samples [`SAMPLE_GAP`] apart, so those figures land a moment later rather
/// than showing cumulative time, which would rank whatever has been running
/// longest instead of whatever is busy now.
///
/// Either way the rows exist from the start and are only *written* late, for
/// the reason spelled out on [`core_grid`]: a popover that grows after it has
/// been mapped gets dismissed out from under the user.
fn append_top_processes(c: &GtkBox, rank: Rank) {
    c.append(&sep_row());
    c.append(&caption("top processes"));
    let (holder, slots) = proc_rows();
    c.append(&holder);

    if rank == Rank::Mem {
        let mut procs = sysinfo::processes();
        procs.sort_unstable_by_key(|p| std::cmp::Reverse(p.rss_kb));
        let rows: Vec<_> = procs
            .iter()
            .take(TOP_N)
            .map(|p| (p.name.clone(), p.pid, format!("{:.1}G", p.rss_kb as f64 / 1024.0 / 1024.0)))
            .collect();
        fill_proc_rows(&slots, &rows);
        return;
    }

    let before: HashMap<u32, u64> =
        sysinfo::processes().into_iter().map(|p| (p.pid, p.cpu_jiffies)).collect();
    glib::timeout_add_local_once(SAMPLE_GAP, move || {
        let mut deltas: Vec<(String, u32, u64)> = sysinfo::processes()
            .into_iter()
            .filter_map(|p| {
                let prev = before.get(&p.pid)?;
                Some((p.name, p.pid, p.cpu_jiffies.saturating_sub(*prev)))
            })
            .filter(|(_, _, d)| *d > 0)
            .collect();
        deltas.sort_unstable_by_key(|d| std::cmp::Reverse(d.2));
        // USER_HZ is 100 on Linux, so a jiffy is 10ms; over the sample window
        // that converts a delta straight to a percentage of one core.
        let window_ms = SAMPLE_GAP.as_millis() as f64;
        let rows: Vec<_> = deltas
            .into_iter()
            .take(TOP_N)
            .map(|(n, pid, d)| (n, pid, format!("{:.0}%", d as f64 * 10.0 / window_ms * 100.0)))
            .collect();
        fill_proc_rows(&slots, &rows);
    });
}

/// How many processes the list shows. Four fits without scrolling and is about
/// as many as anyone reads before deciding what to kill.
const TOP_N: usize = 4;

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
        append_top_processes(&c, Rank::Mem);
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
                // Our own minimum-interval floor, not the endpoint refusing us —
                // worth wording differently so "rate limited" keeps meaning that.
                ai::Status::Cooldown { until } => {
                    Some(format!("polled recently · next check in {}", ai::until(*until)))
                }
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
                    sh("uwsm app -- alacritty --class tezca-ai -e claude \
                        || alacritty --class tezca-ai -e claude");
                });
                c.append(&b);
            }

            if let Some(l) = &p.local {
                let rows = GtkBox::new(Orientation::Vertical, 7);
                rows.append(&mono_row(
                    "today",
                    &format!("{} tok", ai::compact_count(l.total_tokens())),
                    false,
                ));
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
/// Weather detail: the conditions, the next few hours, and the numbers that
/// only matter once you have decided to go outside.
///
/// Reads a shared snapshot rather than fetching: the poll thread owns the
/// network, and opening a popover must never be able to start a request.
pub fn weather_detail(
    anchor: &impl IsA<gtk4::Widget>,
    state: Rc<RefCell<crate::weather::Snapshot>>,
) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(268);
    let c = content.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let s = state.borrow();
        c.append(&pop_title(if s.place.is_empty() { "Weather" } else { &s.place }));

        if let Some(e) = &s.error {
            c.append(&caption(e));
            return;
        }

        // The headline: temperature large, everything qualifying it small
        // beside it, because the number is what you came for.
        let head = GtkBox::new(Orientation::Horizontal, 12);
        head.set_valign(Align::End);
        let big = Label::new(Some(&s.temp_text()));
        big.add_css_class("pop-big");
        big.add_css_class("tz-xl");
        head.append(&big);
        let col = GtkBox::new(Orientation::Vertical, 2);
        col.set_valign(Align::End);
        if let Some(code) = s.code {
            let l = Label::new(Some(crate::weather::condition(code, s.is_day)));
            l.add_css_class("pop-sub");
            l.set_halign(Align::Start);
            col.append(&l);
        }
        let mut qual = Vec::new();
        if let Some(f) = s.feels_c {
            qual.push(format!("feels {}", s.degrees(f)));
        }
        let range = s.range_text();
        if !range.is_empty() {
            qual.push(range);
        }
        if !qual.is_empty() {
            let l = Label::new(Some(&qual.join(" · ")));
            l.add_css_class("pop-mono");
            l.set_halign(Align::Start);
            col.append(&l);
        }
        head.append(&col);
        c.append(&head);

        if !s.hourly.is_empty() {
            c.append(&sep_row());
            let strip = GtkBox::new(Orientation::Horizontal, 0);
            strip.set_homogeneous(true);
            for h in &s.hourly {
                let cell = GtkBox::new(Orientation::Vertical, 5);
                for (text, class) in
                    [(h.label.clone(), "pop-mono"), (s.degrees(h.temp_c), "pop-mono-val")]
                {
                    let l = Label::new(Some(&text));
                    l.add_css_class(class);
                    cell.append(&l);
                }
                strip.append(&cell);
            }
            c.append(&strip);
        }

        c.append(&sep_row());
        let rows = GtkBox::new(Orientation::Vertical, 7);
        if let Some(h) = s.humidity {
            rows.append(&mono_row("humidity", &format!("{h:.0}%"), false));
        }
        if let Some(w) = s.wind_kmh {
            let dir = s.wind_dir_deg.map(crate::weather::bearing).unwrap_or("");
            rows.append(&mono_row("wind", format!("{w:.0} km/h {dir}").trim_end(), false));
        }
        if let Some(u) = s.uv {
            rows.append(&mono_row("uv index", &format!("{u:.0}"), false));
        }
        if let Some(a) = s.aqi {
            rows.append(&mono_row(
                "aqi",
                &format!("{a:.0} · {}", crate::weather::aqi_band(a)),
                false,
            ));
        }
        if let Some(t) = &s.sunset {
            rows.append(&mono_row("sunset", t, false));
        }
        if s.updated > 0 {
            rows.append(&mono_row("updated", &ai::ago(s.updated), false));
        }
        c.append(&rows);
    });
    pop
}

/// Battery detail: charge, what it is doing, and how well the cell is holding
/// up. `history` is the charge trace the bar has recorded since it started.
///
/// There is no "biggest consumers" list: per-process power attribution needs
/// powertop's kernel accounting, and a plausible-looking guess assembled from
/// CPU time would be a number that reads as measured and is not.
pub fn battery_detail(
    anchor: &impl IsA<gtk4::Widget>,
    history: Rc<RefCell<std::collections::VecDeque<f64>>>,
) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(250);
    let c = content.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let Some(b) = sysinfo::battery() else {
            c.append(&pop_title("Battery"));
            c.append(&caption("no battery on this machine"));
            return;
        };
        let d = sysinfo::battery_detail().unwrap_or_default();
        c.append(&pop_title(if d.model.is_empty() { "Battery" } else { &d.model }));

        let head = GtkBox::new(Orientation::Horizontal, 12);
        head.set_valign(Align::End);
        let big = Label::new(Some(&format!("{}%", b.percent)));
        big.add_css_class("pop-big");
        head.append(&big);
        let sub = match b.secs_remaining.map(sysinfo::duration_short) {
            Some(t) if !t.is_empty() => {
                format!("{t} {}", if b.charging { "to full" } else { "remaining" })
            }
            _ => d.status.to_lowercase(),
        };
        let l = Label::new(Some(&sub));
        l.add_css_class("pop-mono");
        l.set_valign(Align::End);
        head.append(&l);
        c.append(&head);

        // The trace only covers this session — the bar keeps no state across
        // restarts — so it is labelled for what it is rather than "24h".
        let hist = history.borrow();
        if hist.len() >= 2 {
            c.append(&caption("since the bar started"));
            let strip = GtkBox::new(Orientation::Horizontal, 1);
            strip.set_homogeneous(true);
            for v in hist.iter() {
                let bar = LevelBar::builder()
                    .mode(gtk4::LevelBarMode::Continuous)
                    .min_value(0.0)
                    .max_value(1.0)
                    .value(*v)
                    .orientation(Orientation::Vertical)
                    .inverted(true)
                    .hexpand(true)
                    .build();
                bar.add_css_class("core");
                bar.set_size_request(-1, 30);
                strip.append(&bar);
            }
            c.append(&strip);
        }
        drop(hist);

        c.append(&sep_row());
        let rows = GtkBox::new(Orientation::Vertical, 7);
        if let Some(w) = d.power_w {
            rows.append(&mono_row("draw", &format!("{w:.1} W"), false));
        }
        if let Some(h) = d.health_pct {
            rows.append(&mono_row("health", &format!("{h:.0}%"), false));
        }
        if let Some((now, design)) = d.capacity_wh {
            rows.append(&mono_row("capacity", &format!("{now:.1} / {design:.1} Wh"), false));
        }
        if let Some(cy) = d.cycles {
            rows.append(&mono_row("cycles", &cy.to_string(), false));
        }
        if let Some(t) = d.temp_c {
            rows.append(&mono_row("temperature", &format!("{t:.0} \u{00B0}C"), false));
        }
        c.append(&rows);
    });
    pop
}

// ── Now playing ────────────────────────────────────────────────────────────

/// The media pill's popover: cover art, the full metadata, a progress trace and
/// the transport.
///
/// No play queue. MPRIS keeps its track list behind the optional `TrackList`
/// interface and virtually nothing implements it, so the "up next" list in the
/// design has no source on this system — a fabricated one would be the only way
/// to draw it.
pub fn nowplaying_detail(anchor: &impl IsA<gtk4::Widget>, pal: &SharedPalette) -> Popover {
    let (pop, content) = glass(anchor);
    content.set_width_request(272);
    let c = content.clone();
    let pal = pal.clone();
    pop.connect_show(move |_| {
        clear(&c);
        let Some(np) = crate::nowplaying::current() else {
            c.append(&pop_title("Nothing playing"));
            return;
        };
        let d = crate::nowplaying::detail();

        // Head: art beside title / artist / album.
        let head = GtkBox::new(Orientation::Horizontal, 12);
        let art = GtkBox::new(Orientation::Horizontal, 0);
        art.add_css_class("np-art-lg");
        if let Some(tex) = crate::nowplaying::art_texture(&np.art_url) {
            let pic = gtk4::Picture::for_paintable(&tex);
            pic.set_content_fit(gtk4::ContentFit::Cover);
            pic.set_size_request(58, 58);
            art.append(&pic);
        }
        head.append(&art);
        let text = GtkBox::new(Orientation::Vertical, 3);
        text.set_valign(Align::Center);
        text.append(&pop_title(&np.title));
        if !np.artist.is_empty() {
            text.append(&caption(&np.artist));
        }
        if !d.album.is_empty() {
            text.append(&caption(&d.album));
        }
        head.append(&text);
        c.append(&head);

        // Progress. A stream has no length, so it gets its elapsed time alone
        // rather than a bar that would sit at zero forever.
        if let (Some(pos), Some(len)) = (np.position, np.length) {
            let bar = LevelBar::builder()
                .mode(gtk4::LevelBarMode::Continuous)
                .min_value(0.0)
                .max_value(1.0)
                .value((pos as f64 / len.max(1) as f64).clamp(0.0, 1.0))
                .hexpand(true)
                .build();
            bar.add_css_class("mix");
            c.append(&bar);
            let times = GtkBox::new(Orientation::Horizontal, 8);
            let a = Label::new(Some(&crate::nowplaying::clock(pos)));
            a.add_css_class("pop-mono");
            a.set_hexpand(true);
            a.set_halign(Align::Start);
            let b = Label::new(Some(&crate::nowplaying::clock(len)));
            b.add_css_class("pop-mono");
            b.set_halign(Align::End);
            times.append(&a);
            times.append(&b);
            c.append(&times);
        } else if let Some(pos) = np.position {
            c.append(&mono_row("elapsed", &crate::nowplaying::clock(pos), false));
        }

        // Transport.
        let transport = GtkBox::new(Orientation::Horizontal, 18);
        transport.set_halign(Align::Center);
        let button = |kind: Icon| {
            let b = Button::new();
            b.add_css_class("np-transport");
            b.set_child(Some(&icon::icon(&pal, kind).area));
            b
        };
        let prev = button(Icon::TransportPrev);
        prev.connect_clicked(|_| crate::nowplaying::previous());
        let play = button(if np.playing { Icon::TransportPause } else { Icon::TransportPlay });
        play.add_css_class("primary");
        play.connect_clicked(|_| crate::nowplaying::play_pause());
        let next = button(Icon::TransportNext);
        next.connect_clicked(|_| crate::nowplaying::next());
        transport.append(&prev);
        transport.append(&play);
        transport.append(&next);
        c.append(&transport);

        // Shuffle / repeat, as two-state chips.
        let modes = GtkBox::new(Orientation::Horizontal, 6);
        let shuffle = Button::with_label("shuffle");
        shuffle.add_css_class("pop-chip");
        if d.shuffle {
            shuffle.add_css_class("on");
        }
        shuffle.connect_clicked(|_| crate::nowplaying::toggle_shuffle());
        let repeat_on = matches!(d.loop_status.trim(), "Track" | "Playlist");
        let repeat_label = match d.loop_status.trim() {
            "Track" => "repeat one",
            "Playlist" => "repeat all",
            _ => "repeat",
        };
        let repeat = Button::with_label(repeat_label);
        repeat.add_css_class("pop-chip");
        if repeat_on {
            repeat.add_css_class("on");
        }
        let loop_now = d.loop_status.clone();
        repeat.connect_clicked(move |_| {
            crate::nowplaying::cycle_loop(&loop_now);
        });
        modes.append(&shuffle);
        modes.append(&repeat);
        c.append(&modes);

        if !d.player.is_empty() {
            c.append(&sep_row());
            c.append(&mono_row("player", &d.player, false));
        }
    });
    pop
}

/// An empty grid of one cell per logical core, plus the cells to fill in.
///
/// Eight per row, which is the widest that stays legible at popover width and
/// happens to halve a 16-thread desktop neatly. `None` on a machine that does
/// not report a thread count, rather than an empty box holding a caption open.
///
/// Built empty and filled afterwards because **a popover must not change size
/// while it is open**: this grid used to be appended when its first delta
/// landed, 450ms after the click, and the resize that caused had the compositor
/// dismiss the popup — the CPU popover closed itself, every time, a blink after
/// you opened it. Nothing else on the bar defers content, which is why nothing
/// else did it. Cells start at zero and are written once the delta arrives, so
/// the geometry the popup was mapped with is the geometry it keeps.
fn core_grid(threads: usize) -> Option<(GtkBox, Vec<LevelBar>)> {
    if threads == 0 {
        return None;
    }
    let col = GtkBox::new(Orientation::Vertical, 3);
    col.add_css_class("core-grid");
    let mut cells = Vec::with_capacity(threads);
    for chunk in 0..threads.div_ceil(8) {
        let row = GtkBox::new(Orientation::Horizontal, 3);
        row.set_homogeneous(true);
        for _ in 0..(threads - chunk * 8).min(8) {
            let bar = LevelBar::builder()
                .mode(gtk4::LevelBarMode::Continuous)
                .min_value(0.0)
                .max_value(1.0)
                .value(0.0)
                .orientation(Orientation::Vertical)
                .inverted(true)
                .hexpand(true)
                .build();
            bar.add_css_class("core");
            bar.set_size_request(-1, 26);
            row.append(&bar);
            cells.push(bar);
        }
        col.append(&row);
    }
    Some((col, cells))
}

/// The `top processes` block: [`TOP_N`] empty rows of name, pid and figure,
/// returned with the labels to write into.
///
/// Every column is held to a character width rather than sized to its content,
/// so filling a row in — which for the CPU list happens [`SAMPLE_GAP`] after the
/// popover opened — cannot change the popover's width. See [`core_grid`].
fn proc_rows() -> (GtkBox, Vec<(Label, Label, Label)>) {
    let b = GtkBox::new(Orientation::Vertical, 5);
    let mut slots = Vec::with_capacity(TOP_N);
    for _ in 0..TOP_N {
        let row = GtkBox::new(Orientation::Horizontal, 8);
        let n = Label::new(None);
        n.add_css_class("pop-mono-val");
        n.set_halign(Align::Start);
        n.set_hexpand(true);
        n.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        n.set_width_chars(18);
        n.set_max_width_chars(18);
        let p = Label::new(None);
        p.add_css_class("pop-mono");
        p.set_width_chars(PID_CHARS);
        p.set_xalign(1.0);
        let v = Label::new(None);
        v.add_css_class("pop-mono-val");
        v.set_halign(Align::End);
        v.set_width_chars(VAL_CHARS);
        v.set_xalign(1.0);
        row.append(&n);
        row.append(&p);
        row.append(&v);
        b.append(&row);
        slots.push((n, p, v));
    }
    (b, slots)
}

/// Widest a pid gets (`/proc/sys/kernel/pid_max` is 4194304 by default) and
/// widest a figure gets (`100%`, `12.3G`).
const PID_CHARS: i32 = 7;
const VAL_CHARS: i32 = 5;

/// Write rows into the slots [`proc_rows`] reserved, blanking any left over —
/// a machine with fewer busy processes than slots keeps its layout rather than
/// leaving whatever the last open put there.
fn fill_proc_rows(slots: &[(Label, Label, Label)], rows: &[(String, u32, String)]) {
    for (i, (n, p, v)) in slots.iter().enumerate() {
        match rows.get(i) {
            Some((name, pid, val)) => {
                n.set_text(name);
                p.set_text(&pid.to_string());
                v.set_text(val);
            }
            None => {
                n.set_text("");
                p.set_text("");
                v.set_text("");
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::{sample_rate, uptime_long};

    #[test]
    fn uptime_keeps_its_days_where_a_battery_estimate_would_drop_them() {
        assert_eq!(uptime_long(285_120), "3d 7h 12m");
        assert_eq!(uptime_long(13_200), "3h 40m");
        assert_eq!(uptime_long(420), "7m");
    }

    #[test]
    fn a_whole_rate_loses_its_decimal_and_a_fractional_one_keeps_it() {
        assert_eq!(sample_rate("float32le 2ch 48000Hz").as_deref(), Some("48 kHz"));
        assert_eq!(sample_rate("s16le 2ch 44100Hz").as_deref(), Some("44.1 kHz"));
        assert_eq!(sample_rate("s24le 2ch 192000Hz").as_deref(), Some("192 kHz"));
        assert_eq!(sample_rate(""), None);
    }
}
