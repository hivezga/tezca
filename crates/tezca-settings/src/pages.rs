//! The control-center pages. Every real action shells out through `backend` to
//! the `tezca` CLI, so the GUI and the keyboard/CLI paths drive identical code.
//!
//! Convention across controls: set the widget's value FIRST, then connect its
//! handler — so populating a control never fires an apply. Pages that can be
//! "reset" (Desktop) rebuild their rows the same way, so no signal-blocking is
//! needed anywhere.

use crate::{backend, keybinds};
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box, Button, ContentFit, DropDown, Entry, EventControllerKey, FileDialog, FlowBox,
    Label, Orientation, Picture, PolicyType, Scale, ScrolledWindow, SelectionMode, SpinButton,
    Switch, Widget, Window,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

// ===========================================================================
// Appearance — theme + global (palette) wallpaper
// ===========================================================================

pub fn appearance(window: &Window) -> Widget {
    let page = page_box();

    page.append(&section_header("Theme"));
    let grid = FlowBox::new();
    grid.set_selection_mode(SelectionMode::None);
    grid.set_max_children_per_line(4);
    grid.set_column_spacing(10);
    grid.set_row_spacing(10);
    grid.set_halign(Align::Start);
    grid.add_css_class("tz-theme-grid");

    let active = backend::active_theme();
    let mut buttons: Vec<(String, Button)> = Vec::new();
    if let Some(names) = backend::tezca_out(&["theme", "names"]) {
        for name in names.lines() {
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            let btn = Button::with_label(&capitalize(&name));
            btn.add_css_class("tz-theme");
            if active.as_deref() == Some(name.as_str()) {
                btn.add_css_class("tz-active");
            }
            buttons.push((name, btn));
        }
    }
    let all: Rc<Vec<Button>> = Rc::new(buttons.iter().map(|(_, b)| b.clone()).collect());
    for (name, btn) in &buttons {
        let name = name.clone();
        let all = all.clone();
        let me = btn.clone();
        btn.connect_clicked(move |_| {
            backend::tezca(&["theme", "set", &name]);
            for b in all.iter() {
                b.remove_css_class("tz-active");
            }
            me.add_css_class("tz-active");
        });
        grid.append(btn);
    }
    page.append(&grid);
    page.append(&hint(
        "Curated palettes. Switching re-skins the bar, kitty, the dock, hyprlock and the launcher live — no restart.",
    ));

    page.append(&section_header("Wallpaper"));
    let preview = Picture::new();
    preview.add_css_class("tz-wallpreview");
    preview.set_size_request(360, 150);
    preview.set_content_fit(ContentFit::Cover);
    preview.set_halign(Align::Start);
    if let Some(wp) = backend::current_wallpaper() {
        preview.set_filename(Some(&wp));
    }
    page.append(&preview);

    let row = Box::new(Orientation::Horizontal, 8);
    let choose = Button::with_label("Choose image…");
    choose.add_css_class("tz-primary");
    {
        let win = window.clone();
        let preview = preview.clone();
        choose.connect_clicked(move |_| {
            let dialog = FileDialog::builder().title("Choose wallpaper").build();
            let preview = preview.clone();
            dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        if let Some(s) = path.to_str() {
                            backend::tezca(&["theme", "wallpaper", s]);
                            preview.set_filename(Some(&path));
                        }
                    }
                }
            });
        });
    }
    let prev = Button::with_label("Previous");
    prev.connect_clicked(|_| backend::run_script("wallpaper.sh", &["prev"]));
    let next = Button::with_label("Next");
    next.connect_clicked(|_| backend::run_script("wallpaper.sh", &["next"]));
    row.append(&choose);
    row.append(&prev);
    row.append(&next);
    page.append(&row);
    page.append(&hint(
        "This wallpaper drives the whole palette (matugen). For a different picture per screen, use the Displays tab.",
    ));

    scrolled(&page)
}

// ===========================================================================
// Displays — mode / scale + brightness + per-monitor wallpaper
// ===========================================================================

