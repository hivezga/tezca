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
use gtk4::gio::prelude::AppInfoExt;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box, Button, ContentFit, DropDown, Entry, EventControllerKey, FileDialog, FlowBox,
    Label, Orientation, Picture, PolicyType, Scale, ScrolledWindow, SelectionMode, SpinButton,
    Switch, Widget, Window,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// A slot holding a closure that repaints a list, shared with the rows it draws.
type RenderCell = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

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
        "Curated palettes. Switching re-skins the bar, the terminal, the dock, hyprlock and the launcher live — no restart.",
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
    let (page, status) = page_with_status();
    let container = Box::new(Orientation::Vertical, 0);
    let rebuild: RenderCell = Rc::new(RefCell::new(None));
    {
        let c = container.clone();
        let w = window.clone();
        let st = status.clone();
        let rb = rebuild.clone();
        *rebuild.borrow_mut() = Some(Rc::new(move || {
            while let Some(child) = c.first_child() {
                c.remove(&child);
            }
            populate_displays(&c, &w, &st, &rb);
        }));
    }
    populate_displays(&container, window, &status, &rebuild);
    page.append(&container);
    scrolled(&page)
}

/// Re-run the page's own builder (see [`displays`]), so a change that moves other
/// monitors — a placement, a profile — is reflected everywhere at once.
fn redraw(rebuild: &RenderCell) {
    let f = rebuild.borrow().clone();
    if let Some(f) = f {
        f();
    }
}

/// Hyprland's eight output transforms.
///
/// Deliberately not labelled "clockwise"/"counter-clockwise": which way 90°
/// turns the picture depends on the panel, and a label that is wrong half the
/// time is worse than one that just names the angle. If a monitor comes up
/// rotated the wrong way, the other 90° entry is the one you want.
const ORIENTATIONS: &[(&str, &str)] = &[
    ("Landscape", "0"),
    ("Portrait (90°)", "1"),
    ("Upside down (180°)", "2"),
    ("Portrait (270°)", "3"),
    ("Flipped", "4"),
    ("Flipped, 90°", "5"),
    ("Flipped, 180°", "6"),
    ("Flipped, 270°", "7"),
];

const VRR_MODES: &[(&str, &str)] = &[
    ("Inherit global", ""),
    ("Off", "0"),
    ("Always on", "1"),
    ("Fullscreen only", "2"),
];

const BITDEPTHS: &[(&str, &str)] = &[("Automatic", ""), ("8-bit", "8"), ("10-bit", "10")];

/// Index of `value` in a (label, value) table, defaulting to 0.
fn table_index(table: &[(&str, &str)], value: &str) -> u32 {
    table.iter().position(|(_, v)| *v == value).unwrap_or(0) as u32
}

fn table_labels<'a>(table: &[(&'a str, &str)]) -> Vec<&'a str> {
    table.iter().map(|(l, _)| *l).collect()
}

/// The full `display set` flag list that reproduces `m` exactly.
///
/// Used as the *undo* for a change: every field is spelled out, so reverting
/// restores the whole spec rather than only the field that was touched. VRR and
/// bit depth come from the override store (`cfg`), because the compositor cannot
/// report either faithfully — see `backend::Monitor`.
fn spec_args(m: &backend::Monitor, cfg: &[(String, String)]) -> Vec<String> {
    let vrr = backend::override_for(cfg, &m.name, "vrr").unwrap_or_default();
    let bitdepth = backend::override_for(cfg, &m.name, "bitdepth").unwrap_or_default();
    let mirror = if m.mirror.is_empty() { "off".to_string() } else { m.mirror.clone() };
    vec![
        "--mode".into(),
        format!("{}@{}", m.res, m.rate),
        "--scale".into(),
        m.scale.clone(),
        "--pos".into(),
        m.pos.clone(),
        "--transform".into(),
        if m.transform.is_empty() { "0".into() } else { m.transform.clone() },
        "--vrr".into(),
        if vrr.is_empty() { "inherit".into() } else { vrr },
        "--bitdepth".into(),
        bitdepth,
        "--mirror".into(),
        mirror,
        if m.disabled { "--off".into() } else { "--on".into() },
    ]
}

/// Apply a display change, then make the user confirm it is readable.
///
/// The failure mode this exists for: a mode, scale or rotation that leaves the
/// screen unreadable — and the control to undo it is on that screen. So the
/// change is applied, a countdown appears, and anything other than an explicit
/// "Keep" puts the previous spec back. Windows and GNOME both do this, for the
/// same reason.
fn apply_display_confirmed(
    window: &Window,
    status: &Status,
    monitor: &str,
    change: Vec<String>,
    revert: Vec<String>,
    rebuild: &RenderCell,
) {
    let mut args: Vec<String> = vec!["display".into(), "set".into(), monitor.into()];
    args.extend(change);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = backend::tezca_result(&argv);
    if !res.ok() {
        status.err(&res.message());
        return;
    }
    // A warning on stdout (10-bit silently not taking, say) is worth surfacing
    // even though the command succeeded.
    if res.stdout.lines().any(|l| l.contains("did not take")) {
        status.warn(&res.message());
    }
    confirm_or_revert(window, status, monitor, revert, rebuild);
}

const REVERT_SECONDS: u32 = 15;

fn confirm_or_revert(
    window: &Window,
    status: &Status,
    monitor: &str,
    revert: Vec<String>,
    rebuild: &RenderCell,
) {
    let dialog = Window::builder()
        .transient_for(window)
        .modal(true)
        .title("Keep display settings?")
        .default_width(420)
        .resizable(false)
        .build();
    dialog.add_css_class("tz-capture");

    let b = Box::new(Orientation::Vertical, 12);
    b.set_margin_top(18);
    b.set_margin_bottom(18);
    b.set_margin_start(20);
    b.set_margin_end(20);

    let title = Label::new(Some(&format!("Keep the new settings for {monitor}?")));
    title.add_css_class("tz-h2");
    title.set_halign(Align::Start);
    let count = Label::new(Some(&format!("Reverting in {REVERT_SECONDS} seconds…")));
    count.add_css_class("tz-hint");
    count.set_halign(Align::Start);
    b.append(&title);
    b.append(&count);
    b.append(&hint(
        "If the screen is unreadable, wait — the previous settings come back on their own. \
         From a terminal: `tezca display reset <monitor>`, or `hyprctl reload`.",
    ));

    let row = Box::new(Orientation::Horizontal, 8);
    row.set_halign(Align::End);
    let revert_btn = Button::with_label("Revert");
    let keep = Button::with_label("Keep");
    keep.add_css_class("tz-action");
    row.append(&revert_btn);
    row.append(&keep);
    b.append(&row);
    dialog.set_child(Some(&b));

    // One shared "already settled" flag: the timer, both buttons and the window's
    // close box all race to finish this, and whoever gets there first wins.
    let settled = Rc::new(Cell::new(false));
    let do_revert: Rc<dyn Fn()> = {
        let settled = settled.clone();
        let status = status.clone();
        let dialog = dialog.clone();
        let rebuild = rebuild.clone();
        let monitor = monitor.to_string();
        Rc::new(move || {
            if settled.replace(true) {
                return;
            }
            let mut args: Vec<String> = vec!["display".into(), "set".into(), monitor.clone()];
            args.extend(revert.clone());
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            let r = backend::tezca_result(&argv);
            dialog.close();
            if r.ok() {
                status.warn("Reverted to the previous display settings.");
            } else {
                status.err(&format!("Could not revert: {}", r.message()));
            }
            redraw(&rebuild);
        })
    };

    {
        let settled = settled.clone();
        let status = status.clone();
        let dialog = dialog.clone();
        let rebuild = rebuild.clone();
        keep.connect_clicked(move |_| {
            if settled.replace(true) {
                return;
            }
            dialog.close();
            status.ok("Display settings applied.");
            redraw(&rebuild);
        });
    }
    {
        let f = do_revert.clone();
        revert_btn.connect_clicked(move |_| f());
    }
    {
        // Closing the window is a "no" — the same as letting it time out.
        let f = do_revert.clone();
        dialog.connect_close_request(move |_| {
            f();
            glib::Propagation::Proceed
        });
    }

    let left = Rc::new(Cell::new(REVERT_SECONDS));
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if settled.get() {
            return glib::ControlFlow::Break;
        }
        let n = left.get().saturating_sub(1);
        left.set(n);
        if n == 0 {
            do_revert();
            return glib::ControlFlow::Break;
        }
        count.set_text(&format!("Reverting in {n} seconds…"));
        glib::ControlFlow::Continue
    });

    dialog.present();
}

/// Saved layouts: "both screens" vs "just the ultrawide" is one click, not four.
fn profiles_section(c: &Box, status: &Status, rebuild: &RenderCell) {
    let names = backend::tezca_out(&["display", "profile", "list"])
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.contains("no saved"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    c.append(&section_header("Layout profiles"));
    let row = Box::new(Orientation::Horizontal, 8);

    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("New profile name"));
    name_entry.set_hexpand(true);
    row.append(&name_entry);

    let save = small_btn("Save current");
    {
        let entry = name_entry.clone();
        let st = status.clone();
        let rb = rebuild.clone();
        save.connect_clicked(move |_| {
            let name = entry.text().trim().to_string();
            if name.is_empty() {
                st.warn("Give the profile a name first.");
                return;
            }
            let r = backend::tezca_result(&["display", "profile", "save", &name]);
            st.report(&r, &format!("Saved layout profile “{name}”."));
            entry.set_text("");
            redraw(&rb);
        });
    }
    row.append(&save);
    c.append(&row);

    if names.is_empty() {
        c.append(&hint(
            "No saved profiles yet. Save one to snapshot every monitor's mode, position, \
             scale, rotation and advanced settings, then switch back to it in one click.",
        ));
        return;
    }

    for n in names {
        let prow = Box::new(Orientation::Horizontal, 8);
        prow.add_css_class("tz-pinrow");
        let l = Label::new(Some(&n));
        l.set_halign(Align::Start);
        l.set_hexpand(true);
        l.set_xalign(0.0);
        prow.append(&l);

        let apply = small_btn("Apply");
        {
            let n = n.clone();
            let st = status.clone();
            let rb = rebuild.clone();
            apply.connect_clicked(move |_| {
                let r = backend::tezca_result(&["display", "profile", "apply", &n]);
                st.report(&r, &format!("Applied layout profile “{n}”."));
                redraw(&rb);
            });
        }
        let del = small_btn("✕");
        {
            let n = n.clone();
            let st = status.clone();
            let rb = rebuild.clone();
            del.connect_clicked(move |_| {
                let r = backend::tezca_result(&["display", "profile", "rm", &n]);
                st.report(&r, &format!("Removed profile “{n}”."));
                redraw(&rb);
            });
        }
        prow.append(&apply);
        prow.append(&del);
        c.append(&prow);
    }
}

