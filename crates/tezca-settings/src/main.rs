//! tezca-settings — the Project:Tezca GTK4 control center (Phase 8).
//!
//! A single-instance obsidian-glass window: sidebar + stack of pages
//! (Appearance, Keybinds, Gaming, System). It owns no state — every action
//! shells out to the `tezca` CLI / hyprctl / the hypr/scripts helpers, so the
//! GUI and the keyboard bindings drive exactly the same code paths.
//!
//! Launched by `tezca settings` (bound to SUPER+SHIFT+A). An optional
//! `--page <appearance|keybinds|gaming|system>` opens straight to a tab.

mod backend;
mod css;
mod keybinds;
mod pages;

use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box, HeaderBar, Label, Orientation, Separator, Stack,
    StackSidebar, StackTransitionType,
};

const APP_ID: &str = "dev.tezca.Settings";

fn main() -> gtk4::glib::ExitCode {
    let start_page = parse_page_arg();
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| css::install());
    app.connect_activate(move |app| build_ui(app, start_page.as_deref()));
    // We parse our own args (above); hand GTK only argv[0] so it never chokes
    // on `--page`.
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
        .default_width(880)
        .default_height(600)
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

    // Pages.
    let stack = Stack::new();
    stack.add_css_class("tz-stack");
    stack.set_transition_type(StackTransitionType::Crossfade);
    stack.set_transition_duration(150);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    // Size to the visible page, not the widest one — otherwise the long
    // Keybinds rows would stretch every tab.
    stack.set_hhomogeneous(false);

    let win: gtk4::Window = window.clone().upcast();
    stack.add_titled(&pages::appearance(&win), Some("appearance"), "Appearance");
    stack.add_titled(&pages::keybinds(), Some("keybinds"), "Keybinds");
    stack.add_titled(&pages::gaming(), Some("gaming"), "Gaming");
    stack.add_titled(&pages::system(), Some("system"), "System");

    let sidebar = StackSidebar::new();
    sidebar.set_stack(&stack);
    sidebar.add_css_class("tz-sidebar");
    sidebar.set_width_request(178);

    let content = Box::new(Orientation::Horizontal, 0);
    content.append(&sidebar);
    content.append(&Separator::new(Orientation::Vertical));
    content.append(&stack);
    window.set_child(Some(&content));

    if let Some(page) = start_page {
        stack.set_visible_child_name(page);
    }
    window.present();
}