pub fn displays(window: &Window) -> Widget {
    let page = page_box();
    let mons = backend::monitors();
    if mons.is_empty() {
        page.append(&hint("Could not read monitors (are you in a Hyprland session?)."));
        return scrolled(&page);
    }
    let walls = backend::wallpaper_targets();

    for m in &mons {
        page.append(&section_header(&m.name));
        page.append(&hint(&m.desc));

        // --- Mode + scale ---------------------------------------------------
        let mode_refs: Vec<&str> = m.modes.iter().map(String::as_str).collect();
        let dd = DropDown::from_strings(&mode_refs);
        let current = format!("{}@{}", m.res, m.rate);
        if let Some(i) = m.modes.iter().position(|x| *x == current) {
            dd.set_selected(i as u32);
        }
        page.append(&control_row("Resolution & refresh", &dd));

        let scale = SpinButton::with_range(0.5, 3.0, 0.05);
        scale.set_digits(2);
        scale.set_value(m.scale.parse().unwrap_or(1.0));
        page.append(&control_row("Scale", &scale));

        let apply = Button::with_label("Apply mode");
        apply.add_css_class("tz-primary");
        {
            let name = m.name.clone();
            let modes = m.modes.clone();
            let dd = dd.clone();
            let scale = scale.clone();
            apply.connect_clicked(move |_| {
                let idx = dd.selected() as usize;
                let Some(mode) = modes.get(idx) else { return };
                let sc = format!("{:.2}", scale.value());
                backend::tezca(&["display", "set", &name, "--mode", mode, "--scale", &sc]);
            });
        }
        let apply_row = Box::new(Orientation::Horizontal, 8);
        apply_row.set_halign(Align::End);
        apply_row.append(&apply);
        page.append(&apply_row);

        // --- Brightness (DDC/CI) -------------------------------------------
        match backend::brightness(&m.name) {
            Some(cur) => {
                let sl = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
                sl.set_hexpand(true);
                sl.set_draw_value(true);
                sl.set_value(cur as f64);
                debounce_scale(&sl, 300, {
                    let name = m.name.clone();
                    move |v| backend::tezca(&["display", "brightness", &name, &(v as i32).to_string()])
                });
                page.append(&control_row("Brightness", &sl));
            }
            None => {
                page.append(&hint("Brightness: no DDC/CI channel (install ddcutil / not supported)."));
            }
        }

        // --- Per-monitor wallpaper -----------------------------------------
        let (is_override, cur_path) = walls
            .iter()
            .find(|(n, _, _)| *n == m.name)
            .map(|(_, ovr, p)| (*ovr, p.clone()))
            .unwrap_or((false, String::new()));

        let wp = Picture::new();
        wp.add_css_class("tz-wallpreview");
        wp.set_size_request(300, 120);
        wp.set_content_fit(ContentFit::Cover);
        wp.set_halign(Align::Start);
        if !cur_path.is_empty() {
            wp.set_filename(Some(&cur_path));
        }
        page.append(&wp);

        let wrow = Box::new(Orientation::Horizontal, 8);
        let setw = Button::with_label("Set image…");
        {
            let win = window.clone();
            let name = m.name.clone();
            let wp = wp.clone();
            setw.connect_clicked(move |_| {
                let dialog = FileDialog::builder().title("Wallpaper for this monitor").build();
                let name = name.clone();
                let wp = wp.clone();
                dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res {
                        if let Some(path) = file.path() {
                            if let Some(s) = path.to_str() {
                                backend::tezca(&["wallpaper", "set", s, "--monitor", &name]);
                                wp.set_filename(Some(&path));
                            }
                        }
                    }
                });
            });
        }
        let resetw = Button::with_label("Reset to theme");
        {
            let name = m.name.clone();
            resetw.connect_clicked(move |_| {
                backend::tezca(&["wallpaper", "clear", "--monitor", &name]);
            });
        }
        if !is_override {
            resetw.set_sensitive(false);
        }
        wrow.append(&setw);
        wrow.append(&resetw);
        page.append(&wrow);
    }
    scrolled(&page)
}

// ===========================================================================
// Bar — the top menubar (tezca-bar): shape, clock, workspaces, metrics
// ===========================================================================

