//! System tray — the StatusNotifierItem (SNI) host the bar was missing.
//!
//! Unlike every other data source in this crate, a tray can't be shelled out:
//! it's the freedesktop/KDE SNI protocol over the session D-Bus. So this module
//! is the sole D-Bus citizen, hand-rolled on pure-Rust `zbus` (async-io, no
//! tokio) and run on one background thread — exactly like `hypr::subscribe` —
//! pushing [`TrayUpdate`]s through an `async-channel` into the GTK main loop and
//! taking [`TrayCmd`]s back for clicks.
//!
//! Roles played (all on that one thread / connection):
//!   * **Watcher** — we try to own `org.kde.StatusNotifierWatcher`. If nobody
//!     else has it (the usual Hyprland case) we *serve* it so apps register with
//!     us; if KDE already provides one we fall back to *using* it as a client.
//!   * **Host** — we register as a `StatusNotifierHost` so apps show their icons.
//!   * **Item reader** — per app: icon (themed name or ARGB pixmap), tooltip,
//!     and its `com.canonical.dbusmenu` layout for the right-click menu.
//!
//! Whichever watcher is in play, the loop only ever reacts to its
//! Registered/Unregistered signals + the items' `New*` signals, so both paths
//! converge on one code path. Any app that misbehaves is swallowed per-item;
//! the tray degrades to fewer icons rather than taking the bar down. If the bus
//! is unreachable the thread simply exits and the bar shows no tray, like a
//! missing sensor.

use futures_util::{FutureExt, StreamExt};
use std::collections::HashMap;
use zbus::message::Header;
use zbus::names::BusName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::Connection;

// ── Types crossing the thread boundary ──────────────────────────────────────

/// A resolved icon, kept as raw data so the GTK thread owns widget creation.
#[derive(Clone)]
pub enum TrayIcon {
    /// A themed icon name, plus an optional extra icon-theme search path.
    Named { name: String, theme_path: Option<String> },
    /// A raw ARGB32 (network byte order) frame straight from `IconPixmap`.
    Pixmap { width: i32, height: i32, argb: Vec<u8> },
    None,
}

/// One tray item as the bar renders it (keyed by `bus + object-path`).
#[derive(Clone)]
pub struct TrayItemView {
    pub key: String,
    pub icon: TrayIcon,
    pub tooltip: String,
}

/// A DBusMenu node (recursive) for the right-click popover.
#[derive(Clone)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    /// `Some(checked)` for checkbox/radio items, else `None`.
    pub checked: Option<bool>,
    pub children: Vec<MenuNode>,
}

/// Tray → GTK: the full item list, or a freshly-fetched menu layout.
pub enum TrayUpdate {
    Items(Vec<TrayItemView>),
    Menu { key: String, root: MenuNode },
}

/// GTK → tray: user interactions to dispatch back over D-Bus.
pub enum TrayCmd {
    Activate(String),
    SecondaryActivate(String),
    /// Ask the item to show its own menu — the right-click fallback for apps
    /// whose DBusMenu we can't render ourselves.
    ContextMenu(String),
    MenuClicked { key: String, id: i32 },
}

// ── D-Bus proxies (client side) ─────────────────────────────────────────────

#[zbus::proxy(interface = "org.kde.StatusNotifierItem", assume_defaults = false)]
trait StatusNotifierItem {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
    #[zbus(property)]
    fn menu(&self) -> zbus::Result<OwnedObjectPath>;

    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;
}

