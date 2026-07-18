//! The four control-center pages. Each returns a scrollable Widget; all real
//! actions shell out through `backend`.

use crate::{backend, keybinds};
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box, Button, ContentFit, FileDialog, FlowBox, Label, Orientation, Picture, PolicyType,
    ScrolledWindow, SelectionMode, Switch, Widget,
};
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Appearance — theme picker + wallpaper
// ---------------------------------------------------------------------------

pub fn appearance(window: &gtk4::Window) -> Widget {
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
        "Curated palettes. Switching re-skins Waybar, kitty, the dock, hyprlock and the launcher live — no restart.",
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
        "Any image becomes a full palette via matugen. Next / Previous walk your wallpaper folder (~/Pictures).",
    ));

    scrolled(&page)
}

// ---------------------------------------------------------------------------
// Keybinds — cheat-sheet parsed from keybinds.conf
// ---------------------------------------------------------------------------

pub fn keybinds() -> Widget {
    let page = page_box();
    let sections = keybinds::load();
    if sections.is_empty() {
        page.append(&hint("Could not read ~/.config/hypr/conf.d/keybinds.conf."));
    }
    for sec in sections {
        page.append(&section_header(&sec.title));
        let list = Box::new(Orientation::Vertical, 0);
        list.add_css_class("tz-keylist");
        for (combo, desc) in sec.binds {
            let row = Box::new(Orientation::Horizontal, 12);
            row.add_css_class("tz-keyrow");
            let k = Label::new(Some(&combo));
            k.add_css_class("tz-key");
            k.set_width_chars(22);
            k.set_xalign(0.0);
            k.set_halign(Align::Start);
            let d = Label::new(Some(&strip_tag(&desc)));
            d.set_hexpand(true);
            d.set_xalign(0.0);
            d.set_halign(Align::Start);
            d.set_wrap(true);
            d.set_max_width_chars(52);
            row.append(&k);
            row.append(&d);
            list.append(&row);
        }
        page.append(&list);
    }
    scrolled(&page)
}

// ---------------------------------------------------------------------------
// Gaming — profile toggle + detected tools
// ---------------------------------------------------------------------------

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
    // (label, binary-to-probe, description) — note the gamemode package ships
    // `gamemoderun`, not a bare `gamemode`.
    for (label, bin, desc) in [
        ("gamemode", "gamemoderun", "gamemoderun — CPU governor + process priorities"),
        ("mangohud", "mangohud", "MangoHud — in-game FPS / frametime overlay"),
        ("gamescope", "gamescope", "gamescope — micro-compositor for VRR & scaling"),
    ] {
        page.append(&status_row(label, backend::has(bin), desc));
    }
    scrolled(&page)
}

// ---------------------------------------------------------------------------
// System — session actions + info
// ---------------------------------------------------------------------------

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
    // First line is "Hyprland 0.55.4 built from …" — keep just "Hyprland 0.55.4"
    // so the full commit blurb doesn't stretch the window.
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
    // Count monitor blocks by their header line (the -j output repeats "id" in
    // nested workspace objects, which would over-count).
    let monitors = backend::output("hyprctl", &["monitors"])
        .map(|s| s.lines().filter(|l| l.starts_with("Monitor ")).count())
        .unwrap_or(0);
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "wayland".to_string());
    page.append(&info_row("Compositor", &compositor));
    page.append(&info_row("Monitors", &monitors.to_string()));
    page.append(&info_row("Session", &session));

    scrolled(&page)
}

// ---------------------------------------------------------------------------
// Small widget helpers
// ---------------------------------------------------------------------------

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