pub fn bar() -> Widget {
    let page = page_box();
    let cfg = backend::bar_config();
    let get = |k: &str| cfg.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());

    // --- Shape & geometry ---------------------------------------------------
    page.append(&section_header("Shape"));

    // floating | edge — index 0/1 into SHAPES.
    const SHAPES: [&str; 2] = ["floating", "edge"];
    let shape = DropDown::from_strings(&["Floating (rounded, inset)", "Edge (full-width)"]);
    let cur_shape = get("shape").unwrap_or_else(|| "floating".into());
    if let Some(i) = SHAPES.iter().position(|s| *s == cur_shape) {
        shape.set_selected(i as u32);
    }
    page.append(&control_row("Shape", &shape));

    let height = spin_from("height", 20.0, 80.0, 1.0, 0, &get);
    let mtop = spin_from("margin_top", 0.0, 40.0, 1.0, 0, &get);
    let mside = spin_from("margin_side", 0.0, 40.0, 1.0, 0, &get);
    page.append(&control_row("Height (px)", &height));
    page.append(&control_row("Top margin (floating)", &mtop));
    page.append(&control_row("Side margin (floating)", &mside));

    // --- Clock --------------------------------------------------------------
    page.append(&section_header("Clock"));
    let clock = Entry::new();
    clock.set_text(&get("clock_format").unwrap_or_else(|| "%a %d %b   %H:%M".into()));
    clock.set_width_chars(20);
    page.append(&control_row("Format", &clock));
    page.append(&hint("strftime-style — e.g. %a %d %b   %H:%M for “Wed 22 Jul  16:59”. See `man strftime`."));

    // --- Workspaces ---------------------------------------------------------
    page.append(&section_header("Workspaces"));

    const NUMERALS: [&str; 2] = ["arabic", "mayan"];
    let numerals = DropDown::from_strings(&["Arabic  (1 2 3)", "Mayan  (bar & dot)"]);
    let cur_num = get("workspace_numerals").unwrap_or_else(|| "arabic".into());
    if let Some(i) = NUMERALS.iter().position(|s| *s == cur_num) {
        numerals.set_selected(i as u32);
    }
    page.append(&control_row("Numerals", &numerals));

    let hide_empty = Switch::new();
    hide_empty.set_valign(Align::Center);
    hide_empty.set_active(get("workspace_hide_empty").as_deref() == Some("true"));
    page.append(&control_row("Show only used workspaces", &hide_empty));

    let compact_ws = Switch::new();
    compact_ws.set_valign(Align::Center);
    compact_ws.set_active(get("workspace_compact").as_deref() == Some("true"));
    page.append(&control_row("Auto-compact gaps", &compact_ws));
    page.append(&hint(
        "Compaction slides a monitor's workspaces down to close a gap when one you're not on empties — assign each monitor a set below.",
    ));

    // Per-monitor workspace sets (automatic / odd / even / custom list).
    let mut assign_rows: Vec<(String, DropDown, Entry)> = Vec::new();
    for m in backend::monitors() {
        let (row, dd, entry) = ws_assign_row(&m.name, get(&format!("workspaces.{}", m.name)));
        page.append(&row);
        assign_rows.push((m.name, dd, entry));
    }
    if assign_rows.is_empty() {
        page.append(&hint("No monitors detected — per-monitor sets need a live Hyprland session."));
    }
    let assign_rows = Rc::new(assign_rows);

    // --- Metrics ------------------------------------------------------------
    page.append(&section_header("Metrics"));
    let cpu_iv = spin_from("cpu_interval", 1.0, 30.0, 1.0, 0, &get);
    let mem_iv = spin_from("mem_interval", 1.0, 30.0, 1.0, 0, &get);
    let gpu_iv = spin_from("gpu_interval", 1.0, 30.0, 1.0, 0, &get);
    let net_iv = spin_from("net_interval", 1.0, 30.0, 1.0, 0, &get);
    let compact = spin_from("compact_width", 0.0, 6000.0, 100.0, 0, &get);
    page.append(&control_row("CPU poll (s)", &cpu_iv));
    page.append(&control_row("Memory poll (s)", &mem_iv));
    page.append(&control_row("GPU poll (s)", &gpu_iv));
    page.append(&control_row("Network poll (s)", &net_iv));
    page.append(&control_row("Compact below width (px)", &compact));

    // --- Apply --------------------------------------------------------------
    let apply = Button::with_label("Apply bar settings");
    apply.add_css_class("tz-primary");
    {
        let (shape, height, mtop, mside, clock) =
            (shape.clone(), height.clone(), mtop.clone(), mside.clone(), clock.clone());
        let (cpu_iv, mem_iv, gpu_iv, net_iv, compact) =
            (cpu_iv.clone(), mem_iv.clone(), gpu_iv.clone(), net_iv.clone(), compact.clone());
        let (numerals, hide_empty, compact_ws, assign_rows) =
            (numerals.clone(), hide_empty.clone(), compact_ws.clone(), assign_rows.clone());
        apply.connect_clicked(move |_| {
            // Build a flat `key value key value …` arg list (some keys — the
            // per-monitor sets — are dynamic, so a fixed array won't do).
            let mut kvs: Vec<(String, String)> = vec![
                ("shape".into(), SHAPES.get(shape.selected() as usize).copied().unwrap_or("floating").into()),
                ("height".into(), (height.value() as i64).to_string()),
                ("margin_top".into(), (mtop.value() as i64).to_string()),
                ("margin_side".into(), (mside.value() as i64).to_string()),
                ("clock_format".into(), clock.text().to_string()),
                ("workspace_numerals".into(), NUMERALS.get(numerals.selected() as usize).copied().unwrap_or("arabic").into()),
                ("workspace_hide_empty".into(), bool_str(hide_empty.is_active())),
                ("workspace_compact".into(), bool_str(compact_ws.is_active())),
                ("cpu_interval".into(), (cpu_iv.value() as i64).to_string()),
                ("mem_interval".into(), (mem_iv.value() as i64).to_string()),
                ("gpu_interval".into(), (gpu_iv.value() as i64).to_string()),
                ("net_interval".into(), (net_iv.value() as i64).to_string()),
                ("compact_width".into(), (compact.value() as i64).to_string()),
            ];
            for (name, dd, entry) in assign_rows.iter() {
                kvs.push((format!("workspaces.{name}"), ws_spec_value(dd, entry)));
            }
            let mut args: Vec<String> = vec!["bar".into(), "set".into()];
            for (k, v) in &kvs {
                args.push(k.clone());
                args.push(v.clone());
            }
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            backend::tezca(&argv);
        });
    }
    let arow = Box::new(Orientation::Horizontal, 8);
    arow.set_halign(Align::End);
    arow.append(&apply);
    page.append(&arow);
    page.append(&hint("Applying restarts the bar so the new settings take effect."));

    scrolled(&page)
}