/// Night light lives on the Displays page because that is where you look for it,
/// even though it is driven by a separate daemon.
fn night_section(c: &Box, status: &Status) {
    let st = backend::flat(&backend::tezca_out(&["night", "status", "--machine"]).unwrap_or_default());
    let get = |k: &str| st.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();

    c.append(&section_header("Night light"));
    if get("available") != "true" {
        c.append(&hint(
            "hyprsunset is not installed — install it (`paru -S hyprsunset`) to filter blue \
             light in the evening.",
        ));
        return;
    }

    let sw = Switch::new();
    sw.set_valign(Align::Center);
    sw.set_active(get("enabled") == "true");
    {
        let stt = status.clone();
        sw.connect_state_set(move |_, on| {
            let r = backend::tezca_result(&["night", if on { "on" } else { "off" }]);
            stt.report(&r, if on { "Night light on." } else { "Night light off." });
            glib::Propagation::Proceed
        });
    }
    c.append(&control_row("Night light", &sw));

    let temp = Scale::with_range(Orientation::Horizontal, 1000.0, 6500.0, 100.0);
    temp.set_hexpand(true);
    temp.set_draw_value(true);
    temp.set_value(get("temp").parse().unwrap_or(4000.0));
    // Live preview as you drag: the number means nothing until you see it.
    debounce_scale(&temp, 200, |v| {
        backend::tezca(&["night", "temp", &(v as u32).to_string()]);
    });
    c.append(&control_row("Colour temperature (K)", &temp));
    c.append(&hint("Lower is warmer. 6500 K is daylight; 4000 K is a typical evening setting."));

    let from = Entry::new();
    from.set_placeholder_text(Some("21:30"));
    from.set_text(&get("from"));
    from.set_max_width_chars(6);
    let to = Entry::new();
    to.set_placeholder_text(Some("06:30"));
    to.set_text(&get("to"));
    to.set_max_width_chars(6);
    let times = Box::new(Orientation::Horizontal, 8);
    times.append(&from);
    times.append(&to);
    c.append(&control_row("Schedule (from, to)", &times));

    let row = Box::new(Orientation::Horizontal, 8);
    row.set_halign(Align::End);
    let clear = small_btn("Clear schedule");
    {
        let stt = status.clone();
        let (from, to) = (from.clone(), to.clone());
        clear.connect_clicked(move |_| {
            let r = backend::tezca_result(&["night", "schedule", "off"]);
            from.set_text("");
            to.set_text("");
            stt.report(&r, "Schedule cleared — the switch alone decides now.");
        });
    }
    let apply = small_btn("Set schedule");
    {
        let stt = status.clone();
        let (from, to) = (from.clone(), to.clone());
        apply.connect_clicked(move |_| {
            let (f, t) = (from.text().to_string(), to.text().to_string());
            if f.trim().is_empty() || t.trim().is_empty() {
                stt.warn("Enter both a start and an end time, as HH:MM.");
                return;
            }
            let r = backend::tezca_result(&["night", "schedule", &f, &t]);
            stt.report(&r, &format!("Night light runs {f} → {t}."));
        });
    }
    row.append(&clear);
    row.append(&apply);
    c.append(&row);
    c.append(&hint(
        "A window may cross midnight (21:30 → 06:30). The menubar checks the clock and \
         switches the filter on and off at the boundaries.",
    ));
}

fn populate_displays(c: &Box, window: &Window, status: &Status, rebuild: &RenderCell) {
    let mons = backend::monitors();
    if mons.is_empty() {
        c.append(&hint("Could not read monitors (are you in a Hyprland session?)."));
        return;
    }
    let cfg = backend::display_config();
    let walls = backend::wallpaper_targets();

    profiles_section(c, status, rebuild);
    night_section(c, status);

    for m in &mons {
        let title = if m.disabled { format!("{}  (off)", m.name) } else { m.name.clone() };
        c.append(&section_header(&title));
        c.append(&hint(&m.desc));

        // --- Mode + scale ---------------------------------------------------
        // A disabled output can report no modes at all; keep the control usable
        // so it can be switched back on.
        let modes: Vec<String> =
            if m.modes.is_empty() { vec!["preferred".to_string()] } else { m.modes.clone() };
        let mode_refs: Vec<&str> = modes.iter().map(String::as_str).collect();
        let dd = DropDown::from_strings(&mode_refs);
        let current = format!("{}@{}", m.res, m.rate);
        if let Some(i) = modes.iter().position(|x| *x == current) {
            dd.set_selected(i as u32);
        }
        c.append(&control_row("Resolution & refresh", &dd));

        let scale = SpinButton::with_range(0.5, 3.0, 0.05);
        scale.set_digits(2);
        scale.set_value(m.scale.parse().unwrap_or(1.0));
        c.append(&control_row("Scale", &scale));

        // --- Orientation ----------------------------------------------------
        let orient = DropDown::from_strings(&table_labels(ORIENTATIONS));
        orient.set_selected(table_index(ORIENTATIONS, &m.transform));
        c.append(&control_row("Orientation", &orient));

        // --- Advanced -------------------------------------------------------
        let adv = gtk4::Expander::new(Some("Advanced"));
        adv.add_css_class("tz-expander");
        let advbox = Box::new(Orientation::Vertical, 0);
        advbox.set_margin_top(6);
        advbox.set_margin_start(6);

        let enabled = Switch::new();
        enabled.set_valign(Align::Center);
        enabled.set_active(!m.disabled);
        advbox.append(&control_row("Enabled", &enabled));

        let vrr = DropDown::from_strings(&table_labels(VRR_MODES));
        vrr.set_selected(table_index(
            VRR_MODES,
            &backend::override_for(&cfg, &m.name, "vrr").unwrap_or_default(),
        ));
        advbox.append(&control_row("Adaptive sync (VRR)", &vrr));

        let depth = DropDown::from_strings(&table_labels(BITDEPTHS));
        depth.set_selected(table_index(
            BITDEPTHS,
            &backend::override_for(&cfg, &m.name, "bitdepth").unwrap_or_default(),
        ));
        advbox.append(&control_row("Colour depth", &depth));

        // Mirror: "off" plus every other connected output.
        let mut mirror_vals: Vec<String> = vec!["off".to_string()];
        mirror_vals.extend(mons.iter().filter(|o| o.name != m.name).map(|o| o.name.clone()));
        let mirror_labels: Vec<String> = mirror_vals
            .iter()
            .map(|v| if v == "off" { "Off".to_string() } else { format!("Mirror of {v}") })
            .collect();
        let mirror_refs: Vec<&str> = mirror_labels.iter().map(String::as_str).collect();
        let mirror = DropDown::from_strings(&mirror_refs);
        let cur_mirror = if m.mirror.is_empty() { "off" } else { m.mirror.as_str() };
        if let Some(i) = mirror_vals.iter().position(|v| v == cur_mirror) {
            mirror.set_selected(i as u32);
        }
        advbox.append(&control_row("Mirror", &mirror));

        // Position. Hyprland lays out in logical pixels, so these are the same
        // units the "place beside" buttons produce.
        let (px, py) = m
            .pos
            .split_once('x')
            .map(|(a, b)| (a.parse::<f64>().unwrap_or(0.0), b.parse::<f64>().unwrap_or(0.0)))
            .unwrap_or((0.0, 0.0));
        let pos_x = SpinButton::with_range(-32768.0, 32768.0, 10.0);
        pos_x.set_value(px);
        let pos_y = SpinButton::with_range(-32768.0, 32768.0, 10.0);
        pos_y.set_value(py);
        let posrow = Box::new(Orientation::Horizontal, 8);
        posrow.append(&pos_x);
        posrow.append(&pos_y);
        advbox.append(&control_row("Position (x, y)", &posrow));

        if mons.len() > 1 {
            let place = Box::new(Orientation::Horizontal, 6);
            place.set_halign(Align::End);
            for other in mons.iter().filter(|o| o.name != m.name) {
                let btn = small_btn(&format!("Right of {}", other.name));
                let name = m.name.clone();
                let anchor = other.name.clone();
                let st = status.clone();
                let rb = rebuild.clone();
                let win = window.clone();
                let revert = spec_args(m, &cfg);
                btn.connect_clicked(move |_| {
                    apply_display_confirmed(
                        &win,
                        &st,
                        &name,
                        vec!["--right-of".to_string(), anchor.clone()],
                        revert.clone(),
                        &rb,
                    );
                });
                place.append(&btn);
            }
            advbox.append(&place);
        }

        adv.set_child(Some(&advbox));
        c.append(&adv);

        // --- Apply ----------------------------------------------------------
        let apply = Button::with_label("Apply");
        apply.add_css_class("tz-action");
        {
            let name = m.name.clone();
            let modes = modes.clone();
            let (dd, scale, orient, vrr, depth, mirror, enabled) = (
                dd.clone(),
                scale.clone(),
                orient.clone(),
                vrr.clone(),
                depth.clone(),
                mirror.clone(),
                enabled.clone(),
            );
            let (pos_x, pos_y) = (pos_x.clone(), pos_y.clone());
            let mirror_vals = mirror_vals.clone();
            let revert = spec_args(m, &cfg);
            let st = status.clone();
            let rb = rebuild.clone();
            let win = window.clone();
            apply.connect_clicked(move |_| {
                let mode = modes.get(dd.selected() as usize).cloned().unwrap_or_default();
                let change = vec![
                    "--mode".to_string(),
                    mode,
                    "--scale".to_string(),
                    format!("{:.2}", scale.value()),
                    "--pos".to_string(),
                    format!("{}x{}", pos_x.value() as i64, pos_y.value() as i64),
                    "--transform".to_string(),
                    ORIENTATIONS[orient.selected() as usize].1.to_string(),
                    "--vrr".to_string(),
                    match VRR_MODES[vrr.selected() as usize].1 {
                        "" => "inherit".to_string(),
                        v => v.to_string(),
                    },
                    "--bitdepth".to_string(),
                    BITDEPTHS[depth.selected() as usize].1.to_string(),
                    "--mirror".to_string(),
                    mirror_vals.get(mirror.selected() as usize).cloned().unwrap_or_default(),
                    if enabled.is_active() { "--on".to_string() } else { "--off".to_string() },
                ];
                apply_display_confirmed(&win, &st, &name, change, revert.clone(), &rb);
            });
        }
        let reset = small_btn("Reset to shipped");
        {
            let name = m.name.clone();
            let st = status.clone();
            let rb = rebuild.clone();
            reset.connect_clicked(move |_| {
                let r = backend::tezca_result(&["display", "reset", &name]);
                st.report(&r, &format!("{name} reset to the shipped config."));
                redraw(&rb);
            });
        }
        let apply_row = Box::new(Orientation::Horizontal, 8);
        apply_row.set_halign(Align::End);
        apply_row.append(&reset);
        apply_row.append(&apply);
        c.append(&apply_row);

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
                c.append(&control_row("Brightness", &sl));
            }
            None => {
                c.append(&hint("Brightness: no DDC/CI channel (install ddcutil / not supported)."));
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
        c.append(&wp);

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
        c.append(&wrow);
    }
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

    // --- On-screen display (OSD) --------------------------------------------
    page.append(&section_header("On-screen display"));
    let osd_enabled = Switch::new();
    osd_enabled.set_valign(Align::Center);
    osd_enabled.set_active(get("osd_enabled").as_deref() != Some("false"));
    page.append(&control_row("Volume / brightness OSD", &osd_enabled));
    let osd_timeout = spin_from("osd_timeout_ms", 400.0, 10000.0, 100.0, 0, &get);
    page.append(&control_row("Dwell before fading (ms)", &osd_timeout));
    page.append(&hint(
        "The glass pill that flashes when you change the volume (or a laptop's brightness). The camera and microphone privacy indicators are modules — add them under Modules below.",
    ));

    // --- Modules ------------------------------------------------------------
    page.append(&section_header("Modules"));
    page.append(&hint(
        "Choose which modules each region of the bar shows, and their order. Reorder with ↑ ↓, remove with ✕, or add one below. “Separator” inserts a divider. On a narrow (compact) monitor the App name is dropped automatically.",
    ));
    // The Add menu offers the built-ins plus any custom exec modules discovered
    // in ~/.config/tezca-bar/modules (see the hint below).
    let mut defs: Vec<(String, String)> =
        MODULE_DEFS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    defs.extend(backend::custom_bar_modules());
    let defs = Rc::new(defs);

    const DEF_LEFT: &str = "mirror, sep, appname, sep, workspaces, submap";
    const DEF_CENTER: &str = "nowplaying";
    const DEF_RIGHT: &str =
        "gamemode, camera, microphone, recording, caffeine, night, ai, tray, cpu, mem, gpu, sep, network, bluetooth, volume, brightness, battery, sep, bell, clock, power";
    let (left_w, left_ids) = module_region("Left", seed_modules(get("layout_left"), DEF_LEFT), defs.clone());
    let (center_w, center_ids) =
        module_region("Center", seed_modules(get("layout_center"), DEF_CENTER), defs.clone());
    let (right_w, right_ids) = module_region("Right", seed_modules(get("layout_right"), DEF_RIGHT), defs.clone());
    let modules_row = Box::new(Orientation::Horizontal, 16);
    modules_row.add_css_class("tz-modcols");
    left_w.set_hexpand(true);
    center_w.set_hexpand(true);
    right_w.set_hexpand(true);
    modules_row.append(&left_w);
    modules_row.append(&center_w);
    modules_row.append(&right_w);
    page.append(&modules_row);
    page.append(&hint(
        "Custom modules: drop a <name>.toml manifest in ~/.config/tezca-bar/modules/ (a shell command whose output becomes the widget) and it appears in the Add menu here. See the example shipped in that folder.",
    ));

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
        let (osd_enabled, osd_timeout) = (osd_enabled.clone(), osd_timeout.clone());
        let (left_ids, center_ids, right_ids) = (left_ids.clone(), center_ids.clone(), right_ids.clone());
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
                ("osd_enabled".into(), bool_str(osd_enabled.is_active())),
                ("osd_timeout_ms".into(), (osd_timeout.value() as i64).to_string()),
            ];
            for (name, dd, entry) in assign_rows.iter() {
                kvs.push((format!("workspaces.{name}"), ws_spec_value(dd, entry)));
            }
            kvs.push(("layout_left".into(), left_ids.borrow().join(", ")));
            kvs.push(("layout_center".into(), center_ids.borrow().join(", ")));
            kvs.push(("layout_right".into(), right_ids.borrow().join(", ")));
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

