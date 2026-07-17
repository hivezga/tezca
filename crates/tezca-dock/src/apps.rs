//! The dock model — the ordered list of items the magnifier draws.
//!
//! Pinned favourites (from `dock.toml`) come first, then any running app that
//! isn't pinned. gio's `DesktopAppInfo` isn't bound in this gtk-rs release, so we
//! resolve `.desktop` entries ourselves from `$XDG_DATA_DIRS` — which also keeps
//! the dock self-sufficient. Icons come from the GTK icon theme; a click launches
//! a not-running favourite (via `uwsm app` for the right systemd slice) or
//! focuses/cycles a running app's windows.

use crate::config::Config;
use crate::hypr;
use gtk4::gdk::Paintable;
use gtk4::prelude::*;
use gtk4::{IconLookupFlags, IconTheme, TextDirection};
use std::path::PathBuf;

/// Base pixel size icons are looked up at — generous so magnified SVGs stay crisp.
const LOOKUP_SIZE: i32 = 128;

#[derive(Clone)]
pub struct DockItem {
    pub label: String,
    pub icon: Paintable,
    pub running: bool,
    pub pinned: bool,
    /// Live window addresses for this app (for focus/cycle).
    pub addresses: Vec<String>,
    /// What to hand `uwsm app` when launching; None for running-only items.
    pub launch_id: Option<String>,
    /// Draw a group separator immediately before this item.
    pub divider_before: bool,
}

/// A parsed `.desktop` entry, trimmed to what the dock uses.
struct AppEntry {
    id: String, // desktop id incl. ".desktop"
    name: String,
    icon: Option<String>,
    wmclass: Option<String>,
}

/// Build the current dock model from config + live Hyprland clients.
pub fn build(cfg: &Config, theme: &IconTheme) -> Vec<DockItem> {
    let clients = hypr::clients();

    // Group running windows by class, preserving first-seen order.
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for c in &clients {
        match groups.iter_mut().find(|(cl, _)| cl == &c.class) {
            Some((_, addrs)) => addrs.push(c.address.clone()),
            None => groups.push((c.class.clone(), vec![c.address.clone()])),
        }
    }
    let mut consumed = vec![false; groups.len()];
    let mut items = Vec::new();

    // Pinned favourites, in config order.
    for id in &cfg.pinned {
        let app = resolve_app(id);
        let mut addresses = Vec::new();
        for (i, (class, addrs)) in groups.iter().enumerate() {
            if !consumed[i] && class_matches(class, id, app.as_ref()) {
                consumed[i] = true;
                addresses.extend(addrs.iter().cloned());
            }
        }
        let label = app.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| pretty(id));
        let icon = resolve_icon(theme, app.as_ref(), id);
        let launch_id = app.as_ref().map(|a| a.id.clone()).unwrap_or_else(|| id.clone());
        items.push(DockItem {
            label,
            icon,
            running: !addresses.is_empty(),
            pinned: true,
            addresses,
            launch_id: Some(launch_id),
            divider_before: false,
        });
    }

    // Running apps that aren't pinned — trailing group, separated by a divider.
    let mut first_running = true;
    for (i, (class, addrs)) in groups.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        let app = resolve_app(class);
        let label = app.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| pretty(class));
        let icon = resolve_icon(theme, app.as_ref(), class);
        items.push(DockItem {
            label,
            icon,
            running: true,
            pinned: false,
            addresses: addrs.clone(),
            launch_id: None,
            divider_before: std::mem::take(&mut first_running),
        });
    }

    items
}

// ---------------------------------------------------------------------------
// .desktop resolution
// ---------------------------------------------------------------------------

/// Resolve a desktop-app id or window class to a `.desktop` entry by scanning
/// the XDG application dirs for `<id>.desktop` (case-insensitive on the stem).
fn resolve_app(id: &str) -> Option<AppEntry> {
    let want = format!("{}.desktop", id).to_lowercase();
    for dir in app_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let fname = entry.file_name();
            let Some(fname) = fname.to_str() else { continue };
            if fname.to_lowercase() == want {
                if let Some(app) = parse_desktop(&entry.path(), fname) {
                    return Some(app);
                }
            }
        }
    }
    None
}

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

/// Minimal `[Desktop Entry]` parse for Name / Icon / StartupWMClass.
fn parse_desktop(path: &std::path::Path, id: &str) -> Option<AppEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let (mut name, mut icon, mut wmclass) = (None, None, None);
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(v) = l.strip_prefix("Name=") {
            name.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("Icon=") {
            icon.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("StartupWMClass=") {
            wmclass.get_or_insert_with(|| v.trim().to_string());
        }
    }
    Some(AppEntry {
        id: id.to_string(),
        name: name.unwrap_or_else(|| pretty(id.trim_end_matches(".desktop"))),
        icon,
        wmclass,
    })
}

/// Does a Hyprland window `class` correspond to this pinned id / app?
fn class_matches(class: &str, id: &str, app: Option<&AppEntry>) -> bool {
    let lc = class.to_lowercase();
    let mut cands: Vec<String> = vec![id.to_lowercase(), last_segment(id)];
    if let Some(app) = app {
        let stem = app.id.trim_end_matches(".desktop").to_lowercase();
        cands.push(stem.clone());
        cands.push(last_segment(&stem));
        if let Some(wm) = &app.wmclass {
            cands.push(wm.to_lowercase());
        }
    }
    cands
        .iter()
        .any(|c| !c.is_empty() && (*c == lc || last_segment(c) == last_segment(&lc)))
}

/// Resolve a paintable: prefer the app's `Icon=` name, else the class/id itself,
/// with a generic-executable icon when nothing matches.
///
/// NOTE: pass NO fallback list to `lookup_icon` — supplying one makes this
/// gtk-rs release return the fallback even when the primary name exists. We do
/// our own one-step fallback instead.
fn resolve_icon(theme: &IconTheme, app: Option<&AppEntry>, name: &str) -> Paintable {
    let icon_name = app.and_then(|a| a.icon.clone()).unwrap_or_else(|| name.to_string());
    let ip = lookup(theme, &icon_name);
    let missing = ip
        .icon_name()
        .as_deref()
        .and_then(|p| p.to_str())
        .map(|s| s == "image-missing")
        .unwrap_or(false);
    if missing {
        return lookup(theme, "application-x-executable").upcast();
    }
    ip.upcast()
}

fn lookup(theme: &IconTheme, name: &str) -> gtk4::IconPaintable {
    theme.lookup_icon(name, &[], LOOKUP_SIZE, 1, TextDirection::None, IconLookupFlags::empty())
}

/// `org.gnome.Nautilus` → `Nautilus`; `kitty` → `Kitty`.
fn pretty(id: &str) -> String {
    let seg = id.rsplit('.').next().unwrap_or(id);
    let mut chars = seg.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => seg.to_string(),
    }
}

fn last_segment(s: &str) -> String {
    s.rsplit('.').next().unwrap_or(s).to_lowercase()
}
