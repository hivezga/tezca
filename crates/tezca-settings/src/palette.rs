//! The Ctrl+K command palette — one search field over every setting in the panel.
//!
//! Thirteen pages is more than a sidebar can make findable: "where do I change
//! the workspace numerals" is a question the nav cannot answer, because the
//! answer is three levels down inside Bar. The palette indexes the *settings*
//! rather than the pages, and jumps to the page that owns the one you pick.
//!
//! Hints are read live from the same `tezca … config` calls the pages use, so a
//! row says what the value actually is right now. Where there is no cheap live
//! value the hint describes the control instead — it never invents one.

use crate::backend;
use gtk4::prelude::*;
use gtk4::{
    Align, Box, EventControllerKey, GestureClick, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow, SearchEntry, SelectionMode,
};
use std::cell::RefCell;
use std::rc::Rc;

/// How many matches the list shows before it stops. The palette is for jumping,
/// not for browsing — past this you are better off typing another character.
const MAX_RESULTS: usize = 8;

/// One indexed setting: which page owns it, what it is called, what it is set to.
pub struct Item {
    pub page: &'static str,
    pub page_label: &'static str,
    pub label: &'static str,
    pub hint: String,
}

/// The palette overlay plus the handles needed to drive it.
pub struct Palette {
    scrim: Box,
    entry: SearchEntry,
    list: ListBox,
    items: Rc<RefCell<Vec<Item>>>,
    /// Index into `items` for each currently visible row, so activating row 2
    /// resolves to the right page after filtering.
    shown: Rc<RefCell<Vec<usize>>>,
}

impl Palette {
    /// The widget to hand to [`gtk4::Overlay::add_overlay`].
    pub fn widget(&self) -> &Box {
        &self.scrim
    }

    pub fn is_open(&self) -> bool {
        self.scrim.is_visible()
    }

    /// Rebuild the index, clear the query and take focus.
    ///
    /// The index is rebuilt per open rather than cached: a cached hint is a
    /// value that *was* true, and the whole point of showing it is that it is.
    pub fn open(&self) {
        *self.items.borrow_mut() = index();
        self.entry.set_text("");
        self.refilter();
        self.scrim.set_visible(true);
        self.entry.grab_focus();
    }

    pub fn close(&self) {
        self.scrim.set_visible(false);
    }

    pub fn toggle(&self) {
        if self.is_open() {
            self.close();
        } else {
            self.open();
        }
    }

    /// Repopulate the result rows for the current query.
    fn refilter(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let q = self.entry.text().trim().to_lowercase();
        let items = self.items.borrow();
        let mut shown = Vec::new();
        for (i, it) in items.iter().enumerate() {
            if shown.len() >= MAX_RESULTS {
                break;
            }
            if q.is_empty() || matches(it, &q) {
                self.list.append(&result_row(it));
                shown.push(i);
            }
        }
        *self.shown.borrow_mut() = shown;
        if let Some(row) = self.list.row_at_index(0) {
            self.list.select_row(Some(&row));
        }
    }

    /// Move the selection by `delta`, clamped to the visible rows.
    ///
    /// Selection only — focus stays in the query field throughout. [`MAX_RESULTS`]
    /// rows always fit the list's height, so nothing ever needs scrolling into
    /// view and the selection can be purely visual.
    fn move_selection(&self, delta: i32) {
        let n = self.shown.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.list.selected_row().map(|r| r.index()).unwrap_or(0);
        let next = (cur + delta).clamp(0, n - 1);
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
        }
    }

    /// The page id behind the highlighted row.
    fn selected_page(&self) -> Option<&'static str> {
        let idx = self.list.selected_row()?.index();
        let i = *self.shown.borrow().get(idx as usize)?;
        self.items.borrow().get(i).map(|it| it.page)
    }
}

fn matches(it: &Item, q: &str) -> bool {
    let hay = format!("{} {} {}", it.page_label, it.label, it.hint).to_lowercase();
    q.split_whitespace().all(|word| hay.contains(word))
}