/// The placeable bar modules — id (matching crates/tezca-bar/src/config.rs) and
/// the friendly name shown in the Modules editor. Mirrored vocabulary, the same
/// way the CLI's SCALARS table mirrors the bar's config keys.
const MODULE_DEFS: &[(&str, &str)] = &[
    ("mirror", "Tezca menu"),
    ("appname", "App name"),
    ("workspaces", "Workspaces"),
    ("submap", "Submap indicator"),
    ("nowplaying", "Now playing"),
    ("gamemode", "Game mode"),
    ("camera", "Camera indicator"),
    ("microphone", "Microphone indicator"),
    ("ai", "AI usage"),
    ("tray", "System tray"),
    ("cpu", "CPU"),
    ("mem", "Memory"),
    ("gpu", "GPU"),
    ("network", "Network"),
    ("bluetooth", "Bluetooth"),
    ("recording", "Recording indicator"),
    ("caffeine", "Keep awake"),
    ("night", "Night light"),
    ("volume", "Volume"),
    ("brightness", "Brightness"),
    ("battery", "Battery"),
    ("bell", "Notifications"),
    ("clock", "Clock"),
    ("power", "Power"),
    ("sep", "│  Separator"),
];

/// Friendly label for a module id, looked up in `defs` (built-ins + discovered
/// customs). A `custom:<name>` id with no matching manifest is flagged as
/// missing so the user can spot and remove a stale reference.
fn label_for(id: &str, defs: &[(String, String)]) -> String {
    if let Some((_, name)) = defs.iter().find(|(k, _)| k == id) {
        return name.clone();
    }
    if let Some(name) = id.strip_prefix("custom:") {
        return format!("{} (missing)", capitalize(name));
    }
    capitalize(id)
}

/// Split a `layout_*` CSV into module ids, falling back to `default` when the
/// config value is missing or blank.
fn seed_modules(current: Option<String>, default: &str) -> Vec<String> {
    let src = current.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| default.to_string());
    src.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// One region's module editor: a titled, reorderable list (↑ / ↓ / ✕ per row)
/// plus an "Add" dropdown. Returns the widget to append and a shared handle on
/// the ordered id list to read back at Apply time. Rows are fully rebuilt on
/// every edit, so the per-row captured indices are always current.
fn module_region(
    title: &str,
    ids: Vec<String>,
    defs: Rc<Vec<(String, String)>>,
) -> (Box, Rc<RefCell<Vec<String>>>) {
    let wrap = Box::new(Orientation::Vertical, 4);
    wrap.add_css_class("tz-modregion");
    let head = Label::new(Some(title));
    head.add_css_class("tz-key2");
    head.set_halign(Align::Start);
    wrap.append(&head);

    let list = Box::new(Orientation::Vertical, 4);
    list.add_css_class("tz-modlist");
    wrap.append(&list);

    let state = Rc::new(RefCell::new(ids));

    // A self-referential render closure: each row's buttons call back into it to
    // repaint the list after mutating `state`. The cell is what breaks the cycle —
    // the closure cannot capture itself, so it captures this and fills it in after.
    let render: RenderCell = Rc::new(RefCell::new(None));
    {
        let list = list.clone();
        let state = state.clone();
        let render_ref = render.clone();
        let defs = defs.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let ids = state.borrow().clone();
            let len = ids.len();
            for (i, id) in ids.iter().enumerate() {
                let row = Box::new(Orientation::Horizontal, 6);
                row.add_css_class("tz-modrow");
                let name = Label::new(Some(&label_for(id, &defs)));
                name.set_halign(Align::Start);
                name.set_hexpand(true);
                name.set_xalign(0.0);
                row.append(&name);

                let up = small_btn("↑");
                up.set_sensitive(i > 0);
                let down = small_btn("↓");
                down.set_sensitive(i + 1 < len);
                let del = small_btn("✕");

                for (btn, delta) in [(&up, -1i32), (&down, 1i32)] {
                    let state = state.clone();
                    let render_ref = render_ref.clone();
                    btn.connect_clicked(move |_| {
                        let j = i as i32 + delta;
                        if j >= 0 && (j as usize) < state.borrow().len() {
                            state.borrow_mut().swap(i, j as usize);
                        }
                        if let Some(f) = render_ref.borrow().clone() {
                            f();
                        }
                    });
                }
                {
                    let state = state.clone();
                    let render_ref = render_ref.clone();
                    del.connect_clicked(move |_| {
                        if i < state.borrow().len() {
                            state.borrow_mut().remove(i);
                        }
                        if let Some(f) = render_ref.borrow().clone() {
                            f();
                        }
                    });
                }
                row.append(&up);
                row.append(&down);
                row.append(&del);
                list.append(&row);
            }
        });
        *render.borrow_mut() = Some(f);
    }
    if let Some(f) = render.borrow().clone() {
        f();
    }

    // Add row: pick any module (built-in or a discovered custom one). Duplicates
    // land harmlessly — the bar parents each non-separator widget once and skips
    // repeats.
    let labels: Vec<&str> = defs.iter().map(|(_, n)| n.as_str()).collect();
    let dd = DropDown::from_strings(&labels);
    let add = small_btn("Add");
    {
        let state = state.clone();
        let render_ref = render.clone();
        let dd = dd.clone();
        let defs = defs.clone();
        add.connect_clicked(move |_| {
            if let Some((id, _)) = defs.get(dd.selected() as usize) {
                state.borrow_mut().push(id.clone());
                if let Some(f) = render_ref.borrow().clone() {
                    f();
                }
            }
        });
    }
    let add_row = Box::new(Orientation::Horizontal, 8);
    add_row.add_css_class("tz-modadd");
    add_row.append(&dd);
    add_row.append(&add);
    wrap.append(&add_row);

    (wrap, state)
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
    page.append(&hint("Changes apply instantly and persist across reload/relogin (~/.config/tezca/overrides.lua). Reset clears them."));

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
    c.append(&hint(
        "VRR here is the default for every output. A per-monitor setting (Displays → \
         Advanced) overrides it for that monitor.",
    ));
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
        let l = hint("Could not read keybinds.lua.");
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

    // "Action" edits what the bind does (which app / dispatcher); "Rebind"
    // edits the key combo.
    let edit = small_btn("Action");
    edit.add_css_class("tz-rebind");
    {
        let window = window.clone();
        let b = b.clone();
        let on_done = on_done.clone();
        edit.connect_clicked(move |_| edit_action(&window, &b, on_done.clone()));
    }

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
    row.append(&edit);
    row.append(&rebind);
    row
}