/// A per-monitor workspace-set row: a preset dropdown (Automatic / Odd / Even /
/// Custom) with an inline list entry that lights up only for Custom. Returns the
/// row plus the two controls to read back at apply time.
fn ws_assign_row(name: &str, current: Option<String>) -> (Box, DropDown, Entry) {
    let dd = DropDown::from_strings(&["Automatic", "Odd (1 3 5…)", "Even (2 4 6…)", "Custom…"]);
    let entry = Entry::new();
    entry.set_placeholder_text(Some("1,3,5,7,9  or  1-5"));
    entry.set_width_chars(14);

    let cur = current.unwrap_or_default();
    let cur = cur.trim();
    let idx: u32 = match cur {
        "" | "auto" | "dynamic" => 0,
        "odd" => 1,
        "even" => 2,
        other => {
            entry.set_text(other);
            3
        }
    };
    dd.set_selected(idx);
    entry.set_sensitive(idx == 3);
    {
        let entry = entry.clone();
        dd.connect_selected_notify(move |d| entry.set_sensitive(d.selected() == 3));
    }

    let ctl = Box::new(Orientation::Horizontal, 8);
    ctl.append(&dd);
    ctl.append(&entry);
    (control_row(name, &ctl), dd, entry)
}

/// Read a per-monitor workspace spec back from its row's controls.
fn ws_spec_value(dd: &DropDown, entry: &Entry) -> String {
    match dd.selected() {
        1 => "odd".into(),
        2 => "even".into(),
        3 => {
            let t = entry.text().trim().to_string();
            if t.is_empty() { "auto".into() } else { t }
        }
        _ => "auto".into(),
    }
}

fn bool_str(on: bool) -> String {
    if on { "true" } else { "false" }.to_string()
}

// ===========================================================================
// Dock — geometry + pinned favourites
// ===========================================================================

pub fn dock() -> Widget {
    let page = page_box();
    let cfg = backend::dock_config();
    let get = |k: &str| cfg.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());

    page.append(&section_header("Feel"));

    let icon = spin_from("icon_size", 16.0, 128.0, 1.0, 0, &get);
    let scale = spin_from("max_scale", 1.0, 3.0, 0.1, 1, &get);
    let infl = spin_from("influence", 40.0, 260.0, 5.0, 0, &get);
    let gap = spin_from("gap", 0.0, 40.0, 1.0, 0, &get);
    let margin = spin_from("margin_bottom", 0.0, 40.0, 1.0, 0, &get);
    let delay = spin_from("hide_delay_ms", 0.0, 1200.0, 50.0, 0, &get);

    page.append(&control_row("Icon size", &icon));
    page.append(&control_row("Magnification", &scale));
    page.append(&control_row("Magnify radius", &infl));
    page.append(&control_row("Icon gap", &gap));
    page.append(&control_row("Bottom margin", &margin));
    page.append(&control_row("Autohide delay (ms)", &delay));

    let apply = Button::with_label("Apply dock geometry");
    apply.add_css_class("tz-primary");
    {
        let (icon, scale, infl, gap, margin, delay) =
            (icon.clone(), scale.clone(), infl.clone(), gap.clone(), margin.clone(), delay.clone());
        apply.connect_clicked(move |_| {
            let icon_s = (icon.value() as i64).to_string();
            let scale_s = format!("{:.1}", scale.value());
            let infl_s = (infl.value() as i64).to_string();
            let gap_s = (gap.value() as i64).to_string();
            let margin_s = (margin.value() as i64).to_string();
            let delay_s = (delay.value() as i64).to_string();
            backend::tezca(&[
                "dock", "set",
                "icon_size", &icon_s,
                "max_scale", &scale_s,
                "influence", &infl_s,
                "gap", &gap_s,
                "margin_bottom", &margin_s,
                "hide_delay_ms", &delay_s,
            ]);
        });
    }
    let arow = Box::new(Orientation::Horizontal, 8);
    arow.set_halign(Align::End);
    arow.append(&apply);
    page.append(&arow);
    page.append(&hint("Applying restarts the dock (seamless — it's autohidden)."));

    // --- Pinned favourites -------------------------------------------------
    page.append(&section_header("Pinned favourites"));
    let pinned: Vec<String> = get("pinned")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let state = Rc::new(RefCell::new(pinned));

    let list = Box::new(Orientation::Vertical, 4);
    list.add_css_class("tz-pinlist");
    page.append(&list);
    rebuild_pinned(&list, &state);

    let addrow = Box::new(Orientation::Horizontal, 8);
    let entry = Entry::new();
    entry.set_placeholder_text(Some("app id or window class (e.g. org.kde.dolphin)"));
    entry.set_hexpand(true);
    let add = Button::with_label("Add");
    add.add_css_class("tz-primary");
    {
        let state = state.clone();
        let list = list.clone();
        let entry2 = entry.clone();
        let doit = move || {
            let t = entry2.text().trim().to_string();
            if t.is_empty() {
                return;
            }
            state.borrow_mut().push(t);
            entry2.set_text("");
            rebuild_pinned(&list, &state);
            save_pinned(&state);
        };
        let d2 = doit.clone();
        add.connect_clicked(move |_| d2());
        entry.connect_activate(move |_| doit());
    }
    addrow.append(&entry);
    addrow.append(&add);
    page.append(&addrow);
    page.append(&hint("Drag order isn't here yet — use the arrows. Click an icon in the dock to launch or focus it."));

    scrolled(&page)
}