/// Build the palette. `on_go` is called with a page id when a row is activated.
pub fn build(on_go: Rc<dyn Fn(&str)>) -> Rc<Palette> {
    let scrim = Box::new(Orientation::Vertical, 0);
    scrim.add_css_class("tz-palette-scrim");
    scrim.set_visible(false);
    scrim.set_halign(Align::Fill);
    scrim.set_valign(Align::Fill);

    let card = Box::new(Orientation::Vertical, 0);
    card.add_css_class("tz-palette");
    card.set_halign(Align::Center);
    card.set_valign(Align::Start);
    card.set_margin_top(96);
    card.set_size_request(620, -1);

    let entry = SearchEntry::new();
    entry.add_css_class("tz-palette-entry");
    entry.set_placeholder_text(Some("Jump to a setting, theme, or keybind…"));
    card.append(&entry);

    let list = ListBox::new();
    list.add_css_class("tz-palette-list");
    list.set_selection_mode(SelectionMode::Single);
    let scroll = ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(340);
    scroll.set_vexpand(false);
    card.append(&scroll);

    let foot = Box::new(Orientation::Horizontal, 16);
    foot.add_css_class("tz-palette-foot");
    for tip in ["↑↓ navigate", "↵ open", "esc close"] {
        let l = Label::new(Some(tip));
        l.add_css_class("tz-palette-tip");
        foot.append(&l);
    }
    card.append(&foot);

    scrim.append(&card);

    let p = Rc::new(Palette {
        scrim: scrim.clone(),
        entry: entry.clone(),
        list: list.clone(),
        items: Rc::new(RefCell::new(Vec::new())),
        shown: Rc::new(RefCell::new(Vec::new())),
    });

    // --- query -------------------------------------------------------------
    {
        let p = p.clone();
        entry.connect_search_changed(move |_| p.refilter());
    }

    // --- keyboard ----------------------------------------------------------
    // Up/Down/Enter are driven from the entry rather than from the list, so the
    // caret never leaves the field while you are choosing.
    //
    // Capture phase, because GtkSearchEntry's inner text widget turns Return
    // into its own `activate` and swallows it — a bubble-phase controller
    // never sees the keypress that chooses a result.
    {
        let p = p.clone();
        let on_go = on_go.clone();
        let keys = EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            use gtk4::gdk::Key;
            match key {
                Key::Up => p.move_selection(-1),
                Key::Down => p.move_selection(1),
                Key::Escape => p.close(),
                Key::Return | Key::KP_Enter => {
                    if let Some(page) = p.selected_page() {
                        p.close();
                        on_go(page);
                    }
                }
                _ => return gtk4::glib::Propagation::Proceed,
            }
            gtk4::glib::Propagation::Stop
        });
        entry.add_controller(keys);
    }

    // --- activation --------------------------------------------------------
    {
        let p = p.clone();
        let on_go = on_go.clone();
        list.connect_row_activated(move |_, row| {
            let i = row.index() as usize;
            let page =
                p.shown.borrow().get(i).and_then(|i| p.items.borrow().get(*i).map(|x| x.page));
            if let Some(page) = page {
                p.close();
                on_go(page);
            }
        });
    }

    // --- click-away --------------------------------------------------------
    // The card claims its own presses in the capture phase, so a click inside
    // the palette never reaches the scrim's dismiss gesture underneath it.
    {
        let inside = GestureClick::new();
        inside.set_propagation_phase(gtk4::PropagationPhase::Capture);
        inside.connect_pressed(|g, _, _, _| {
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        card.add_controller(inside);

        let p = p.clone();
        let away = GestureClick::new();
        away.connect_pressed(move |_, _, _, _| p.close());
        scrim.add_controller(away);
    }

    p
}

fn result_row(it: &Item) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.add_css_class("tz-palette-row");
    let b = Box::new(Orientation::Horizontal, 12);

    let page = Label::new(Some(it.page_label));
    page.add_css_class("tz-palette-page");
    page.set_xalign(0.0);
    page.set_width_chars(11);
    b.append(&page);

    let label = Label::new(Some(it.label));
    label.add_css_class("tz-palette-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    b.append(&label);

    let hint = Label::new(Some(&it.hint));
    hint.add_css_class("tz-palette-hint");
    hint.set_xalign(1.0);
    hint.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    hint.set_max_width_chars(26);
    b.append(&hint);

    row.set_child(Some(&b));
    row
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Every setting the palette can jump to, with its value read live.
fn index() -> Vec<Item> {
    let bar = backend::bar_config();
    let dock = backend::dock_config();
    let get = |cfg: &[(String, String)], k: &str| -> String {
        cfg.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default()
    };
    let bar_get = |k: &str| get(&bar, k);
    let dock_get = |k: &str| get(&dock, k);

    let mut v: Vec<Item> = Vec::new();
    let mut add =
        |page: &'static str, page_label: &'static str, label: &'static str, hint: String| {
            v.push(Item { page, page_label, label, hint });
        };

    // --- Appearance --------------------------------------------------------
    add(
        "appearance",
        "Appearance",
        "Theme",
        backend::active_theme().unwrap_or_else(|| "derived from wallpaper".into()),
    );
    add(
        "appearance",
        "Appearance",
        "Wallpaper",
        backend::current_wallpaper()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "not set".into()),
    );
    for (label, opt) in [
        ("Inner gaps", "general:gaps_in"),
        ("Outer gaps", "general:gaps_out"),
        ("Border size", "general:border_size"),
        ("Corner rounding", "decoration:rounding"),
        ("Active opacity", "decoration:active_opacity"),
        ("Inactive opacity", "decoration:inactive_opacity"),
        ("Blur", "decoration:blur:enabled"),
        ("Shadows", "decoration:shadow:enabled"),
        ("Animations", "animations:enabled"),
    ] {
        add("appearance", "Appearance", label, on_off(backend::hypr_get(opt)));
    }

    // --- Bar ---------------------------------------------------------------
    add("bar", "Bar", "Bar shape", bar_get("shape"));
    add("bar", "Bar", "Bar height", px(&bar_get("height")));
    add("bar", "Bar", "Workspace numerals", bar_get("workspace_numerals"));
    add("bar", "Bar", "Clock format", bar_get("clock_format"));
    add("bar", "Bar", "On-screen display", on_off(Some(bar_get("osd_enabled"))));
    add("bar", "Bar", "AI usage module", on_off(Some(bar_get("ai_enabled"))));
    add("bar", "Bar", "Bar modules", modules_hint(&bar));

    // --- Dock --------------------------------------------------------------
    add("dock", "Dock", "Icon size", px(&dock_get("icon_size")));
    add("dock", "Dock", "Magnification", scale_hint(&dock_get("max_scale")));
    add("dock", "Dock", "Autohide delay", ms(&dock_get("hide_delay_ms")));

    // --- Displays ----------------------------------------------------------
    let mons = backend::monitors();
    add(
        "displays",
        "Displays",
        "Arrangement",
        format!("{} display{}", mons.len(), if mons.len() == 1 { "" } else { "s" }),
    );
    if let Some(m) = mons.iter().find(|m| !m.disabled) {
        add("displays", "Displays", "Resolution", format!("{} · {}", m.name, m.res));
        add("displays", "Displays", "Refresh rate", format!("{} Hz", m.rate));
    }
    add("displays", "Displays", "Night light", "colour temperature".into());
    add("displays", "Displays", "Layout profiles", "save and restore".into());

    // --- Devices -----------------------------------------------------------
    add("sound", "Sound", "Output volume", "levels and devices".into());
    add("sound", "Sound", "Microphone", "input level and boost".into());
    add("input", "Input", "Keyboard layout", "xkb layout and options".into());
    add("input", "Input", "Repeat rate", "rate and delay".into());
    add("input", "Input", "Cursor size", "and hide-while-typing".into());
    add("input", "Input", "Pointer speed", "acceleration profile".into());
    add("network", "Network", "Wi-Fi", "networks and radio".into());
    add("network", "Network", "Bluetooth", "pair and connect".into());
    add("network", "Network", "VPN", "connections".into());
    add("power", "Power", "Idle timeouts", "dim, lock, suspend".into());
    add("power", "Power", "Caffeine", "hold the idle inhibitor".into());

    // --- System ------------------------------------------------------------
    add("startup", "Startup", "Tezca services", "bar, dock, idle, notify".into());
    add("startup", "Startup", "Your apps", "~/.config/autostart".into());
    add("keybinds", "Keybinds", "Keyboard shortcuts", "capture and rebind".into());
    add("gaming", "Gaming", "Game mode", on_off_bool(backend::game_on()));
    add("system", "System", "Log out", "and lock, suspend, reboot".into());
    add("system", "System", "About this install", "versions and paths".into());

    v
}