/// Modal "set action" dialog: pick an installed app (→ `exec, uwsm app -- <id>`)
/// or type a raw dispatcher+args. Writes through `tezca keybind set-action`,
/// which guards the combo and backs the file up, then reloads Hyprland.
fn edit_action(parent: &Window, b: &keybinds::Bind, on_done: Rc<dyn Fn()>) {
    let dialog = Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Set action")
        .default_width(440)
        .default_height(560)
        .build();
    dialog.add_css_class("tz-capture");

    let v = Box::new(Orientation::Vertical, 10);
    v.set_margin_top(20);
    v.set_margin_bottom(20);
    v.set_margin_start(22);
    v.set_margin_end(22);

    let title = Label::new(Some(&format!("What should “{}” do?", b.combo())));
    title.add_css_class("tz-h2");
    title.set_wrap(true);
    title.set_xalign(0.0);
    let now = Label::new(Some(&format!("Now: {}", b.action)));
    now.add_css_class("tz-hint");
    now.set_wrap(true);
    now.set_xalign(0.0);
    v.append(&title);
    v.append(&now);

    let status = Label::new(None);
    status.add_css_class("tz-hint");
    status.set_wrap(true);
    status.set_xalign(0.0);

    // --- App picker ---------------------------------------------------------
    let search = Entry::new();
    search.set_placeholder_text(Some("Search apps…"));
    search.add_css_class("tz-search");
    v.append(&search);

    let list = Box::new(Orientation::Vertical, 2);
    list.add_css_class("tz-applist");
    let scroller = ScrolledWindow::new();
    scroller.set_hscrollbar_policy(PolicyType::Never);
    scroller.set_vexpand(true);
    scroller.set_child(Some(&list));
    v.append(&scroller);

    let apps = Rc::new(installed_apps());
    let populate: Rc<dyn Fn(&str)> = {
        let (list, apps, dialog, b, on_done, status) =
            (list.clone(), apps.clone(), dialog.clone(), b.clone(), on_done.clone(), status.clone());
        Rc::new(move |filter: &str| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let f = filter.trim().to_lowercase();
            for (name, id) in apps
                .iter()
                .filter(|(n, _)| f.is_empty() || n.to_lowercase().contains(&f))
                .take(300)
            {
                let btn = Button::with_label(name);
                btn.add_css_class("tz-approw");
                btn.set_halign(Align::Fill);
                if let Some(child) = btn.child() {
                    child.set_halign(Align::Start);
                }
                let (id, name) = (id.clone(), name.clone());
                let (dialog, b, on_done, status) =
                    (dialog.clone(), b.clone(), on_done.clone(), status.clone());
                btn.connect_clicked(move |_| {
                    let action = format!("exec, uwsm app -- {id}");
                    apply_action(&dialog, &b, &action, Some(&name), &on_done, &status);
                });
                list.append(&btn);
            }
        })
    };
    populate("");
    {
        let (populate, search2) = (populate.clone(), search.clone());
        search.connect_changed(move |_| populate(&search2.text()));
    }

    // --- Custom action ------------------------------------------------------
    let clbl = Label::new(Some("…or a custom action (dispatcher, args):"));
    clbl.add_css_class("tz-hint");
    clbl.set_xalign(0.0);
    v.append(&clbl);

    let crow = Box::new(Orientation::Horizontal, 8);
    let custom = Entry::new();
    custom.set_text(&b.action);
    custom.set_hexpand(true);
    let setbtn = Button::with_label("Set");
    setbtn.add_css_class("tz-primary");
    let apply_custom: Rc<dyn Fn()> = {
        let (dialog, b, on_done, status, custom) =
            (dialog.clone(), b.clone(), on_done.clone(), status.clone(), custom.clone());
        Rc::new(move || {
            let a = custom.text().trim().to_string();
            if !a.is_empty() {
                apply_action(&dialog, &b, &a, None, &on_done, &status);
            }
        })
    };
    {
        let ac = apply_custom.clone();
        setbtn.connect_clicked(move |_| ac());
        custom.connect_activate(move |_| apply_custom());
    }
    crow.append(&custom);
    crow.append(&setbtn);
    v.append(&crow);
    v.append(&status);
    dialog.set_child(Some(&v));

    // Esc cancels; other keys fall through to the entries.
    let keyctl = EventControllerKey::new();
    {
        let dialog = dialog.clone();
        keyctl.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gdk::Key::Escape {
                dialog.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    dialog.add_controller(keyctl);

    dialog.present();
    search.grab_focus();
}

/// Run `keybind set-action` for one bind; on success close + reload, else show
/// the error inline.
fn apply_action(
    dialog: &Window,
    b: &keybinds::Bind,
    action: &str,
    desc: Option<&str>,
    on_done: &Rc<dyn Fn()>,
    status: &Label,
) {
    let line = b.line.to_string();
    let mut args: Vec<&str> = vec![
        "keybind", "set-action",
        "--line", &line,
        "--action", action,
        "--expect-mods", &b.mods,
        "--expect-key", &b.key,
    ];
    if let Some(d) = desc {
        args.push("--desc");
        args.push(d);
    }
    let res = backend::tezca_result(&args);
    if res.code == 0 {
        dialog.close();
        on_done();
    } else {
        status.set_text(&format!("Error: {}", res.stderr));
        status.remove_css_class("tz-hint");
        status.add_css_class("tz-warn");
    }
}

/// Installed applications as `(display name, desktop id)`, de-duplicated and
/// sorted — the source for the action picker's app list.
fn installed_apps() -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut v: Vec<(String, String)> = Vec::new();
    for a in gio::AppInfo::all() {
        if !a.should_show() {
            continue;
        }
        let Some(id) = a.id() else { continue };
        let id = id.to_string();
        if !id.ends_with(".desktop") || !seen.insert(id.clone()) {
            continue;
        }
        let name = a.display_name().to_string();
        if !name.is_empty() {
            v.push((name, id));
        }
    }
    v.sort_by_key(|x| x.0.to_lowercase());
    v
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
// Sound — output/input device switching and levels
// ===========================================================================

pub fn sound() -> Widget {
    let (page, status) = page_with_status();
    let container = Box::new(Orientation::Vertical, 0);
    let rebuild: RenderCell = Rc::new(RefCell::new(None));
    {
        let c = container.clone();
        let st = status.clone();
        let rb = rebuild.clone();
        *rebuild.borrow_mut() = Some(Rc::new(move || {
            while let Some(child) = c.first_child() {
                c.remove(&child);
            }
            populate_sound(&c, &st, &rb);
        }));
    }
    populate_sound(&container, &status, &rebuild);
    page.append(&container);
    scrolled(&page)
}

fn populate_sound(c: &Box, status: &Status, rebuild: &RenderCell) {
    let st = backend::flat(&backend::tezca_out(&["audio", "status", "--machine"]).unwrap_or_default());
    let get = |k: &str| st.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();

    // --- Levels -------------------------------------------------------------
    c.append(&section_header("Levels"));
    for (label, key, flag) in
        [("Output volume", "output_volume", ""), ("Input volume", "input_volume", "--input")]
    {
        let sl = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
        sl.set_hexpand(true);
        sl.set_draw_value(true);
        sl.set_value(get(key).parse().unwrap_or(0.0));
        let flag = flag.to_string();
        debounce_scale(&sl, 150, move |v| {
            let val = (v as u32).to_string();
            if flag.is_empty() {
                backend::tezca(&["audio", "volume", &val]);
            } else {
                backend::tezca(&["audio", "volume", &val, &flag]);
            }
        });
        c.append(&control_row(label, &sl));
    }
    for (label, key, flag) in
        [("Mute output", "output_muted", ""), ("Mute input", "input_muted", "--input")]
    {
        let sw = Switch::new();
        sw.set_valign(Align::Center);
        sw.set_active(get(key) == "true");
        let flag = flag.to_string();
        let stt = status.clone();
        sw.connect_state_set(move |_, on| {
            let state = if on { "on" } else { "off" };
            let r = if flag.is_empty() {
                backend::tezca_result(&["audio", "mute", state])
            } else {
                backend::tezca_result(&["audio", "mute", state, &flag])
            };
            if !r.ok() {
                stt.err(&r.message());
            }
            glib::Propagation::Proceed
        });
        c.append(&control_row(label, &sw));
    }

    // --- Devices ------------------------------------------------------------
    for (title, sub, setter, empty) in [
        ("Output device", "outputs", "set-output", "No output devices found."),
        ("Input device", "inputs", "set-input", "No input devices found."),
    ] {
        c.append(&section_header(title));
        let out = backend::tezca_out(&["audio", sub, "--machine"]).unwrap_or_default();
        let devices = backend::records(&out);
        if devices.is_empty() {
            c.append(&hint(empty));
            continue;
        }
        // A CheckButton group is the honest widget for "exactly one of these":
        // the radio behaviour comes free, including deselecting the old one.
        let mut group: Option<gtk4::CheckButton> = None;
        for d in &devices {
            let name = backend::rec(d, "name");
            let desc = backend::rec(d, "description");
            let is_default = backend::rec_bool(d, "default");

            let btn = gtk4::CheckButton::with_label(&desc);
            if let Some(g) = &group {
                btn.set_group(Some(g));
            } else {
                group = Some(btn.clone());
            }
            // Set state before connecting, so seeding never fires an apply.
            btn.set_active(is_default);
            {
                let stt = status.clone();
                let rb = rebuild.clone();
                let (name, desc, setter) = (name.clone(), desc.clone(), setter.to_string());
                btn.connect_toggled(move |b| {
                    if !b.is_active() {
                        return;
                    }
                    let r = backend::tezca_result(&["audio", &setter, &name]);
                    stt.report(&r, &format!("Now using {desc}."));
                    redraw(&rb);
                });
            }
            c.append(&btn);
        }
    }
    c.append(&hint(
        "Switching the output also moves whatever is already playing — otherwise the change \
         only affects the next thing you start, which reads as nothing happening.",
    ));
}

// ===========================================================================
// Power — idle timeouts and keep-awake
// ===========================================================================

pub fn power() -> Widget {
    let (page, status) = page_with_status();
    let container = Box::new(Orientation::Vertical, 0);
    let rebuild: RenderCell = Rc::new(RefCell::new(None));
    {
        let c = container.clone();
        let st = status.clone();
        let rb = rebuild.clone();
        *rebuild.borrow_mut() = Some(Rc::new(move || {
            while let Some(child) = c.first_child() {
                c.remove(&child);
            }
            populate_power(&c, &st, &rb);
        }));
    }
    populate_power(&container, &status, &rebuild);
    page.append(&container);
    scrolled(&page)
}

/// Minutes offered for each idle timeout. "Never" is index 0.
const IDLE_CHOICES: &[(&str, u32)] = &[
    ("Never", 0),
    ("1 minute", 60),
    ("2 minutes", 120),
    ("5 minutes", 300),
    ("10 minutes", 600),
    ("15 minutes", 900),
    ("30 minutes", 1800),
    ("1 hour", 3600),
    ("2 hours", 7200),
];

fn populate_power(c: &Box, status: &Status, rebuild: &RenderCell) {
    let st = backend::flat(&backend::tezca_out(&["idle", "status", "--machine"]).unwrap_or_default());
    let get = |k: &str| st.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();

    c.append(&section_header("Idle timeouts"));
    c.append(&hint(
        "Tezca ships an \"always on\" profile — nothing locks or blanks by itself. Anything \
         you set here is written to ~/.config/tezca/hypridle.conf; delete that file to get \
         the shipped behaviour back.",
    ));

    for (label, key, flag) in [
        ("Lock the session", "lock", "--lock"),
        ("Turn off the screens", "dpms", "--dpms"),
        ("Suspend", "suspend", "--suspend"),
    ] {
        let labels: Vec<&str> = IDLE_CHOICES.iter().map(|(l, _)| *l).collect();
        let dd = DropDown::from_strings(&labels);
        let current: u32 = get(key).parse().unwrap_or(0);
        let idx = IDLE_CHOICES.iter().position(|(_, v)| *v == current).unwrap_or(0);
        dd.set_selected(idx as u32);
        {
            let stt = status.clone();
            let rb = rebuild.clone();
            let flag = flag.to_string();
            let label = label.to_string();
            dd.connect_selected_notify(move |d| {
                let (_, secs) = IDLE_CHOICES[d.selected() as usize];
                let value = if secs == 0 { "off".to_string() } else { secs.to_string() };
                let r = backend::tezca_result(&["idle", "set", &flag, &value]);
                if r.ok() {
                    stt.ok(&format!("{label}: {}.", IDLE_CHOICES[d.selected() as usize].0));
                } else {
                    // The CLI refuses a suspend that would fire before the lock,
                    // which would resume to an unlocked session.
                    stt.err(&r.message());
                    redraw(&rb);
                }
            });
        }
        c.append(&control_row(label, &dd));
    }

    c.append(&section_header("Keep awake"));
    let caffeine = Switch::new();
    caffeine.set_valign(Align::Center);
    caffeine.set_active(get("inhibited") == "true");
    {
        let stt = status.clone();
        caffeine.connect_state_set(move |_, on| {
            let r = backend::tezca_result(&["idle", "inhibit", if on { "on" } else { "off" }]);
            stt.report(
                &r,
                if on { "Holding the session awake." } else { "Idle timers resumed." },
            );
            glib::Propagation::Proceed
        });
    }
    c.append(&control_row("Keep the session awake", &caffeine));
    c.append(&hint(
        "Holds a systemd idle inhibitor for as long as it is on — it survives closing this \
         window. Also available as a bar module (\"Keep awake\").",
    ));

    if get("running") != "true" {
        c.append(&hint("hypridle does not appear to be running, so no timeout will fire."));
    }
}

// ===========================================================================
// Startup — what launches at login
// ===========================================================================

pub fn startup(window: &Window) -> Widget {
    let (page, status) = page_with_status();
    let container = Box::new(Orientation::Vertical, 0);
    let rebuild: RenderCell = Rc::new(RefCell::new(None));
    {
        let c = container.clone();
        let w = window.clone();
        let st = status.clone();
        let rb = rebuild.clone();
        *rebuild.borrow_mut() = Some(Rc::new(move || {
            while let Some(child) = c.first_child() {
                c.remove(&child);
            }
            populate_startup(&c, &w, &st, &rb);
        }));
    }
    populate_startup(&container, window, &status, &rebuild);
    page.append(&container);
    scrolled(&page)
}

fn populate_startup(c: &Box, window: &Window, status: &Status, rebuild: &RenderCell) {
    let out = backend::tezca_out(&["startup", "list", "--machine"]).unwrap_or_default();
    // Services and user entries come back in one stream; `@service` / `@entry`
    // open each record, and the CLI emits services first.
    let mut services: Vec<Vec<(String, String)>> = Vec::new();
    let mut entries: Vec<Vec<(String, String)>> = Vec::new();
    {
        let mut is_entry = false;
        for line in out.lines() {
            match line {
                "@service" => {
                    is_entry = false;
                    services.push(Vec::new());
                    continue;
                }
                "@entry" => {
                    is_entry = true;
                    entries.push(Vec::new());
                    continue;
                }
                _ => {}
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let target = if is_entry { entries.last_mut() } else { services.last_mut() };
            if let Some(r) = target {
                r.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }

    // --- Tezca's own services ----------------------------------------------
    c.append(&section_header("Tezca services"));
    c.append(&hint(
        "What the desktop starts for itself. Turning one off takes effect at your next login.",
    ));
    for s in &services {
        let id = backend::rec(s, "id");
        let label = backend::rec(s, "label");
        let enabled = backend::rec_bool(s, "enabled");
        let essential = backend::rec_bool(s, "essential");
        let cost = backend::rec(s, "cost");

        let row = Box::new(Orientation::Horizontal, 12);
        row.add_css_class("tz-switchrow");
        let l = Label::new(Some(&label));
        l.set_halign(Align::Start);
        l.set_hexpand(true);
        l.set_xalign(0.0);
        let sw = Switch::new();
        sw.set_valign(Align::Center);
        sw.set_active(enabled);
        if essential {
            // Not merely discouraged: without the bar there is no route back to
            // this window to switch it on again, and the CLI refuses too.
            sw.set_sensitive(false);
            sw.set_tooltip_text(Some("Required — the menubar is how you get back here."));
        } else {
            let stt = status.clone();
            let rb = rebuild.clone();
            let id = id.clone();
            let label = label.clone();
            let cost = cost.clone();
            sw.connect_state_set(move |_, on| {
                let sub = if on { "enable" } else { "disable" };
                let r = backend::tezca_result(&["startup", sub, &id]);
                if !r.ok() {
                    stt.err(&r.message());
                } else if on {
                    stt.ok(&format!("{label} will start at your next login."));
                } else if cost.is_empty() {
                    stt.warn(&format!("{label} disabled — takes effect at your next login."));
                } else {
                    stt.warn(&format!("{label} disabled — {cost}."));
                }
                redraw(&rb);
                glib::Propagation::Proceed
            });
        }
        row.append(&l);
        row.append(&sw);
        c.append(&row);
        if !cost.is_empty() && !enabled {
            c.append(&hint(&format!("Off: {cost}.")));
        }
    }

    // --- The user's own apps ------------------------------------------------
    c.append(&section_header("Your apps"));
    if entries.is_empty() {
        c.append(&hint("Nothing yet. Add an application or a command below."));
    }
    for e in &entries {
        let id = backend::rec(e, "id");
        let label = backend::rec(e, "label");
        let exec = backend::rec(e, "exec");
        let enabled = backend::rec_bool(e, "enabled");
        let delay: f64 = backend::rec(e, "delay").parse().unwrap_or(0.0);

        let row = Box::new(Orientation::Horizontal, 10);
        row.add_css_class("tz-pinrow");

        let sw = Switch::new();
        sw.set_valign(Align::Center);
        sw.set_active(enabled);
        {
            let stt = status.clone();
            let rb = rebuild.clone();
            let id = id.clone();
            sw.connect_state_set(move |_, on| {
                let sub = if on { "enable" } else { "disable" };
                let r = backend::tezca_result(&["startup", sub, &id]);
                stt.report(&r, "Saved.");
                redraw(&rb);
                glib::Propagation::Proceed
            });
        }

        let text = Box::new(Orientation::Vertical, 0);
        let name = Label::new(Some(&label));
        name.set_halign(Align::Start);
        name.set_xalign(0.0);
        let cmd = Label::new(Some(&exec));
        cmd.add_css_class("tz-hint");
        cmd.set_halign(Align::Start);
        cmd.set_xalign(0.0);
        cmd.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        text.append(&name);
        text.append(&cmd);
        text.set_hexpand(true);

        let delay_spin = SpinButton::with_range(0.0, 120.0, 1.0);
        delay_spin.set_value(delay);
        delay_spin.set_tooltip_text(Some("Seconds to wait before launching"));
        {
            // Re-adding with the same id is how a delay changes: the CLI has no
            // "modify", and add+remove would reorder the list.
            let stt = status.clone();
            let rb = rebuild.clone();
            let (id, exec, label) = (id.clone(), exec.clone(), label.clone());
            delay_spin.connect_value_changed(move |s| {
                let secs = (s.value() as u32).to_string();
                let _ = backend::tezca_result(&["startup", "remove", &id]);
                let r = backend::tezca_result(&[
                    "startup", "add", &exec, "--id", &id, "--label", &label, "--delay", &secs,
                ]);
                stt.report(&r, &format!("{label} delay set to {secs}s."));
                redraw(&rb);
            });
        }

        let run = small_btn("Run now");
        {
            let stt = status.clone();
            let id = id.clone();
            let label = label.clone();
            run.connect_clicked(move |_| {
                let r = backend::tezca_result(&["startup", "run", &id]);
                stt.report(&r, &format!("Launched {label}."));
            });
        }
        let del = small_btn("✕");
        {
            let stt = status.clone();
            let rb = rebuild.clone();
            let id = id.clone();
            let label = label.clone();
            del.connect_clicked(move |_| {
                let r = backend::tezca_result(&["startup", "remove", &id]);
                stt.report(&r, &format!("Removed {label}."));
                redraw(&rb);
            });
        }
        row.append(&sw);
        row.append(&text);
        row.append(&delay_spin);
        row.append(&run);
        row.append(&del);
        c.append(&row);
    }

    let addrow = Box::new(Orientation::Horizontal, 8);
    addrow.set_halign(Align::End);
    let add_app = small_btn("Add application…");
    {
        let win = window.clone();
        let stt = status.clone();
        let rb = rebuild.clone();
        add_app.connect_clicked(move |_| pick_startup_app(&win, &stt, &rb));
    }
    let add_cmd = small_btn("Add command…");
    {
        let win = window.clone();
        let stt = status.clone();
        let rb = rebuild.clone();
        add_cmd.connect_clicked(move |_| add_startup_command(&win, &stt, &rb));
    }
    addrow.append(&add_cmd);
    addrow.append(&add_app);
    c.append(&addrow);

    // --- XDG autostart ------------------------------------------------------
    let xdg = xdg_autostart_entries();
    if !xdg.is_empty() {
        c.append(&section_header("Also in ~/.config/autostart"));
        let active = xdg_autostart_is_active();
        c.append(&hint(if active {
            "Standard XDG autostart entries. This session runs them, but they are not \
             managed here — edit or delete the .desktop files directly."
        } else {
            "Standard XDG autostart entries found on disk. This session does NOT appear to \
             run them (no xdg-desktop-autostart.target), so they are listed for information \
             only."
        }));
        for name in xdg {
            c.append(&info_row("Entry", &name));
        }
    }
}

/// Add a startup entry from the installed-application list.
///
/// The CLI never parses `.desktop` files — it stays std-only. The GUI already has
/// GIO's application index (it powers the keybind editor's action picker), so it
/// resolves the choice here and hands the CLI `uwsm app -- <id>.desktop`, which
/// uwsm knows how to launch natively.
fn pick_startup_app(window: &Window, status: &Status, rebuild: &RenderCell) {
    let dialog = Window::builder()
        .transient_for(window)
        .modal(true)
        .title("Add a startup application")
        .default_width(460)
        .default_height(520)
        .build();
    dialog.add_css_class("tz-capture");

    let b = Box::new(Orientation::Vertical, 10);
    b.set_margin_top(16);
    b.set_margin_bottom(16);
    b.set_margin_start(18);
    b.set_margin_end(18);

    let search = Entry::new();
    search.set_placeholder_text(Some("Search applications"));
    b.append(&search);

    let list = Box::new(Orientation::Vertical, 2);
    list.add_css_class("tz-applist");
    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    b.append(&scroll);
    dialog.set_child(Some(&b));

    let apps = Rc::new(installed_apps());
    let render: Rc<dyn Fn(&str)> = {
        let (list, apps, dialog, status, rebuild) =
            (list.clone(), apps.clone(), dialog.clone(), status.clone(), rebuild.clone());
        Rc::new(move |filter: &str| {
            while let Some(ch) = list.first_child() {
                list.remove(&ch);
            }
            let f = filter.to_lowercase();
            for (name, id) in apps.iter().filter(|(n, _)| n.to_lowercase().contains(&f)).take(200) {
                let btn = Button::with_label(name);
                btn.add_css_class("tz-approw");
                if let Some(ch) = btn.child() {
                    ch.set_halign(Align::Start);
                }
                let (id, name) = (id.clone(), name.clone());
                let (dialog, status, rebuild) =
                    (dialog.clone(), status.clone(), rebuild.clone());
                btn.connect_clicked(move |_| {
                    let r = backend::tezca_result(&[
                        "startup", "add", "--desktop", &id, "--label", &name,
                    ]);
                    dialog.close();
                    status.report(&r, &format!("{name} will start at your next login."));
                    redraw(&rebuild);
                });
                list.append(&btn);
            }
        })
    };
    render("");
    {
        let render = render.clone();
        search.connect_changed(move |e| render(&e.text()));
    }
    dialog.present();
    search.grab_focus();
}

/// Add a startup entry from a free-text command line.
fn add_startup_command(window: &Window, status: &Status, rebuild: &RenderCell) {
    let dialog = Window::builder()
        .transient_for(window)
        .modal(true)
        .title("Add a startup command")
        .default_width(460)
        .resizable(false)
        .build();
    dialog.add_css_class("tz-capture");

    let b = Box::new(Orientation::Vertical, 10);
    b.set_margin_top(16);
    b.set_margin_bottom(16);
    b.set_margin_start(18);
    b.set_margin_end(18);

    let cmd = Entry::new();
    cmd.set_placeholder_text(Some("e.g. uwsm app -- syncthing serve --no-browser"));
    cmd.set_activates_default(true);
    let label = Entry::new();
    label.set_placeholder_text(Some("Name (optional)"));
    let delay = SpinButton::with_range(0.0, 120.0, 1.0);

    b.append(&Label::new(Some("Command")));
    b.append(&cmd);
    b.append(&Label::new(Some("Name")));
    b.append(&label);
    b.append(&control_row("Delay (seconds)", &delay));
    b.append(&hint(
        "A delay is worth setting for anything that wants the menubar and its tray up first.",
    ));

    let row = Box::new(Orientation::Horizontal, 8);
    row.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let ok = Button::with_label("Add");
    ok.add_css_class("tz-action");
    row.append(&cancel);
    row.append(&ok);
    b.append(&row);
    dialog.set_child(Some(&b));

    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    let go: Rc<dyn Fn()> = {
        let (cmd, label, delay, dialog, status, rebuild) = (
            cmd.clone(),
            label.clone(),
            delay.clone(),
            dialog.clone(),
            status.clone(),
            rebuild.clone(),
        );
        Rc::new(move || {
            let command = cmd.text().to_string();
            if command.trim().is_empty() {
                return;
            }
            let secs = (delay.value() as u32).to_string();
            let mut args: Vec<String> =
                vec!["startup".into(), "add".into(), command, "--delay".into(), secs];
            let name = label.text().to_string();
            if !name.trim().is_empty() {
                args.push("--label".into());
                args.push(name);
            }
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            let r = backend::tezca_result(&argv);
            dialog.close();
            status.report(&r, "Added — it starts at your next login.");
            redraw(&rebuild);
        })
    };
    {
        let go = go.clone();
        ok.connect_clicked(move |_| go());
    }
    {
        let go = go.clone();
        cmd.connect_activate(move |_| go());
    }
    dialog.present();
    cmd.grab_focus();
}

/// `.desktop` files in ~/.config/autostart, by display name.
fn xdg_autostart_entries() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
    let dir = PathBuf::from(home).join(".config/autostart");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("desktop") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let name = text
            .lines()
            .find_map(|l| l.strip_prefix("Name="))
            .map(str::to_string)
            .or_else(|| p.file_stem().and_then(|s| s.to_str()).map(str::to_string));
        if let Some(n) = name {
            out.push(n);
        }
    }
    out.sort();
    out
}

/// Whether this session actually runs XDG autostart entries.
///
/// Worth checking rather than assuming: uwsm sessions do not necessarily pull
/// `xdg-desktop-autostart.target`, and telling someone an app starts when it
/// does not is worse than not listing it at all.
fn xdg_autostart_is_active() -> bool {
    backend::output("systemctl", &["--user", "is-active", "xdg-desktop-autostart.target"])
        .map(|s| s.trim() == "active")
        .unwrap_or(false)
}

// ===========================================================================
// Network — Wi-Fi, Bluetooth, VPN, airplane mode
// ===========================================================================

/// Wi-Fi and Bluetooth share a page because they share a mental model ("what am
/// I connected to?") and a kill switch (airplane mode turns off both).
///
/// Everything that talks to hardware here runs through `backend::run_async`. A
/// Wi-Fi rescan takes seconds and a Bluetooth scan takes exactly as long as it is
/// told to; doing either on the main loop freezes the window, and freezing the
/// window is how a scan starts looking like a crash.
pub fn network(window: &Window) -> Widget {
    let (page, status) = page_with_status();
    let container = Box::new(Orientation::Vertical, 0);
    let rebuild: RenderCell = Rc::new(RefCell::new(None));
    {
        let c = container.clone();
        let w = window.clone();
        let st = status.clone();
        let rb = rebuild.clone();
        *rebuild.borrow_mut() = Some(Rc::new(move || {
            while let Some(child) = c.first_child() {
                c.remove(&child);
            }
            populate_network(&c, &w, &st, &rb);
        }));
    }
    populate_network(&container, window, &status, &rebuild);
    page.append(&container);
    scrolled(&page)
}

fn populate_network(c: &Box, window: &Window, status: &Status, rebuild: &RenderCell) {
    let st = backend::flat(&backend::tezca_out(&["net", "status", "--machine"]).unwrap_or_default());
    let get = |k: &str| st.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();

    // --- Current connection -------------------------------------------------
    c.append(&section_header("Connection"));
    match get("kind").as_str() {
        "" => c.append(&info_row("Status", "Disconnected")),
        "wifi" => {
            c.append(&info_row("Wi-Fi", &format!("{} ({}%)", get("ssid"), get("signal"))));
        }
        _ => c.append(&info_row("Wired", &get("device"))),
    }
    for (label, key) in [("IPv4", "ip"), ("Gateway", "gateway"), ("DNS", "dns")] {
        let v = get(key);
        if !v.is_empty() {
            c.append(&info_row(label, &v));
        }
    }
    if !get("vpn").is_empty() {
        c.append(&info_row("VPN", &get("vpn")));
    }

    wifi_section(c, window, status, rebuild, get("wifi_radio") == "true");
    bluetooth_section(c, status, rebuild);
    vpn_section(c, status, rebuild);

    // --- Airplane mode ------------------------------------------------------
    c.append(&section_header("Airplane mode"));
    let air = Button::with_label("Turn every radio off");
    air.add_css_class("tz-action");
    {
        let stt = status.clone();
        let rb = rebuild.clone();
        air.connect_clicked(move |_| {
            let r = backend::tezca_result(&["net", "airplane", "toggle"]);
            stt.report(&r, "Airplane mode toggled.");
            redraw(&rb);
        });
    }
    let arow = Box::new(Orientation::Horizontal, 8);
    arow.set_halign(Align::End);
    arow.append(&air);
    c.append(&arow);
    c.append(&hint("Toggles Wi-Fi and Bluetooth together (nmcli radio all)."));

    if backend::has("nm-connection-editor") {
        let edit = small_btn("Advanced (nm-connection-editor)…");
        edit.connect_clicked(|_| backend::tezca(&["net", "edit"]));
        let erow = Box::new(Orientation::Horizontal, 8);
        erow.set_halign(Align::End);
        erow.append(&edit);
        c.append(&erow);
    }
}

fn wifi_section(
    c: &Box,
    window: &Window,
    status: &Status,
    rebuild: &RenderCell,
    radio_on: bool,
) {
    c.append(&section_header("Wi-Fi"));

    let radio = Switch::new();
    radio.set_valign(Align::Center);
    radio.set_active(radio_on);
    {
        let stt = status.clone();
        let rb = rebuild.clone();
        radio.connect_state_set(move |_, on| {
            let r = backend::tezca_result(&["net", "radio", if on { "on" } else { "off" }]);
            stt.report(&r, if on { "Wi-Fi radio on." } else { "Wi-Fi radio off." });
            redraw(&rb);
            glib::Propagation::Proceed
        });
    }
    c.append(&control_row("Wi-Fi radio", &radio));

    if !radio_on {
        c.append(&hint("The Wi-Fi radio is off. Turn it on to see networks."));
        return;
    }

    // The cached list paints immediately; the rescan is the slow part and is
    // opt-in, because NetworkManager's cache is usually seconds old anyway.
    let list = Box::new(Orientation::Vertical, 0);
    let scan = small_btn("Scan for networks");
    {
        let stt = status.clone();
        let rb = rebuild.clone();
        let scan_btn = scan.clone();
        scan.connect_clicked(move |_| {
            stt.busy("Scanning for networks…");
            scan_btn.set_sensitive(false);
            let stt2 = stt.clone();
            let rb2 = rb.clone();
            let btn2 = scan_btn.clone();
            backend::tezca_async(&["net", "list", "--rescan", "--machine"], move |r| {
                btn2.set_sensitive(true);
                if r.ok() {
                    stt2.clear();
                    redraw(&rb2);
                } else {
                    stt2.err(&r.message());
                }
            });
        });
    }
    let srow = Box::new(Orientation::Horizontal, 8);
    srow.set_halign(Align::End);
    srow.append(&scan);
    c.append(&srow);

    let out = backend::tezca_out(&["net", "list", "--machine"]).unwrap_or_default();
    let aps = backend::records(&out);
    if aps.is_empty() {
        list.append(&hint("No networks found yet — try Scan."));
    }
    for ap in &aps {
        let ssid = backend::rec(ap, "ssid");
        let signal: u32 = backend::rec(ap, "signal").parse().unwrap_or(0);
        let secured = backend::rec(ap, "security") != "open";
        let saved = backend::rec_bool(ap, "saved");
        let active = backend::rec_bool(ap, "active");

        let row = Box::new(Orientation::Horizontal, 10);
        row.add_css_class("tz-pinrow");
        let dot = Label::new(Some(if active { "●" } else { "○" }));
        dot.add_css_class(if active { "tz-ok" } else { "tz-miss" });
        let name = Label::new(Some(&ssid));
        name.set_halign(Align::Start);
        name.set_hexpand(true);
        name.set_xalign(0.0);
        let mut tags: Vec<String> = vec![format!("{signal}%")];
        if secured {
            tags.push("secured".into());
        }
        if saved {
            tags.push("saved".into());
        }
        let meta = Label::new(Some(&tags.join(" · ")));
        meta.add_css_class("tz-hint");
        row.append(&dot);
        row.append(&name);
        row.append(&meta);

        if active {
            let dis = small_btn("Disconnect");
            let stt = status.clone();
            let rb = rebuild.clone();
            dis.connect_clicked(move |_| {
                let r = backend::tezca_result(&["net", "disconnect"]);
                stt.report(&r, "Disconnected.");
                redraw(&rb);
            });
            row.append(&dis);
        } else {
            let go = small_btn("Connect");
            let stt = status.clone();
            let rb = rebuild.clone();
            let win = window.clone();
            let ssid_c = ssid.clone();
            go.connect_clicked(move |_| {
                if saved || !secured {
                    // Nothing to ask for: an open network needs no secret and a
                    // saved one already has its own.
                    stt.busy(&format!("Connecting to {ssid_c}…"));
                    let stt2 = stt.clone();
                    let rb2 = rb.clone();
                    let name = ssid_c.clone();
                    backend::tezca_async(&["net", "connect", &ssid_c], move |r| {
                        stt2.report(&r, &format!("Connected to {name}."));
                        redraw(&rb2);
                    });
                } else {
                    ask_wifi_password(&win, &stt, &ssid_c, &rb);
                }
            });
            row.append(&go);
        }
        if saved {
            let forget = small_btn("✕");
            let stt = status.clone();
            let rb = rebuild.clone();
            let ssid_c = ssid.clone();
            forget.connect_clicked(move |_| {
                let r = backend::tezca_result(&["net", "forget", &ssid_c]);
                stt.report(&r, &format!("Forgot {ssid_c}."));
                redraw(&rb);
            });
            row.append(&forget);
        }
        list.append(&row);
    }
    c.append(&list);
}

/// Ask for a Wi-Fi password and hand it to the CLI **on stdin**.
///
/// The entry's contents never become a command-line argument: `tezca net connect
/// --password-stdin` reads the secret from a pipe, so it is not visible in `ps`
/// to other processes on the machine.
fn ask_wifi_password(window: &Window, status: &Status, ssid: &str, rebuild: &RenderCell) {
    let dialog = Window::builder()
        .transient_for(window)
        .modal(true)
        .title("Wi-Fi password")
        .default_width(400)
        .resizable(false)
        .build();
    dialog.add_css_class("tz-capture");

    let b = Box::new(Orientation::Vertical, 12);
    b.set_margin_top(18);
    b.set_margin_bottom(18);
    b.set_margin_start(20);
    b.set_margin_end(20);

    let title = Label::new(Some(&format!("Connect to {ssid}")));
    title.add_css_class("tz-h2");
    title.set_halign(Align::Start);
    b.append(&title);

    let entry = Entry::new();
    entry.set_visibility(false);
    entry.set_input_purpose(gtk4::InputPurpose::Password);
    entry.set_placeholder_text(Some("Network password"));
    entry.set_activates_default(true);
    b.append(&entry);

    let show = gtk4::CheckButton::with_label("Show password");
    {
        let entry = entry.clone();
        show.connect_toggled(move |b| entry.set_visibility(b.is_active()));
    }
    b.append(&show);

    let row = Box::new(Orientation::Horizontal, 8);
    row.set_halign(Align::End);
    let cancel = Button::with_label("Cancel");
    let connect = Button::with_label("Connect");
    connect.add_css_class("tz-action");
    row.append(&cancel);
    row.append(&connect);
    b.append(&row);
    dialog.set_child(Some(&b));

    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    let go: Rc<dyn Fn()> = {
        let (entry, dialog, status, rebuild) =
            (entry.clone(), dialog.clone(), status.clone(), rebuild.clone());
        let ssid = ssid.to_string();
        Rc::new(move || {
            let secret = entry.text().to_string();
            if secret.is_empty() {
                return;
            }
            dialog.close();
            status.busy(&format!("Connecting to {ssid}…"));
            let st2 = status.clone();
            let rb2 = rebuild.clone();
            let name = ssid.clone();
            backend::tezca_async_stdin(
                &["net", "connect", &ssid, "--password-stdin"],
                secret,
                move |r| {
                    st2.report(&r, &format!("Connected to {name}."));
                    redraw(&rb2);
                },
            );
        })
    };
    {
        let go = go.clone();
        connect.connect_clicked(move |_| go());
    }
    {
        let go = go.clone();
        entry.connect_activate(move |_| go());
    }
    dialog.present();
    entry.grab_focus();
}

fn bluetooth_section(c: &Box, status: &Status, rebuild: &RenderCell) {
    let st = backend::flat(&backend::tezca_out(&["bt", "status", "--machine"]).unwrap_or_default());
    let get = |k: &str| st.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();

    c.append(&section_header("Bluetooth"));
    if get("present") != "true" {
        c.append(&hint("No Bluetooth adapter detected."));
        return;
    }
    let powered = get("powered") == "true";

    let sw = Switch::new();
    sw.set_valign(Align::Center);
    sw.set_active(powered);
    {
        let stt = status.clone();
        let rb = rebuild.clone();
        sw.connect_state_set(move |_, on| {
            let r = backend::tezca_result(&["bt", "power", if on { "on" } else { "off" }]);
            stt.report(&r, if on { "Bluetooth on." } else { "Bluetooth off." });
            redraw(&rb);
            glib::Propagation::Proceed
        });
    }
    c.append(&control_row(&format!("Bluetooth ({})", get("name")), &sw));

    if !powered {
        c.append(&hint("Bluetooth is off. Turn it on to see and connect devices."));
        return;
    }

    let scan = small_btn("Scan for devices");
    {
        let stt = status.clone();
        let rb = rebuild.clone();
        let btn = scan.clone();
        scan.connect_clicked(move |_| {
            stt.busy("Scanning for Bluetooth devices (10 seconds)…");
            btn.set_sensitive(false);
            let stt2 = stt.clone();
            let rb2 = rb.clone();
            let btn2 = btn.clone();
            // Bounded on purpose: `bt scan` passes --timeout to bluetoothctl, so
            // this child cannot outlive the scan.
            backend::tezca_async(&["bt", "scan", "--seconds", "10"], move |r| {
                btn2.set_sensitive(true);
                if r.ok() {
                    stt2.clear();
                    redraw(&rb2);
                } else {
                    stt2.err(&r.message());
                }
            });
        });
    }
    let srow = Box::new(Orientation::Horizontal, 8);
    srow.set_halign(Align::End);
    srow.append(&scan);
    c.append(&srow);

    let out = backend::tezca_out(&["bt", "list", "--all", "--machine"]).unwrap_or_default();
    let devices = backend::records(&out);
    if devices.is_empty() {
        c.append(&hint("No devices yet — put one in pairing mode and press Scan."));
    }
    for d in &devices {
        let mac = backend::rec(d, "mac");
        let name = backend::rec(d, "name");
        let connected = backend::rec_bool(d, "connected");
        let paired = backend::rec_bool(d, "paired");
        let battery = backend::rec(d, "battery");

        let row = Box::new(Orientation::Horizontal, 10);
        row.add_css_class("tz-pinrow");
        let dot = Label::new(Some(if connected { "●" } else { "○" }));
        dot.add_css_class(if connected { "tz-ok" } else { "tz-miss" });
        let l = Label::new(Some(&name));
        l.set_halign(Align::Start);
        l.set_hexpand(true);
        l.set_xalign(0.0);
        let mut tags: Vec<String> = Vec::new();
        if !battery.is_empty() {
            tags.push(format!("{battery}%"));
        }
        if !paired {
            tags.push("not paired".into());
        }
        let meta = Label::new(Some(&tags.join(" · ")));
        meta.add_css_class("tz-hint");
        row.append(&dot);
        row.append(&l);
        row.append(&meta);

        let action = if connected {
            "disconnect"
        } else if paired {
            "connect"
        } else {
            "pair"
        };
        let btn = small_btn(match action {
            "disconnect" => "Disconnect",
            "connect" => "Connect",
            _ => "Pair",
        });
        {
            let stt = status.clone();
            let rb = rebuild.clone();
            let mac = mac.clone();
            let name = name.clone();
            let action = action.to_string();
            btn.connect_clicked(move |_| {
                stt.busy(&format!("{action} {name}…"));
                let stt2 = stt.clone();
                let rb2 = rb.clone();
                let done = format!("{name} {action}ed.");
                backend::tezca_async(&["bt", &action, &mac], move |r| {
                    stt2.report(&r, &done);
                    redraw(&rb2);
                });
            });
        }
        row.append(&btn);

        if paired {
            let rm = small_btn("✕");
            let stt = status.clone();
            let rb = rebuild.clone();
            let mac = mac.clone();
            let name = name.clone();
            rm.connect_clicked(move |_| {
                let r = backend::tezca_result(&["bt", "remove", &mac]);
                stt.report(&r, &format!("Removed {name}."));
                redraw(&rb);
            });
            row.append(&rm);
        }
        c.append(&row);
    }
}

fn vpn_section(c: &Box, status: &Status, rebuild: &RenderCell) {
    let out = backend::tezca_out(&["net", "vpn", "list", "--machine"]).unwrap_or_default();
    let vpns = backend::records(&out);
    if vpns.is_empty() {
        return;
    }
    c.append(&section_header("VPN"));
    for v in &vpns {
        let name = backend::rec(v, "name");
        let active = backend::rec_bool(v, "active");
        let sw = Switch::new();
        sw.set_valign(Align::Center);
        sw.set_active(active);
        {
            let stt = status.clone();
            let rb = rebuild.clone();
            let name = name.clone();
            sw.connect_state_set(move |_, on| {
                let sub = if on { "up" } else { "down" };
                let r = backend::tezca_result(&["net", "vpn", sub, &name]);
                stt.report(&r, &format!("{name} {sub}."));
                redraw(&rb);
                glib::Propagation::Proceed
            });
        }
        c.append(&control_row(&name, &sw));
    }
}

// ===========================================================================
// Input — keyboard, mouse, touchpad, cursor
// ===========================================================================

/// Every control here is an ordinary Hyprland option driven through
/// `tezca hypr set`, which applies it live *and* records it in the override
/// store — the same path the Desktop page uses. There is no new machinery; these
/// settings were simply unreachable without hand-editing `conf.d/input.lua`.
///
/// One rule matters more than the rest: **option keys are all underscores**. The
/// Lua schema silently ignores a hyphenated key rather than rejecting it, so
/// `tap-to-click` would render a switch that flips, persists, and changes
/// nothing. Every key below is spelled the way `hyprctl getoption` spells it.
pub fn input() -> Widget {
    let (page, status) = page_with_status();

    page.append(&section_header("Keyboard"));
    let layout = Entry::new();
    layout.set_text(&backend::hypr_get("input:kb_layout").unwrap_or_default());
    layout.set_placeholder_text(Some("us"));
    let variant = Entry::new();
    variant.set_text(&backend::hypr_get("input:kb_variant").unwrap_or_default());
    let options = Entry::new();
    options.set_text(&backend::hypr_get("input:kb_options").unwrap_or_default());
    options.set_placeholder_text(Some("e.g. compose:ralt"));
    page.append(&control_row("Layout", &layout));
    page.append(&control_row("Variant", &variant));
    page.append(&control_row("Options", &options));

    // The layout is the one input setting that can lock you out of your own
    // machine — a bad value and the keyboard stops producing the characters you
    // would need to fix it. So it is applied deliberately rather than on every
    // keystroke, and the message says how to undo it without typing much.
    let apply_kb = Button::with_label("Apply keyboard layout");
    apply_kb.add_css_class("tz-action");
    {
        let (layout, variant, options, st) =
            (layout.clone(), variant.clone(), options.clone(), status.clone());
        apply_kb.connect_clicked(move |_| {
            let prev = backend::hypr_get("input:kb_layout").unwrap_or_else(|| "us".into());
            // Hyprland accepts an empty variant/options, but `tezca hypr set`
            // refuses an empty value outright (it would be an ambiguous unset),
            // so send a single space — which the config parser treats as none.
            let blank = |s: String| if s.trim().is_empty() { " ".to_string() } else { s };
            let l = blank(layout.text().to_string());
            let v = blank(variant.text().to_string());
            let o = blank(options.text().to_string());
            let r = backend::tezca_result(&[
                "hypr",
                "set",
                "input:kb_layout",
                &l,
                "input:kb_variant",
                &v,
                "input:kb_options",
                &o,
            ]);
            if !r.ok() {
                st.err(&r.message());
                return;
            }
            st.warn(&format!(
                "Layout is now “{}”. Type something to check it. If the keyboard is wrong, \
                 undo it from a terminal with:  tezca hypr set input:kb_layout {}",
                l.trim(),
                prev
            ));
        });
    }
    let kbrow = Box::new(Orientation::Horizontal, 8);
    kbrow.set_halign(Align::End);
    kbrow.append(&apply_kb);
    page.append(&kbrow);

    page.append(&control_row("Repeat rate", &spin_opt("input:repeat_rate", 1.0, 100.0, 1.0, 0)));
    page.append(&control_row(
        "Repeat delay (ms)",
        &spin_opt("input:repeat_delay", 100.0, 2000.0, 25.0, 0),
    ));
    page.append(&control_row("Num Lock at login", &switch_opt("input:numlock_by_default")));

    page.append(&section_header("Mouse"));
    let sens = Scale::with_range(Orientation::Horizontal, -1.0, 1.0, 0.05);
    sens.set_hexpand(true);
    sens.set_draw_value(true);
    sens.set_value(
        backend::hypr_get("input:sensitivity").and_then(|v| v.parse().ok()).unwrap_or(0.0),
    );
    debounce_scale(&sens, 250, |v| {
        backend::tezca(&["hypr", "set", "input:sensitivity", &format!("{v:.2}")]);
    });
    page.append(&control_row("Sensitivity", &sens));
    page.append(&hint(
        "0 is the unaccelerated hardware default; the range runs -1 (slower) to 1 (faster).",
    ));

    let accel = DropDown::from_strings(&["Adaptive", "Flat"]);
    let cur_accel = backend::hypr_get("input:accel_profile").unwrap_or_default();
    accel.set_selected(u32::from(cur_accel.starts_with("flat")));
    accel.connect_selected_notify(|d| {
        let v = if d.selected() == 1 { "flat" } else { "adaptive" };
        backend::tezca(&["hypr", "set", "input:accel_profile", v]);
    });
    page.append(&control_row("Acceleration", &accel));
    page.append(&control_row("Force no acceleration", &switch_opt("input:force_no_accel")));

    let follow = DropDown::from_strings(&["Click to focus", "Follows mouse", "Loose", "Full"]);
    follow.set_selected(
        backend::hypr_get("input:follow_mouse")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            .min(3),
    );
    follow.connect_selected_notify(|d| {
        backend::tezca(&["hypr", "set", "input:follow_mouse", &d.selected().to_string()]);
    });
    page.append(&control_row("Focus follows mouse", &follow));

    // --- Touchpad: only worth showing when there is one ---------------------
    if has_touchpad() {
        page.append(&section_header("Touchpad"));
        page.append(&control_row(
            "Natural scrolling",
            &switch_opt("input:touchpad:natural_scroll"),
        ));
        page.append(&control_row("Tap to click", &switch_opt("input:touchpad:tap_to_click")));
        page.append(&control_row(
            "Disable while typing",
            &switch_opt("input:touchpad:disable_while_typing"),
        ));
        let sf = SpinButton::with_range(0.1, 5.0, 0.1);
        sf.set_digits(1);
        if let Some(v) =
            backend::hypr_get("input:touchpad:scroll_factor").and_then(|x| x.parse().ok())
        {
            sf.set_value(v);
        }
        sf.connect_value_changed(|s| {
            backend::tezca(&[
                "hypr",
                "set",
                "input:touchpad:scroll_factor",
                &format!("{:.1}", s.value()),
            ]);
        });
        page.append(&control_row("Scroll speed", &sf));
    }

    page.append(&section_header("Cursor"));
    page.append(&control_row(
        "Hide after (seconds, 0 = never)",
        &spin_opt("cursor:inactive_timeout", 0.0, 600.0, 5.0, 0),
    ));
    page.append(&control_row("Hide while typing", &switch_opt("cursor:hide_on_key_press")));
    page.append(&control_row("Software cursors", &switch_opt("cursor:no_hardware_cursors")));
    page.append(&hint(
        "Software cursors cost a little performance but fix cursor flicker and disappearing \
         pointers on NVIDIA — worth trying if you see either.",
    ));

    page.append(&hint(
        "Per-device overrides (a different sensitivity for one specific mouse) are not editable \
         here — add a device block to ~/.config/hypr/conf.d/local.lua for that.",
    ));
    scrolled(&page)
}

/// True when libinput reports a touchpad. `hyprctl devices` names one, so the
/// whole section stays hidden on a desktop rather than offering four controls
/// that would silently do nothing.
fn has_touchpad() -> bool {
    let Some(out) = backend::output("hyprctl", &["devices"]) else { return false };
    out.lines().any(|l| {
        let l = l.trim().to_lowercase();
        l.contains("touchpad") || l.contains("trackpad")
    })
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
    let bar_toggle = action("Toggle menubar");
    bar_toggle.connect_clicked(|_| backend::run_script("bar-toggle.sh", &[]));
    let dock = action("Restart dock");
    dock.connect_clicked(|_| backend::tezca(&["dock", "restart"]));
    let diag = action("Diagnostics");
    diag.connect_clicked(|_| {
        let t = backend::tezca_bin();
        backend::spawn("alacritty", &["--hold", "-e", t.as_str(), "doctor"]);
    });
    let logout = action("Logout menu");
    logout.connect_clicked(|_| backend::spawn("wlogout", &["-b", "4"]));
    // Recording lives here rather than on its own page: it is a session action,
    // like locking or logging out. The bar's red dot is the actual indicator.
    let recording = backend::flat(
        &backend::tezca_out(&["record", "status", "--machine"]).unwrap_or_default(),
    );
    let is_recording = recording.iter().any(|(k, v)| k == "recording" && v == "true");
    let rec = action(if is_recording { "Stop recording" } else { "Record screen" });
    rec.connect_clicked(|_| backend::tezca(&["record", "toggle"]));
    for b in [&lock, &reload, &bar_toggle, &dock, &rec, &diag, &logout] {
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
// Page status surface
// ===========================================================================

/// The strip a page uses to say what happened.
///
/// Until this existed the GUI had no way to report failure at all: every action
/// went through `backend::tezca()`, which spawns detached with stderr pointed at
/// /dev/null. That is fine for "set rounding to 14" — you can see the corners
/// change — and useless for "connect to this network", where the interesting
/// outcomes are a wrong password, a missing binary, or a polkit refusal, and the
/// visible result of all three is nothing happening.
///
/// Successes fade after a few seconds; failures stay until dismissed.
#[derive(Clone)]
pub struct Status {
    root: gtk4::Revealer,
    frame: Box,
    glyph: Label,
    label: Label,
    close: Button,
    pending: Rc<RefCell<Option<glib::SourceId>>>,
}

/// How long a self-clearing message stays up.
const STATUS_DWELL: Duration = Duration::from_secs(6);

impl Status {
    fn new() -> Status {
        let frame = Box::new(Orientation::Horizontal, 8);
        frame.add_css_class("tz-status");

        let glyph = Label::new(None);
        glyph.add_css_class("tz-status-glyph");
        let label = Label::new(None);
        label.set_halign(Align::Start);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_wrap(true);
        label.set_max_width_chars(72);
        // stderr from a failing tool can be long; keep the page usable.
        label.set_lines(4);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let close = Button::with_label("✕");
        close.add_css_class("tz-statusclose");
        close.set_valign(Align::Center);

        frame.append(&glyph);
        frame.append(&label);
        frame.append(&close);

        let root = gtk4::Revealer::new();
        root.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        root.set_transition_duration(140);
        root.set_child(Some(&frame));

        let s = Status {
            root,
            frame,
            glyph,
            label,
            close: close.clone(),
            pending: Rc::new(RefCell::new(None)),
        };
        {
            let s2 = s.clone();
            close.connect_clicked(move |_| s2.clear());
        }
        s
    }

    /// The widget to put at the top of the page.
    fn widget(&self) -> &gtk4::Revealer {
        &self.root
    }

    pub fn ok(&self, msg: &str) {
        self.show("ok", "✓", msg, true);
    }

    pub fn warn(&self, msg: &str) {
        self.show("warn", "!", msg, true);
    }

    pub fn err(&self, msg: &str) {
        self.show("err", "✕", msg, false);
    }

    /// A message that stays up until something replaces it — for work in flight.
    pub fn busy(&self, msg: &str) {
        self.show("busy", "…", msg, false);
    }

    /// Report a finished command: its own error text on failure, `ok_msg` on success.
    pub fn report(&self, r: &backend::CmdResult, ok_msg: &str) {
        if r.ok() {
            self.ok(ok_msg);
        } else {
            self.err(&r.message());
        }
    }

    pub fn clear(&self) {
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        self.root.set_reveal_child(false);
    }

    fn show(&self, class: &str, glyph: &str, msg: &str, auto_hide: bool) {
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        for c in ["ok", "warn", "err", "busy"] {
            self.frame.remove_css_class(c);
        }
        self.frame.add_css_class(class);
        self.glyph.set_text(glyph);
        self.label.set_text(msg);
        // Nothing to dismiss on a message that dismisses itself.
        self.close.set_visible(!auto_hide);
        self.root.set_reveal_child(true);
        if auto_hide {
            let me = self.clone();
            let id = glib::timeout_add_local_once(STATUS_DWELL, move || {
                *me.pending.borrow_mut() = None;
                me.root.set_reveal_child(false);
            });
            *self.pending.borrow_mut() = Some(id);
        }
    }
}

/// A page box with a status surface already installed at the top.
fn page_with_status() -> (Box, Status) {
    let page = page_box();
    let status = Status::new();
    page.append(status.widget());
    (page, status)
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