fn rebuild_pinned(list: &Box, state: &Rc<RefCell<Vec<String>>>) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let items = state.borrow().clone();
    let n = items.len();
    for (i, app) in items.into_iter().enumerate() {
        let row = Box::new(Orientation::Horizontal, 8);
        row.add_css_class("tz-pinrow");
        let name = Label::new(Some(&app));
        name.set_halign(Align::Start);
        name.set_hexpand(true);
        name.set_xalign(0.0);
        row.append(&name);

        let up = small_btn("↑");
        up.set_sensitive(i > 0);
        let down = small_btn("↓");
        down.set_sensitive(i + 1 < n);
        let rm = small_btn("✕");
        rm.add_css_class("tz-danger");

        {
            let (s, l) = (state.clone(), list.clone());
            up.connect_clicked(move |_| {
                s.borrow_mut().swap(i, i - 1);
                rebuild_pinned(&l, &s);
                save_pinned(&s);
            });
        }
        {
            let (s, l) = (state.clone(), list.clone());
            down.connect_clicked(move |_| {
                s.borrow_mut().swap(i, i + 1);
                rebuild_pinned(&l, &s);
                save_pinned(&s);
            });
        }
        {
            let (s, l) = (state.clone(), list.clone());
            rm.connect_clicked(move |_| {
                s.borrow_mut().remove(i);
                rebuild_pinned(&l, &s);
                save_pinned(&s);
            });
        }
        row.append(&up);
        row.append(&down);
        row.append(&rm);
        list.append(&row);
    }
}

fn save_pinned(state: &Rc<RefCell<Vec<String>>>) {
    let csv = state.borrow().join(",");
    backend::tezca(&["dock", "set", "pinned", &csv]);
}

// ===========================================================================
// Desktop — live Hyprland look & feel (persisted)
// ===========================================================================

pub fn desktop() -> Widget {
    let page = page_box();
    page.append(&section_header("Look & feel"));
    let container = Box::new(Orientation::Vertical, 0);
    populate_desktop(&container);
    page.append(&container);

    page.append(&section_header("Reset"));
    let reset = Button::with_label("Reset to Tezca defaults");
    reset.add_css_class("tz-action");
    {
        let c = container.clone();
        reset.connect_clicked(move |_| {
            // Synchronous so the reload lands before we re-read the values.
            let _ = backend::tezca_result(&["hypr", "reset"]);
            while let Some(child) = c.first_child() {
                c.remove(&child);
            }
            populate_desktop(&c);
        });
    }
    page.append(&reset);
    page.append(&hint("Changes apply instantly and persist across reload/relogin (conf.d/local.conf). Reset clears them."));

    scrolled(&page)
}

fn populate_desktop(c: &Box) {
    c.append(&control_row("Inner gaps", &spin_opt("general:gaps_in", 0.0, 40.0, 1.0, 0)));
    c.append(&control_row("Outer gaps", &spin_opt("general:gaps_out", 0.0, 60.0, 1.0, 0)));
    c.append(&control_row("Border size", &spin_opt("general:border_size", 0.0, 8.0, 1.0, 0)));
    c.append(&control_row("Corner rounding", &spin_opt("decoration:rounding", 0.0, 24.0, 1.0, 0)));

    c.append(&control_row("Active opacity", &opacity_opt("decoration:active_opacity")));
    c.append(&control_row("Inactive opacity", &opacity_opt("decoration:inactive_opacity")));

    c.append(&control_row("Blur", &switch_opt("decoration:blur:enabled")));
    c.append(&control_row("Blur size", &spin_opt("decoration:blur:size", 1.0, 20.0, 1.0, 0)));
    c.append(&control_row("Blur passes", &spin_opt("decoration:blur:passes", 1.0, 5.0, 1.0, 0)));
    c.append(&control_row("Shadows", &switch_opt("decoration:shadow:enabled")));
    c.append(&control_row("Animations", &switch_opt("animations:enabled")));

    let vrr = DropDown::from_strings(&["Off", "Always on", "Fullscreen only"]);
    let cur = backend::hypr_get("misc:vrr").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    vrr.set_selected(cur.min(2));
    vrr.connect_selected_notify(|d| {
        backend::tezca(&["hypr", "set", "misc:vrr", &d.selected().to_string()]);
    });
    c.append(&control_row("Adaptive sync (VRR)", &vrr));
}

/// An integer SpinButton bound to a Hyprland option (get on build, set on change).
fn spin_opt(opt: &'static str, min: f64, max: f64, step: f64, digits: u32) -> SpinButton {
    let s = SpinButton::with_range(min, max, step);
    s.set_digits(digits);
    if let Some(v) = backend::hypr_get(opt).and_then(|x| x.parse::<f64>().ok()) {
        s.set_value(v);
    }
    s.connect_value_changed(move |s| {
        backend::tezca(&["hypr", "set", opt, &(s.value() as i64).to_string()]);
    });
    s
}