/// Hyprland answers with whatever type the option has: `1`/`0` for a bool, but
/// also `0.700000` for an opacity and `12` for a radius. A hint is one glance
/// wide, so booleans read as words and numbers lose their padding.
fn on_off(v: Option<String>) -> String {
    match v.as_deref() {
        Some("1") | Some("true") | Some("on") => "on".into(),
        Some("0") | Some("false") | Some("off") => "off".into(),
        Some(other) if !other.is_empty() => tidy_number(other),
        _ => "—".into(),
    }
}

/// `0.700000` → `0.7`, `1.000000` → `1`, `12` → `12`.
fn tidy_number(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    match s.parse::<f64>() {
        Ok(_) => {
            let t = s.trim_end_matches('0');
            t.strip_suffix('.').unwrap_or(t).to_string()
        }
        Err(_) => s.to_string(),
    }
}

fn on_off_bool(b: bool) -> String {
    if b { "on" } else { "off" }.into()
}

fn px(v: &str) -> String {
    if v.is_empty() {
        "—".into()
    } else {
        format!("{v} px")
    }
}

fn ms(v: &str) -> String {
    if v.is_empty() {
        "—".into()
    } else {
        format!("{v} ms")
    }
}

fn scale_hint(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(f) => format!("{f:.2}×"),
        Err(_) => "—".into(),
    }
}

/// `layout_left` + `layout_center` + `layout_right`, counting real modules.
fn modules_hint(bar: &[(String, String)]) -> String {
    let n: usize = ["layout_left", "layout_center", "layout_right"]
        .iter()
        .filter_map(|k| bar.iter().find(|(key, _)| key == k))
        .flat_map(|(_, v)| v.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "sep")
        .count();
    format!("{n} active")
}
