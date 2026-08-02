//! tezca-settings — the Project:Tezca GTK4 control center.
//!
//! A single-instance obsidian-glass window: a grouped sidebar + a stack of pages
//! (Appearance, Bar, Dock, Displays, Sound, Input, Network, Power, Startup,
//! Keybinds, Gaming, System). It owns no state — every action shells out to the
//! `tezca` CLI / hyprctl / the hypr/scripts helpers, so the GUI and the keyboard
//! bindings drive exactly the same code paths, and the footer shows you which
//! invocation your click just made. Pages are built lazily on first visit (so
//! e.g. the slow DDC brightness probe on the Displays tab never blocks startup).
//!
//! Launched by `tezca settings` (bound to SUPER+SHIFT+A). An optional
//! `--page <appearance|bar|dock|displays|sound|input|network|power|startup|
//! keybinds|gaming|system>` opens straight to a tab. Ctrl+K opens the command
//! palette, which searches every setting rather than only the page names.

mod arrange;
mod backend;
mod css;
mod keybinds;
mod pages;
mod palette;

use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box, Button, EventControllerKey, HeaderBar, Image,
    Label, ListBox, ListBoxRow, Orientation, Overlay, ScrolledWindow, Separator, Stack,
    StackTransitionType, Widget, Window,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

const APP_ID: &str = "dev.tezca.Settings";

/// One sidebar entry: (id, label, symbolic icon, group heading).
///
/// The group is carried per-page rather than as a nested structure so the
/// ListBox stays one flat, index-addressable list — the headings are drawn by
/// `set_header_func`, which needs exactly this.
struct Page {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    group: &'static str,
}

const LOOK: &str = "Look & feel";
const DEVICES: &str = "Devices";
const SYSTEM: &str = "System";

const PAGES: &[Page] = &[
    Page {
        id: "appearance",
        label: "Appearance",
        icon: "applications-graphics-symbolic",
        group: LOOK,
    },
    Page { id: "bar", label: "Bar", icon: "open-menu-symbolic", group: LOOK },
    Page { id: "dock", label: "Dock", icon: "view-grid-symbolic", group: LOOK },
    Page { id: "displays", label: "Displays", icon: "video-display-symbolic", group: DEVICES },
    Page { id: "sound", label: "Sound", icon: "audio-volume-high-symbolic", group: DEVICES },
    Page { id: "input", label: "Input", icon: "input-mouse-symbolic", group: DEVICES },
    Page { id: "network", label: "Network", icon: "network-wireless-symbolic", group: DEVICES },
    Page { id: "power", label: "Power", icon: "battery-symbolic", group: DEVICES },
    Page { id: "startup", label: "Startup", icon: "system-run-symbolic", group: SYSTEM },
    Page { id: "keybinds", label: "Keybinds", icon: "input-keyboard-symbolic", group: SYSTEM },
    Page { id: "gaming", label: "Gaming", icon: "applications-games-symbolic", group: SYSTEM },
    Page { id: "system", label: "System", icon: "emblem-system-symbolic", group: SYSTEM },
];

/// Page ids that no longer exist, and where their content went.
///
/// `desktop` was folded into Appearance ▸ Windows & motion — window gaps and
/// blur are things you reach for while choosing a theme, not a separate errand.
/// The old id stays routable because it is in shipped keybinds and in `tezca
/// settings --page desktop`, and silently opening nothing would be worse than
/// opening the page that now owns those controls.
const ALIASES: &[(&str, &str)] = &[("desktop", "appearance")];

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