/// `(id, properties, children-as-variants)` — one DBusMenu layout node.
type RawMenu = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[zbus::proxy(interface = "com.canonical.dbusmenu", assume_defaults = false)]
trait DBusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, RawMenu)>;

    fn event(
        &self,
        id: i32,
        event_id: &str,
        data: &zbus::zvariant::Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_host(&self, service: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;
}

// ── The watcher we serve when nobody else does ──────────────────────────────

#[derive(Default)]
struct Watcher {
    items: std::sync::Mutex<Vec<String>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let full = full_service(service, &sender);
        {
            let mut items = self.items.lock().unwrap();
            if !items.contains(&full) {
                items.push(full.clone());
            }
        }
        let _ = Watcher::status_notifier_item_registered(&emitter, &full).await;
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Watcher::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ── Thread entry point ──────────────────────────────────────────────────────

/// Spawn the tray on its own thread. Silent no-op if the session bus is absent.
pub fn spawn(updates: async_channel::Sender<TrayUpdate>, cmds: async_channel::Receiver<TrayCmd>) {
    std::thread::spawn(move || {
        if let Err(e) = zbus::block_on(run(updates, cmds)) {
            eprintln!("tezca-bar: tray disabled ({e})");
        }
    });
}

/// Live per-item bookkeeping on the tray thread.
struct Item {
    bus: String,
    path: String,
    unique: String,
    menu_path: Option<String>,
    view: TrayItemView,
}

#[derive(Default)]
struct State {
    order: Vec<String>,
    items: HashMap<String, Item>,
}

async fn run(
    updates: async_channel::Sender<TrayUpdate>,
    cmds: async_channel::Receiver<TrayCmd>,
) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    conn.request_name(host.as_str()).await?;

    // Become the watcher if the seat is empty; otherwise use the incumbent.
    conn.object_server()
        .at("/StatusNotifierWatcher", Watcher::default())
        .await?;
    let owned = matches!(
        conn.request_name_with_flags(
            "org.kde.StatusNotifierWatcher",
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await,
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner)
    );
    if !owned {
        // An external watcher exists — drop ours and act purely as a client.
        let _ = conn.object_server().remove::<Watcher, _>("/StatusNotifierWatcher").await;
    }

    // Register as a host and seed from whatever the live watcher already knows.
    if let Ok(w) = StatusNotifierWatcherProxy::new(&conn).await {
        let _ = w.register_status_notifier_host(&host).await;
        if let Ok(seed) = w.registered_status_notifier_items().await {
            let mut state = State::default();
            for svc in seed {
                add_item(&conn, &mut state, &updates, &svc).await;
            }
            return pump(conn, updates, cmds, state).await;
        }
    }
    pump(conn, updates, cmds, State::default()).await
}

/// The select loop: merge every relevant signal stream + the command channel.
async fn pump(
    conn: Connection,
    updates: async_channel::Sender<TrayUpdate>,
    cmds: async_channel::Receiver<TrayCmd>,
    mut state: State,
) -> zbus::Result<()> {
    let signal = zbus::message::Type::Signal;
    let rule = |iface: &str| {
        zbus::MatchRule::builder()
            .msg_type(signal)
            .interface(iface.to_string())
            .expect("valid interface")
            .build()
    };
    let mut streams = futures_util::stream::select_all(vec![
        zbus::MessageStream::for_match_rule(rule("org.kde.StatusNotifierWatcher"), &conn, None).await?,
        zbus::MessageStream::for_match_rule(rule("org.kde.StatusNotifierItem"), &conn, None).await?,
        zbus::MessageStream::for_match_rule(rule("com.canonical.dbusmenu"), &conn, None).await?,
        zbus::MessageStream::for_match_rule(rule("org.freedesktop.DBus"), &conn, None).await?,
    ]);

    loop {
        futures_util::select! {
            msg = streams.next().fuse() => match msg {
                Some(Ok(m)) => on_signal(&conn, &mut state, &updates, &m).await,
                _ => {}
            },
            cmd = cmds.recv().fuse() => match cmd {
                Ok(c) => on_cmd(&conn, &state, c).await,
                Err(_) => return Ok(()), // GTK side gone → shut the tray down.
            },
        }
    }
}

// ── Signal handling ─────────────────────────────────────────────────────────

async fn on_signal(
    conn: &Connection,
    state: &mut State,
    updates: &async_channel::Sender<TrayUpdate>,
    msg: &zbus::Message,
) {
    let hdr = msg.header();
    let iface = hdr.interface().map(|i| i.to_string()).unwrap_or_default();
    let member = hdr.member().map(|m| m.to_string()).unwrap_or_default();
    let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
    let body = msg.body();

    match (iface.as_str(), member.as_str()) {
        ("org.kde.StatusNotifierWatcher", "StatusNotifierItemRegistered") => {
            if let Ok(svc) = body.deserialize::<String>() {
                add_item(conn, state, updates, &svc).await;
            }
        }
        ("org.kde.StatusNotifierWatcher", "StatusNotifierItemUnregistered") => {
            if let Ok(svc) = body.deserialize::<String>() {
                let (bus, _) = split_service(&svc);
                remove_where(state, updates, |it| it.bus == bus).await;
            }
        }
        // Any New* icon/title/tooltip/status change → re-read that item.
        ("org.kde.StatusNotifierItem", _) => {
            if let Some(key) = state
                .items
                .iter()
                .find(|(_, it)| it.unique == sender)
                .map(|(k, _)| k.clone())
            {
                refresh_item(conn, state, updates, &key).await;
            }
        }
        ("com.canonical.dbusmenu", "LayoutUpdated") => {
            if let Some((key, mp)) = state
                .items
                .iter()
                .find(|(_, it)| it.unique == sender)
                .and_then(|(k, it)| it.menu_path.clone().map(|m| (k.clone(), m)))
            {
                fetch_menu(conn, state, updates, &key, &mp).await;
            }
        }
        // An app vanishing without unregistering (crash) frees its bus name.
        ("org.freedesktop.DBus", "NameOwnerChanged") => {
            if let Ok((name, _old, new)) = body.deserialize::<(String, String, String)>() {
                if new.is_empty() {
                    remove_where(state, updates, |it| it.bus == name || it.unique == name).await;
                }
            }
        }
        _ => {}
    }
}

// ── Item lifecycle ──────────────────────────────────────────────────────────

async fn add_item(
    conn: &Connection,
    state: &mut State,
    updates: &async_channel::Sender<TrayUpdate>,
    service: &str,
) {
    let (bus, path) = split_service(service);
    let key = format!("{bus}{path}");
    let unique = name_owner(conn, &bus).await.unwrap_or_else(|| bus.clone());

    let Ok(proxy) = StatusNotifierItemProxy::builder(conn)
        .destination(bus.clone())
        .and_then(|b| b.path(path.clone()))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
    else {
        return;
    };
    let Ok(proxy) = proxy.build().await else { return };

    let menu_path = proxy
        .menu()
        .await
        .ok()
        .map(|o| o.to_string())
        .filter(|s| !s.is_empty() && s != "/");
    let (icon, tooltip) = read_item(&proxy).await;
    let view = TrayItemView { key: key.clone(), icon, tooltip };

    if !state.items.contains_key(&key) {
        state.order.push(key.clone());
    }
    state.items.insert(key.clone(), Item { bus, path, unique, menu_path: menu_path.clone(), view });
    emit_items(state, updates).await;

    if let Some(mp) = menu_path {
        fetch_menu(conn, state, updates, &key, &mp).await;
    }
}

async fn refresh_item(
    conn: &Connection,
    state: &mut State,
    updates: &async_channel::Sender<TrayUpdate>,
    key: &str,
) {
    let Some(it) = state.items.get(key) else { return };
    let (bus, path) = (it.bus.clone(), it.path.clone());
    let Ok(builder) = StatusNotifierItemProxy::builder(conn)
        .destination(bus)
        .and_then(|b| b.path(path))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
    else {
        return;
    };
    let Ok(proxy) = builder.build().await else { return };
    let (icon, tooltip) = read_item(&proxy).await;
    if let Some(it) = state.items.get_mut(key) {
        it.view.icon = icon;
        it.view.tooltip = tooltip;
    }
    emit_items(state, updates).await;
}

/// Read the icon (name preferred, pixmap fallback) and tooltip of one item.
async fn read_item(proxy: &StatusNotifierItemProxy<'_>) -> (TrayIcon, String) {
    let name = proxy.icon_name().await.unwrap_or_default();
    let icon = if !name.is_empty() {
        let theme_path = proxy.icon_theme_path().await.ok().filter(|s| !s.is_empty());
        TrayIcon::Named { name, theme_path }
    } else if let Ok(px) = proxy.icon_pixmap().await {
        best_pixmap(px)
            .map(|(width, height, argb)| TrayIcon::Pixmap { width, height, argb })
            .unwrap_or(TrayIcon::None)
    } else {
        TrayIcon::None
    };
    let mut tip = proxy.title().await.unwrap_or_default();
    if tip.is_empty() {
        tip = proxy.id().await.unwrap_or_default();
    }
    (icon, tip)
}

async fn remove_where(
    state: &mut State,
    updates: &async_channel::Sender<TrayUpdate>,
    pred: impl Fn(&Item) -> bool,
) {
    let gone: Vec<String> =
        state.items.iter().filter(|(_, it)| pred(it)).map(|(k, _)| k.clone()).collect();
    if gone.is_empty() {
        return;
    }
    for k in gone {
        state.items.remove(&k);
        state.order.retain(|o| o != &k);
    }
    emit_items(state, updates).await;
}

async fn emit_items(state: &State, updates: &async_channel::Sender<TrayUpdate>) {
    let views = state.order.iter().filter_map(|k| state.items.get(k)).map(|it| it.view.clone()).collect();
    let _ = updates.send(TrayUpdate::Items(views)).await;
}

async fn fetch_menu(
    conn: &Connection,
    state: &State,
    updates: &async_channel::Sender<TrayUpdate>,
    key: &str,
    menu_path: &str,
) {
    let Some(it) = state.items.get(key) else { return };
    let Ok(builder) =
        DBusMenuProxy::builder(conn).destination(it.bus.clone()).and_then(|b| b.path(menu_path.to_string()))
    else {
        return;
    };
    let Ok(menu) = builder.build().await else { return };
    let _ = menu.about_to_show(0).await;
    if let Ok((_rev, raw)) = menu.get_layout(0, -1, &[]).await {
        let _ = updates.send(TrayUpdate::Menu { key: key.to_string(), root: parse_menu(raw) }).await;
    }
}

// ── Command handling (GTK → D-Bus) ──────────────────────────────────────────

async fn on_cmd(conn: &Connection, state: &State, cmd: TrayCmd) {
    match cmd {
        TrayCmd::Activate(key) => {
            if let Some(p) = item_proxy(conn, state, &key).await {
                let _ = p.activate(0, 0).await;
            }
        }
        TrayCmd::SecondaryActivate(key) => {
            if let Some(p) = item_proxy(conn, state, &key).await {
                let _ = p.secondary_activate(0, 0).await;
            }
        }
        TrayCmd::ContextMenu(key) => {
            if let Some(p) = item_proxy(conn, state, &key).await {
                let _ = p.context_menu(0, 0).await;
            }
        }
        TrayCmd::MenuClicked { key, id } => {
            let Some(it) = state.items.get(&key) else { return };
            let Some(mp) = it.menu_path.clone() else { return };
            let Ok(builder) =
                DBusMenuProxy::builder(conn).destination(it.bus.clone()).and_then(|b| b.path(mp))
            else {
                return;
            };
            if let Ok(menu) = builder.build().await {
                let _ = menu.event(id, "clicked", &zbus::zvariant::Value::I32(0), 0).await;
            }
        }
    }
}

async fn item_proxy<'a>(
    conn: &'a Connection,
    state: &State,
    key: &str,
) -> Option<StatusNotifierItemProxy<'a>> {
    let it = state.items.get(key)?;
    StatusNotifierItemProxy::builder(conn)
        .destination(it.bus.clone())
        .ok()?
        .path(it.path.clone())
        .ok()?
        .build()
        .await
        .ok()
}