/// A 0–1 opacity Scale bound to a float option, debounced.
fn opacity_opt(opt: &'static str) -> Scale {
    let s = Scale::with_range(Orientation::Horizontal, 0.3, 1.0, 0.01);
    s.set_hexpand(true);
    s.set_draw_value(true);
    if let Some(v) = backend::hypr_get(opt).and_then(|x| x.parse::<f64>().ok()) {
        s.set_value(v);
    }
    debounce_scale(&s, 200, move |v| {
        backend::tezca(&["hypr", "set", opt, &format!("{v:.2}")]);
    });
    s
}

/// A boolean Switch bound to a Hyprland option.
fn switch_opt(opt: &'static str) -> Switch {
    let sw = Switch::new();
    sw.set_valign(Align::Center);
    let on = backend::hypr_get(opt).map(|v| v == "1" || v == "true").unwrap_or(false);
    sw.set_active(on);
    sw.connect_state_set(move |_, on| {
        backend::tezca(&["hypr", "set", opt, if on { "true" } else { "false" }]);
        glib::Propagation::Proceed
    });
    sw
}

/// A dock-config SpinButton seeded from `tezca dock config`.
fn spin_from(
    key: &str,
    min: f64,
    max: f64,
    step: f64,
    digits: u32,
    get: &dyn Fn(&str) -> Option<String>,
) -> SpinButton {
    let s = SpinButton::with_range(min, max, step);
    s.set_digits(digits);
    if let Some(v) = get(key).and_then(|x| x.parse::<f64>().ok()) {
        s.set_value(v);
    }
    s
}

// ===========================================================================
// Keybinds — editable, with search + conflict-aware rebinding
// ===========================================================================

pub fn keybinds(window: &Window) -> Widget {
    let page = page_box();

    let search = Entry::new();
    search.set_placeholder_text(Some("Search keybindings…"));
    search.add_css_class("tz-search");
    page.append(&search);

    let list = Box::new(Orientation::Vertical, 0);
    page.append(&list);

    let rebuild: Rc<dyn Fn()> = {
        let list = list.clone();
        let search = search.clone();
        let window = window.clone();
        Rc::new(move || {
            populate_keybinds(&list, &window, &search.text().to_lowercase());
        })
    };
    // Bind the rebuild to the search box, then do the first population.
    {
        let rebuild = rebuild.clone();
        search.connect_changed(move |_| rebuild());
    }
    populate_keybinds(&list, window, "");

    scrolled(&page)
}

fn populate_keybinds(list: &Box, window: &Window, filter: &str) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let sections = keybinds::load();
    if sections.is_empty() {
        let l = hint("Could not read keybinds.conf.");
        list.append(&l);
        return;
    }
    let rebuild: Rc<dyn Fn()> = {
        let list = list.clone();
        let window = window.clone();
        let filter = filter.to_string();
        Rc::new(move || populate_keybinds(&list, &window, &filter))
    };

    for sec in sections {
        let matching: Vec<_> = sec
            .binds
            .into_iter()
            .filter(|b| {
                filter.is_empty()
                    || b.desc.to_lowercase().contains(filter)
                    || b.combo().to_lowercase().contains(filter)
            })
            .collect();
        if matching.is_empty() {
            continue;
        }
        list.append(&section_header(&sec.title));
        let box_ = Box::new(Orientation::Vertical, 0);
        box_.add_css_class("tz-keylist");
        for b in matching {
            box_.append(&keybind_row(window, &b, rebuild.clone()));
        }
        list.append(&box_);
    }
}

fn keybind_row(window: &Window, b: &keybinds::Bind, on_done: Rc<dyn Fn()>) -> Box {
    let row = Box::new(Orientation::Horizontal, 12);
    row.add_css_class("tz-keyrow");

    let combo = Label::new(Some(&b.combo()));
    combo.add_css_class("tz-key");
    combo.set_width_chars(22);
    combo.set_xalign(0.0);
    combo.set_halign(Align::Start);

    let desc = Label::new(Some(&strip_tag(&b.desc)));
    desc.set_hexpand(true);
    desc.set_xalign(0.0);
    desc.set_halign(Align::Start);
    desc.set_wrap(true);
    desc.set_max_width_chars(46);

    let rebind = small_btn("Rebind");
    rebind.add_css_class("tz-rebind");
    {
        let window = window.clone();
        let b = b.clone();
        let on_done = on_done.clone();
        rebind.connect_clicked(move |_| capture_rebind(&window, &b, on_done.clone()));
    }

    row.append(&combo);
    row.append(&desc);
    row.append(&rebind);
    row
}

