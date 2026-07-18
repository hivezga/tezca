//! tezca-settings — the Project:Tezca GTK4 control center.
//!
//! A single-instance obsidian-glass window: an icon sidebar + a stack of pages
//! (Appearance, Displays, Dock, Desktop, Keybinds, Gaming, System). It owns no
//! state — every action shells out to the `tezca` CLI / hyprctl / the
//! hypr/scripts helpers, so the GUI and the keyboard bindings drive exactly the
//! same code paths. Pages are built lazily on first visit (so e.g. the slow DDC
//! brightness probe on the Displays tab never blocks startup).
//!
//! Launched by `tezca settings` (bound to SUPER+SHIFT+A). An optional
//! `--page <appearance|displays|dock|desktop|keybinds|gaming|system>` opens
//! straight to a tab.

mod backend;
mod css;
mod keybinds;
mod pages;

use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box, HeaderBar, Image, Label, ListBox, ListBoxRow,
    Orientation, Separator, Stack, StackTransitionType, Widget, Window,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

const APP_ID: &str = "dev.tezca.Settings";

/// (id, sidebar label, symbolic icon name).
const PAGES: &[(&str, &str, &str)] = &[
    ("appearance", "Appearance", "applications-graphics-symbolic"),
    ("displays", "Displays", "video-display-symbolic"),
    ("dock", "Dock", "view-grid-symbolic"),
    ("desktop", "Desktop", "preferences-desktop-symbolic"),
    ("keybinds", "Keybinds", "input-keyboard-symbolic"),
    ("gaming", "Gaming", "applications-games-symbolic"),
    ("system", "System", "emblem-system-symbolic"),
];

fn main() -> gtk4::glib::ExitCode {
    let start_page = parse_page_arg();
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| css::install());
    app.connect_activate(move |app| build_ui(app, start_page.as_deref()));
    // We parse our own args (above); hand GTK only argv[0] so it never chokes on
    // `--page`.
    let argv: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv)
}

/// Pull `--page NAME` (or `--page=NAME`) out of argv, if present.
fn parse_page_arg() -> Option<String> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--page=") {
            return Some(v.to_string());
        }
        if a == "--page" {
            return it.next();
        }
    }
    None
}

fn build_ui(app: &Application, start_page: Option<&str>) {
    // Single instance: a second launch just raises the open window.
    if let Some(win) = app.active_window() {
        win.present();
        return;
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Tezca Settings")
        .default_width(940)
        .default_height(680)
        .build();
    window.add_css_class("tezca-settings");

    // Header with the brand wordmark.
    let header = HeaderBar::new();
    header.add_css_class("tz-header");
    let title = Box::new(Orientation::Horizontal, 8);
    let brand = Label::new(Some("Tezca"));
    brand.add_css_class("tz-brand");
    let sub = Label::new(Some("Settings"));
    sub.add_css_class("tz-subtitle");
    title.append(&brand);
    title.append(&sub);
    header.set_title_widget(Some(&title));
    window.set_titlebar(Some(&header));

    // Stack of (initially empty) page placeholders — filled on first visit.
    let stack = Stack::new();
    stack.add_css_class("tz-stack");
    stack.set_transition_type(StackTransitionType::Crossfade);
    stack.set_transition_duration(140);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_hhomogeneous(false);

    let mut placeholders: Vec<Box> = Vec::new();
    for (id, _label, _icon) in PAGES {
        let ph = Box::new(Orientation::Vertical, 0);
        ph.set_hexpand(true);
        ph.set_vexpand(true);
        stack.add_named(&ph, Some(id));
        placeholders.push(ph);
    }
    let placeholders = Rc::new(placeholders);
    let built: Rc<RefCell<HashSet<usize>>> = Rc::new(RefCell::new(HashSet::new()));

    // Icon sidebar.
    let sidebar = ListBox::new();
    sidebar.add_css_class("tz-nav");
    sidebar.set_width_request(190);
    for (_id, label, icon) in PAGES {
        sidebar.append(&nav_row(label, icon));
    }

    let win_for_build: Window = window.clone().upcast();
    {
        let stack = stack.clone();
        let placeholders = placeholders.clone();
        let built = built.clone();
        sidebar.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let i = row.index() as usize;
            let Some((id, _, _)) = PAGES.get(i) else { return };
            stack.set_visible_child_name(id);
            if built.borrow_mut().insert(i) {
                let widget = build_page(id, &win_for_build);
                placeholders[i].append(&widget);
            }
        });
    }

    let content = Box::new(Orientation::Horizontal, 0);
    content.append(&sidebar);
    content.append(&Separator::new(Orientation::Vertical));
    content.append(&stack);
    window.set_child(Some(&content));

    // Select the requested page (or the first), which triggers its build.
    let start_index = start_page
        .and_then(|p| PAGES.iter().position(|(id, _, _)| *id == p))
        .unwrap_or(0);
    if let Some(row) = sidebar.row_at_index(start_index as i32) {
        sidebar.select_row(Some(&row));
    }

    window.present();
}

fn build_page(id: &str, window: &Window) -> Widget {
    match id {
        "appearance" => pages::appearance(window),
        "displays" => pages::displays(window),
        "dock" => pages::dock(),
        "desktop" => pages::desktop(),
        "keybinds" => pages::keybinds(window),
        "gaming" => pages::gaming(),
        "system" => pages::system(),
        _ => Label::new(Some("unknown page")).upcast(),
    }
}

fn nav_row(label: &str, icon: &str) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.add_css_class("tz-navrow");
    let b = Box::new(Orientation::Horizontal, 12);
    let img = Image::from_icon_name(icon);
    img.add_css_class("tz-navicon");
    let l = Label::new(Some(label));
    l.set_halign(Align::Start);
    b.append(&img);
    b.append(&l);
    row.set_child(Some(&b));
    row
}