// ── Parsing helpers ─────────────────────────────────────────────────────────

/// Combine a `RegisterStatusNotifierItem` argument with its D-Bus sender into a
/// canonical `busname[/path]` string, per the SNI spec's two calling styles:
/// apps pass either a bare object path (bus name = the D-Bus sender) or a full
/// `busname[/path]` service string that we keep verbatim.
fn full_service(service: &str, sender: &str) -> String {
    if service.starts_with('/') {
        format!("{sender}{service}")
    } else {
        service.to_string()
    }
}

/// Split a canonical service string into `(bus, object_path)`, defaulting the
/// path to `/StatusNotifierItem` when only a bus name was provided.
fn split_service(service: &str) -> (String, String) {
    match service.find('/') {
        Some(i) => (service[..i].to_string(), service[i..].to_string()),
        None => (service.to_string(), "/StatusNotifierItem".to_string()),
    }
}

async fn name_owner(conn: &Connection, bus: &str) -> Option<String> {
    if bus.starts_with(':') {
        return Some(bus.to_string());
    }
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = BusName::try_from(bus).ok()?;
    dbus.get_name_owner(name).await.ok().map(|u| u.to_string())
}

/// Pick the smallest frame ≥18px tall (crisp at bar size), else the largest.
fn best_pixmap(mut frames: Vec<(i32, i32, Vec<u8>)>) -> Option<(i32, i32, Vec<u8>)> {
    frames.retain(|(w, h, b)| *w > 0 && *h > 0 && b.len() as i64 >= (*w as i64) * (*h as i64) * 4);
    frames.sort_by_key(|(_, h, _)| *h);
    if let Some(i) = frames.iter().position(|(_, h, _)| *h >= 18) {
        return Some(frames.swap_remove(i));
    }
    frames.pop()
}

fn parse_menu(raw: RawMenu) -> MenuNode {
    let (id, props, children) = raw;
    let s = |k: &str| props.get(k).and_then(|v| String::try_from(v.clone()).ok());
    let b = |k: &str| props.get(k).and_then(|v| bool::try_from(v.clone()).ok());

    let separator = s("type").as_deref() == Some("separator");
    let toggle = s("toggle-type").filter(|t| !t.is_empty());
    let checked = toggle.map(|_| {
        props.get("toggle-state").and_then(|v| i32::try_from(v.clone()).ok()).unwrap_or(0) == 1
    });
    let kids = children
        .into_iter()
        .filter_map(|c| RawMenu::try_from(c).ok().map(parse_menu))
        .collect();

    MenuNode {
        id,
        label: strip_mnemonic(&s("label").unwrap_or_default()),
        enabled: b("enabled").unwrap_or(true),
        visible: b("visible").unwrap_or(true),
        separator,
        checked,
        children: kids,
    }
}

/// DBusMenu labels mark the accelerator with `_`; drop a single mnemonic marker.
fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    let mut stripped = false;
    while let Some(c) = chars.next() {
        if c == '_' && !stripped {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            stripped = true;
        } else {
            out.push(c);
        }
    }
    out
}