/// Modal "press a shortcut" capture → `tezca keybind rebind`. Handles conflicts
/// (exit 2) inline; on success closes and reloads the list.
fn capture_rebind(parent: &Window, b: &keybinds::Bind, on_done: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Rebind")
        .default_width(380)
        .default_height(150)
        .build();
    dialog.add_css_class("tz-capture");

    let v = Box::new(Orientation::Vertical, 10);
    v.set_margin_top(20);
    v.set_margin_bottom(20);
    v.set_margin_start(22);
    v.set_margin_end(22);

    let title = Label::new(Some(&format!("New shortcut for “{}”", strip_tag(&b.desc))));
    title.add_css_class("tz-h2");
    title.set_wrap(true);
    let prompt = Label::new(Some(&format!("Currently {}. Press a new combination…", b.combo())));
    prompt.add_css_class("tz-hint");
    prompt.set_wrap(true);
    let status = Label::new(Some("Esc to cancel"));
    status.add_css_class("tz-hint");
    v.append(&title);
    v.append(&prompt);
    v.append(&status);
    dialog.set_child(Some(&v));

    let keyctl = EventControllerKey::new();
    {
        let dialog = dialog.clone();
        let b = b.clone();
        let on_done = on_done.clone();
        let status = status.clone();
        keyctl.connect_key_pressed(move |_, keyval, _code, state| {
            if keyval == gdk::Key::Escape {
                dialog.close();
                return glib::Propagation::Stop;
            }
            if is_modifier(keyval) {
                return glib::Propagation::Proceed; // wait for the real key
            }
            let Some((mods, key)) = combo_from_event(keyval, state) else {
                return glib::Propagation::Stop;
            };
            let line = b.line.to_string();
            let res = backend::tezca_result(&[
                "keybind", "rebind",
                "--line", &line,
                "--expect-mods", &b.mods,
                "--expect-key", &b.key,
                "--mods", &mods,
                "--key", &key,
            ]);
            match res.code {
                0 => {
                    dialog.close();
                    on_done();
                }
                2 => {
                    let msg = res.stderr.trim_start_matches("conflict:").trim();
                    status.set_text(&format!("⚠ {msg} — try another"));
                    status.remove_css_class("tz-hint");
                    status.add_css_class("tz-warn");
                }
                _ => {
                    status.set_text(&format!("Error: {}", res.stderr));
                    status.add_css_class("tz-warn");
                }
            }
            glib::Propagation::Stop
        });
    }
    dialog.add_controller(keyctl);
    dialog.present();
}

/// Build a Hyprland combo (mods string, key name) from a GDK key event.
fn combo_from_event(keyval: gdk::Key, state: gdk::ModifierType) -> Option<(String, String)> {
    let mut mods = Vec::new();
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        mods.push("SUPER");
    }
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        mods.push("CTRL");
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        mods.push("ALT");
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        mods.push("SHIFT");
    }
    let key = keyval.name()?.to_string();
    Some((mods.join(" "), key))
}

fn is_modifier(k: gdk::Key) -> bool {
    matches!(
        k,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::Caps_Lock
    )
}

// ===========================================================================
// Gaming — profile toggle + detected tools
// ===========================================================================

pub fn gaming() -> Widget {
    let page = page_box();
    page.append(&section_header("Game mode"));

    let row = Box::new(Orientation::Horizontal, 12);
    row.add_css_class("tz-switchrow");
    let lbl = Label::new(Some("Low-latency profile"));
    lbl.set_halign(Align::Start);
    lbl.set_hexpand(true);
    let sw = Switch::new();
    sw.set_active(backend::game_on());
    sw.set_valign(Align::Center);
    sw.connect_state_set(|_, on| {
        backend::tezca(&["game", if on { "on" } else { "off" }]);
        glib::Propagation::Proceed
    });
    row.append(&lbl);
    row.append(&sw);
    page.append(&row);
    page.append(&hint(
        "Turns off blur, shadows and animations for maximum frame pacing. Also on SUPER+ALT+G. Games auto-move to workspace 5.",
    ));

    page.append(&section_header("Tools"));
    for (label, bin, desc) in [
        ("gamemode", "gamemoderun", "gamemoderun — CPU governor + process priorities"),
        ("mangohud", "mangohud", "MangoHud — in-game FPS / frametime overlay"),
        ("gamescope", "gamescope", "gamescope — micro-compositor for VRR & scaling"),
    ] {
        page.append(&status_row(label, backend::has(bin), desc));
    }
    scrolled(&page)
}

// ===========================================================================
// System — session actions + info
// ===========================================================================