/// Row index for a page id, following [`ALIASES`] for retired ids.
fn index_of(id: &str) -> Option<usize> {
    let id = ALIASES.iter().find(|(from, _)| *from == id).map(|(_, to)| *to).unwrap_or(id);
    PAGES.iter().position(|p| p.id == id)
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
        .default_width(1040)
        .default_height(720)
        .build();
    window.add_css_class("tezca-settings");

    // Stack of (initially empty) page placeholders — filled on first visit.
    let stack = Stack::new();
    stack.add_css_class("tz-stack");
    stack.set_transition_type(StackTransitionType::Crossfade);
    stack.set_transition_duration(140);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_hhomogeneous(false);

    let mut placeholders: Vec<Box> = Vec::new();
    for p in PAGES {
        let ph = Box::new(Orientation::Vertical, 0);
        ph.set_hexpand(true);
        ph.set_vexpand(true);
        stack.add_named(&ph, Some(p.id));
        placeholders.push(ph);
    }
    let placeholders = Rc::new(placeholders);
    let built: Rc<RefCell<HashSet<usize>>> = Rc::new(RefCell::new(HashSet::new()));

    // --- sidebar ------------------------------------------------------------
    let nav = ListBox::new();
    nav.add_css_class("tz-nav");
    for p in PAGES {
        nav.append(&nav_row(p.label, p.icon));
    }
    nav.set_header_func(|row, before| {
        let Some(page) = PAGES.get(row.index() as usize) else { return };
        // A heading only where the group changes — the first row always gets one.
        let prev = before.and_then(|b| PAGES.get(b.index() as usize)).map(|p| p.group);
        if prev == Some(page.group) {
            row.set_header(None::<&Widget>);
            return;
        }
        let h = Label::new(Some(page.group));
        h.add_css_class("tz-navgroup");
        h.set_xalign(0.0);
        h.set_halign(Align::Start);
        row.set_header(Some(&h));
    });

    let win_for_build: Window = window.clone().upcast();
    {
        let stack = stack.clone();
        let placeholders = placeholders.clone();
        let built = built.clone();
        nav.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let i = row.index() as usize;
            let Some(page) = PAGES.get(i) else { return };
            stack.set_visible_child_name(page.id);
            if built.borrow_mut().insert(i) {
                let widget = build_page(page.id, &win_for_build);
                placeholders[i].append(&widget);
            }
        });
    }

    let nav_scroll = ScrolledWindow::new();
    nav_scroll.set_child(Some(&nav));
    nav_scroll.set_vexpand(true);
    nav_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let sidebar = Box::new(Orientation::Vertical, 0);
    sidebar.add_css_class("tz-sidebar");
    sidebar.set_size_request(224, -1);
    sidebar.append(&nav_scroll);
    let session_card = session_card();
    sidebar.append(&session_card.0);

    // --- header -------------------------------------------------------------
    let header = HeaderBar::new();
    header.add_css_class("tz-header");
    let title = Box::new(Orientation::Horizontal, 8);
    let brand = Label::new(Some("Tezca"));
    brand.add_css_class("tz-brand");
    let sub = Label::new(Some("Settings"));
    sub.add_css_class("tz-subtitle");
    title.append(&brand);
    title.append(&sub);
    header.pack_start(&title);

    let search = search_button();
    header.set_title_widget(Some(&search));
    window.set_titlebar(Some(&header));

    // --- echo footer --------------------------------------------------------
    let (echo_bar, echo_cmd, echo_state) = echo_footer();

    let content = Box::new(Orientation::Horizontal, 0);
    content.append(&sidebar);
    content.append(&Separator::new(Orientation::Vertical));
    content.append(&stack);

    let column = Box::new(Orientation::Vertical, 0);
    column.append(&content);
    column.append(&echo_bar);

    // The palette floats over everything, so the whole column lives in an
    // Overlay rather than being the window child directly.
    let overlay = Overlay::new();
    overlay.set_child(Some(&column));

    let go_to: Rc<dyn Fn(&str)> = {
        let nav = nav.clone();
        Rc::new(move |id: &str| {
            if let Some(i) = index_of(id) {
                if let Some(row) = nav.row_at_index(i as i32) {
                    nav.select_row(Some(&row));
                }
            }
        })
    };
    let pal = palette::build(go_to.clone());
    overlay.add_overlay(pal.widget());
    window.set_child(Some(&overlay));

    {
        let pal = pal.clone();
        search.connect_clicked(move |_| pal.open());
    }

    // Ctrl+K toggles, Escape dismisses. Capture phase so a page's own entry
    // cannot swallow the shortcut.
    {
        let pal = pal.clone();
        let keys = EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, state| {
            use gtk4::gdk::{Key, ModifierType};
            if state.contains(ModifierType::CONTROL_MASK) && matches!(key, Key::k | Key::K) {
                pal.toggle();
                return gtk4::glib::Propagation::Stop;
            }
            if key == Key::Escape && pal.is_open() {
                pal.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    // Every mutating `tezca` call from any page lands in the footer.
    backend::set_echo_sink(move |e| show_echo(&echo_bar, &echo_cmd, &echo_state, e));

    // Select the requested page (or the first), which triggers its build.
    let start_index = start_page.and_then(index_of).unwrap_or(0);
    if let Some(row) = nav.row_at_index(start_index as i32) {
        nav.select_row(Some(&row));
    }

    window.present();

    // Identity needs `hyprctl` and `tezca display list`; run it after the first
    // frame so the window does not wait on two subprocesses to appear.
    let (host_label, meta_label) = (session_card.1, session_card.2);
    gtk4::glib::idle_add_local_once(move || {
        let (host, meta) = backend::session_summary();
        host_label.set_text(&host);
        meta_label.set_text(&meta);
        meta_label.set_visible(!meta.is_empty());
    });
}

fn build_page(id: &str, window: &Window) -> Widget {
    match id {
        "appearance" => pages::appearance(window),
        "displays" => pages::displays(window),
        "bar" => pages::bar(),
        "dock" => pages::dock(),
        "network" => pages::network(window),
        "startup" => pages::startup(window),
        "sound" => pages::sound(),
        "power" => pages::power(),
        "input" => pages::input(),
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

/// The header's search affordance — a button that reads as a field.
fn search_button() -> Button {
    let b = Button::new();
    b.add_css_class("tz-omni");
    let inner = Box::new(Orientation::Horizontal, 9);
    let icon = Image::from_icon_name("system-search-symbolic");
    icon.add_css_class("tz-omni-icon");
    let label = Label::new(Some("Search all settings…"));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    let key = Label::new(Some("Ctrl K"));
    key.add_css_class("tz-omni-key");
    inner.append(&icon);
    inner.append(&label);
    inner.append(&key);
    b.set_child(Some(&inner));
    b.set_size_request(420, -1);
    b
}

/// Sidebar footer: which machine and session this panel is driving.
///
/// Returned as `(card, hostname label, detail label)` so the caller can fill it
/// in once the compositor has answered.
fn session_card() -> (Box, Label, Label) {
    let card = Box::new(Orientation::Vertical, 6);
    card.add_css_class("tz-session");

    let top = Box::new(Orientation::Horizontal, 7);
    let dot = Box::new(Orientation::Horizontal, 0);
    dot.add_css_class("tz-session-dot");
    dot.set_valign(Align::Center);
    let host = Label::new(Some("…"));
    host.add_css_class("tz-session-host");
    host.set_xalign(0.0);
    host.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    top.append(&dot);
    top.append(&host);

    let meta = Label::new(None);
    meta.add_css_class("tz-session-meta");
    meta.set_xalign(0.0);
    meta.set_visible(false);
    meta.set_wrap(true);

    card.append(&top);
    card.append(&meta);
    (card, host, meta)
}

/// The CLI echo strip. Hidden until the first command runs.
fn echo_footer() -> (Box, Label, Label) {
    let bar = Box::new(Orientation::Horizontal, 10);
    bar.add_css_class("tz-echo");
    bar.set_visible(false);

    let tag = Label::new(Some("CLI"));
    tag.add_css_class("tz-echo-tag");

    let cmd = Label::new(None);
    cmd.add_css_class("tz-echo-cmd");
    cmd.set_hexpand(true);
    cmd.set_xalign(0.0);
    cmd.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    cmd.set_selectable(true);

    let state = Label::new(None);
    state.add_css_class("tz-echo-state");

    bar.append(&tag);
    bar.append(&cmd);
    bar.append(&state);
    (bar, cmd, state)
}

fn show_echo(bar: &Box, cmd: &Label, state: &Label, e: backend::Echo) {
    use backend::EchoState::*;
    cmd.set_text(&format!("$ {}", e.line));
    let (text, class) = match e.state {
        Sent => ("sent", "sent"),
        Running => ("running…", "busy"),
        Applied => ("✓ applied", "ok"),
        Failed => ("✗ failed", "err"),
    };
    state.set_text(text);
    for c in ["sent", "busy", "ok", "err"] {
        state.remove_css_class(c);
    }
    state.add_css_class(class);
    // The one-line strip has no room for a CLI error; the full text is a hover
    // away rather than truncated into nonsense.
    let tip = (!e.detail.is_empty()).then_some(e.detail.as_str());
    state.set_tooltip_text(tip);
    cmd.set_tooltip_text(tip);
    bar.set_visible(true);
}