pub fn system() -> Widget {
    let page = page_box();
    page.append(&section_header("Session"));

    let actions = FlowBox::new();
    actions.set_selection_mode(SelectionMode::None);
    actions.set_max_children_per_line(3);
    actions.set_column_spacing(8);
    actions.set_row_spacing(8);
    actions.set_halign(Align::Start);

    let lock = action("Lock screen");
    lock.connect_clicked(|_| backend::spawn("hyprlock", &[]));
    let reload = action("Reload Hyprland");
    reload.connect_clicked(|_| backend::spawn("hyprctl", &["reload"]));
    let waybar = action("Toggle Waybar");
    waybar.connect_clicked(|_| backend::run_script("waybar-toggle.sh", &[]));
    let dock = action("Restart dock");
    dock.connect_clicked(|_| backend::tezca(&["dock", "restart"]));
    let diag = action("Diagnostics");
    diag.connect_clicked(|_| {
        let t = backend::tezca_bin();
        backend::spawn("kitty", &["--hold", "-e", t.as_str(), "doctor"]);
    });
    let logout = action("Logout menu");
    logout.connect_clicked(|_| backend::spawn("wlogout", &["-b", "4"]));
    for b in [&lock, &reload, &waybar, &dock, &diag, &logout] {
        actions.append(b);
    }
    page.append(&actions);
    page.append(&hint(
        "Diagnostics runs `tezca doctor` in a terminal. Reload re-sources the Hyprland config (restores eye-candy after game mode).",
    ));

    page.append(&section_header("This session"));
    let compositor = backend::output("hyprctl", &["version"])
        .and_then(|s| s.lines().next().map(str::to_string))
        .map(|l| {
            let mut it = l.split_whitespace();
            match (it.next(), it.next()) {
                (Some(a), Some(b)) => format!("{a} {b}"),
                (Some(a), None) => a.to_string(),
                _ => "Hyprland".to_string(),
            }
        })
        .unwrap_or_else(|| "Hyprland".to_string());
    let monitors = backend::output("hyprctl", &["monitors"])
        .map(|s| s.lines().filter(|l| l.starts_with("Monitor ")).count())
        .unwrap_or(0);
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "wayland".to_string());
    page.append(&info_row("Compositor", &compositor));
    page.append(&info_row("Monitors", &monitors.to_string()));
    page.append(&info_row("Session", &session));

    scrolled(&page)
}

// ===========================================================================
// Shared widget helpers
// ===========================================================================

fn page_box() -> Box {
    let b = Box::new(Orientation::Vertical, 8);
    b.add_css_class("tz-page");
    b.set_margin_top(18);
    b.set_margin_bottom(18);
    b.set_margin_start(22);
    b.set_margin_end(22);
    b
}

fn section_header(title: &str) -> Label {
    let l = Label::new(Some(title));
    l.add_css_class("tz-h2");
    l.set_halign(Align::Start);
    l.set_margin_top(10);
    l
}

fn hint(text: &str) -> Label {
    let l = Label::new(Some(text));
    l.add_css_class("tz-hint");
    l.set_halign(Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_max_width_chars(72);
    l
}

fn action(label: &str) -> Button {
    let b = Button::with_label(label);
    b.add_css_class("tz-action");
    b
}

fn small_btn(label: &str) -> Button {
    let b = Button::with_label(label);
    b.add_css_class("tz-small");
    b
}

/// A label on the left, a control pushed to the right — the standard settings row.
fn control_row(label: &str, control: &impl IsA<Widget>) -> Box {
    let row = Box::new(Orientation::Horizontal, 12);
    row.add_css_class("tz-ctlrow");
    let l = Label::new(Some(label));
    l.set_halign(Align::Start);
    l.set_hexpand(true);
    l.set_xalign(0.0);
    row.append(&l);
    row.append(control);
    row
}

fn status_row(name: &str, ok: bool, desc: &str) -> Widget {
    let row = Box::new(Orientation::Horizontal, 10);
    row.add_css_class("tz-statusrow");
    let dot = Label::new(Some(if ok { "●" } else { "○" }));
    dot.add_css_class(if ok { "tz-ok" } else { "tz-miss" });
    let name_l = Label::new(Some(name));
    name_l.set_width_chars(11);
    name_l.set_xalign(0.0);
    let desc_l = Label::new(Some(desc));
    desc_l.add_css_class("tz-hint");
    desc_l.set_hexpand(true);
    desc_l.set_xalign(0.0);
    desc_l.set_halign(Align::Start);
    row.append(&dot);
    row.append(&name_l);
    row.append(&desc_l);
    row.upcast()
}

fn info_row(key: &str, val: &str) -> Widget {
    let row = Box::new(Orientation::Horizontal, 10);
    row.add_css_class("tz-inforow");
    let k = Label::new(Some(key));
    k.add_css_class("tz-key2");
    k.set_width_chars(13);
    k.set_xalign(0.0);
    let v = Label::new(Some(val));
    v.set_xalign(0.0);
    v.set_halign(Align::Start);
    v.set_hexpand(true);
    row.append(&k);
    row.append(&v);
    row.upcast()
}

fn scrolled(child: &Box) -> Widget {
    let s = ScrolledWindow::new();
    s.set_hscrollbar_policy(PolicyType::Never);
    s.set_vexpand(true);
    s.set_child(Some(child));
    s.upcast()
}

/// Apply a Scale's value `ms` after the user stops dragging (coalesces the
/// stream of value-changed events so slow backends like ddcutil aren't hammered).
fn debounce_scale<F: Fn(f64) + 'static>(scale: &Scale, ms: u64, f: F) {
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let f = Rc::new(f);
    scale.connect_value_changed(move |s| {
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let v = s.value();
        let f = f.clone();
        let pending2 = pending.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            *pending2.borrow_mut() = None;
            f(v);
        });
        *pending.borrow_mut() = Some(id);
    });
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Drop a trailing " (HyDE)" / " (Tezca…)" parenthetical from a bind description.
fn strip_tag(s: &str) -> String {
    if s.ends_with(')') {
        if let Some(idx) = s.rfind(" (") {
            return s[..idx].to_string();
        }
    }
    s.to_string()
}
