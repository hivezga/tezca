//! tezca-settings — the Project:Tezca control center.
//!
//! A single-instance obsidian-glass window: a grouped sidebar + a stack of pages
//! (Appearance, Bar, Dock, Displays, Sound, Input, Network, Power, Startup,
//! Keybinds, Gaming, System). It owns no state — every action shells out to the
//! `tezca` CLI / hyprctl / the hypr/scripts helpers, so the GUI and the keyboard
//! bindings drive exactly the same code paths, and the footer shows you which
//! invocation your click just made.
//!
//! Launched by `tezca settings` (bound to SUPER+SHIFT+A). An optional
//! `--page <appearance|bar|dock|displays|sound|input|network|power|startup|
//! keybinds|gaming|system>` opens straight to a tab. Ctrl+K opens the command
//! palette, which searches every setting rather than only the page names.
//!
//! ## The command surface
//!
//! There is deliberately *one* mutating command, [`tz_run`], and it always
//! invokes the `tezca` binary — the front end chooses arguments, never a
//! program. Everything else here is a reader. That keeps the trust boundary
//! where the GTK build had it and means the echo footer has a single place to
//! hook, which is why every control can show its CLI equivalent for free.

mod arrange;
mod backend;
mod keybinds;
mod theme;

use serde::Serialize;
use tauri::{Emitter, Manager};

/// Page ids that no longer exist, and where their content went.
///
/// `desktop` was folded into Appearance ▸ Windows & motion — window gaps and
/// blur are things you reach for while choosing a theme, not a separate errand.
/// The old id stays routable because it is in shipped keybinds and in `tezca
/// settings --page desktop`, and silently opening nothing would be worse than
/// opening the page that now owns those controls.
const ALIASES: &[(&str, &str)] = &[("desktop", "appearance")];

const PAGES: &[&str] = &[
    "appearance",
    "bar",
    "dock",
    "displays",
    "sound",
    "input",
    "network",
    "power",
    "startup",
    "keybinds",
    "gaming",
    "system",
];

/// Resolve `--page <id>` from a command line, applying [`ALIASES`].
///
/// Returns `None` for an unknown id rather than guessing: opening Appearance
/// because you typed `--page dispalys` hides the typo.
fn page_from(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    let raw = loop {
        let a = it.next()?;
        if a == "--page" {
            break it.next()?.clone();
        }
        if let Some(v) = a.strip_prefix("--page=") {
            break v.to_string();
        }
    };
    let id = ALIASES.iter().find(|(from, _)| *from == raw).map(|(_, to)| *to).unwrap_or(&raw);
    PAGES.contains(&id).then(|| id.to_string())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The live palette, as CSS custom properties.
#[tauri::command]
fn tz_tokens() -> theme::Tokens {
    theme::tokens()
}

/// The page `--page` asked for, if any.
#[tauri::command]
fn tz_page() -> Option<String> {
    page_from(&std::env::args().collect::<Vec<_>>())
}

/// Run `tezca <args>`. The one mutating path, and the one that echoes.
#[tauri::command]
async fn tz_run(args: Vec<String>) -> backend::CmdResult {
    tauri::async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        backend::tezca_result(&refs)
    })
    .await
    .unwrap_or_else(|_| backend::CmdResult {
        code: -1,
        stdout: String::new(),
        stderr: "the background task panicked".into(),
    })
}

/// Run `tezca <args>` with `input` on stdin.
///
/// This is how a Wi-Fi password reaches NetworkManager: down a pipe. Passing it
/// as an argument would publish it in `/proc/<pid>/cmdline`, which every process
/// on the machine can read.
#[tauri::command]
async fn tz_run_stdin(args: Vec<String>, input: String) -> backend::CmdResult {
    tauri::async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        backend::tezca_stdin(&refs, &input)
    })
    .await
    .unwrap_or_else(|_| backend::CmdResult {
        code: -1,
        stdout: String::new(),
        stderr: "the background task panicked".into(),
    })
}

/// Announce a slow command before it starts, so the footer is not silent for
/// the seconds a Wi-Fi rescan takes.
#[tauri::command]
fn tz_announce(args: Vec<String>) {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    backend::echo_running(&backend::tezca_bin(), &refs);
}

/// Read `tezca <args>` without echoing — page loads, not user actions.
#[tauri::command]
async fn tz_read(args: Vec<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        backend::without_echo(|| backend::tezca_out(&refs))
    })
    .await
    .ok()
    .flatten()
}

/// Everything the shell needs before it can draw anything.
#[derive(Serialize)]
struct Boot {
    page: Option<String>,
    tokens: theme::Tokens,
    themes: Vec<backend::ThemeInfo>,
    session: (String, String),
    game_on: bool,
}

#[tauri::command]
async fn tz_boot() -> Boot {
    tauri::async_runtime::spawn_blocking(|| {
        backend::without_echo(|| Boot {
            page: page_from(&std::env::args().collect::<Vec<_>>()),
            tokens: theme::tokens(),
            themes: backend::themes(),
            session: backend::session_summary(),
            game_on: backend::game_on(),
        })
    })
    .await
    .unwrap_or_else(|_| Boot {
        page: None,
        tokens: theme::Tokens::default(),
        themes: Vec::new(),
        session: (String::new(), String::new()),
        game_on: false,
    })
}

/// Wallpaper state for the Appearance page.
#[derive(Serialize)]
struct Wallpapers {
    current: String,
    fit: String,
    library: Vec<String>,
    targets: Vec<(String, bool, String)>,
}

#[tauri::command]
async fn tz_wallpapers() -> Wallpapers {
    tauri::async_runtime::spawn_blocking(|| {
        backend::without_echo(|| Wallpapers {
            current: backend::current_wallpaper()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            fit: backend::wallpaper_fit(),
            library: backend::wallpaper_library()
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            targets: backend::wallpaper_targets(),
        })
    })
    .await
    .unwrap_or_else(|_| Wallpapers {
        current: String::new(),
        fit: "fill".into(),
        library: Vec::new(),
        targets: Vec::new(),
    })
}

/// The Displays page's whole world: monitors, their logical rectangles, and the
/// persisted overrides that cannot be read back off the compositor.
#[derive(Serialize)]
struct Displays {
    monitors: Vec<backend::Monitor>,
    rects: Vec<arrange::Rect>,
    config: Vec<(String, String)>,
}

/// A persisted per-monitor override, e.g. `("DP-1", "vrr")`.
///
/// Read through `backend::override_for` rather than re-parsed in JavaScript, so
/// the key format (`monitor:<NAME>.<field>`) has one definition.
#[tauri::command]
fn tz_override(config: Vec<(String, String)>, monitor: String, field: String) -> Option<String> {
    backend::override_for(&config, &monitor, &field)
}

#[tauri::command]
async fn tz_displays() -> Displays {
    tauri::async_runtime::spawn_blocking(|| {
        backend::without_echo(|| {
            let monitors = backend::monitors();
            Displays {
                rects: arrange::rects_of(&monitors),
                config: backend::display_config(),
                monitors,
            }
        })
    })
    .await
    .unwrap_or_else(|_| Displays {
        monitors: Vec::new(),
        rects: Vec::new(),
        config: Vec::new(),
    })
}

/// Pull a dragged monitor onto a neighbour's edge. Called per drag frame; the
/// arithmetic lives in Rust so there is one definition of "flush".
#[tauri::command]
fn tz_snap(rects: Vec<arrange::Rect>, i: usize, x: i32, y: i32, tol: i32) -> (i32, i32) {
    arrange::snap_at(&rects, i, x, y, tol)
}

/// The transform that fits the whole arrangement into the canvas, and the snap
/// tolerance that goes with it at that zoom.
#[tauri::command]
fn tz_fit(rects: Vec<arrange::Rect>, w: f64, h: f64) -> arrange::Fit {
    arrange::fit(&rects, w, h)
}

/// Pack the monitors left to right, vertically centred, origin at 0,0.
#[tauri::command]
fn tz_tidy(rects: Vec<arrange::Rect>) -> Vec<arrange::Rect> {
    let mut r = rects;
    arrange::tidy(&mut r);
    r
}

/// Which monitors moved, and whether the result overlaps.
#[tauri::command]
fn tz_layout_status(
    rects: Vec<arrange::Rect>,
    original: Vec<(i32, i32)>,
) -> (Vec<(String, i32, i32)>, bool) {
    (arrange::changed(&rects, &original), arrange::any_overlap(&rects))
}

/// Read a batch of Hyprland options in one go.
///
/// Batched because the Appearance and Input pages want a dozen each, and a
/// round trip per option turns opening a tab into a visible stall.
#[tauri::command]
async fn tz_hypr(opts: Vec<String>) -> Vec<(String, String)> {
    tauri::async_runtime::spawn_blocking(move || {
        backend::without_echo(|| {
            opts.into_iter()
                .filter_map(|o| backend::hypr_get(&o).map(|v| (o, v)))
                .collect()
        })
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn tz_bar_config() -> Vec<(String, String)> {
    tauri::async_runtime::spawn_blocking(|| backend::without_echo(backend::bar_config))
        .await
        .unwrap_or_default()
}

#[tauri::command]
async fn tz_dock_config() -> Vec<(String, String)> {
    tauri::async_runtime::spawn_blocking(|| backend::without_echo(backend::dock_config))
        .await
        .unwrap_or_default()
}

#[tauri::command]
async fn tz_custom_modules() -> Vec<(String, String)> {
    tauri::async_runtime::spawn_blocking(|| backend::without_echo(backend::custom_bar_modules))
        .await
        .unwrap_or_default()
}

/// The keybind table, as the CLI parses it out of keybinds.lua.
#[derive(Serialize)]
struct KeySection {
    title: String,
    binds: Vec<KeyBind>,
}

#[derive(Serialize)]
struct KeyBind {
    line: usize,
    combo: String,
    desc: String,
    action: String,
}

#[tauri::command]
async fn tz_keybinds() -> Vec<KeySection> {
    tauri::async_runtime::spawn_blocking(|| {
        backend::without_echo(|| {
            keybinds::load()
                .into_iter()
                .map(|s| KeySection {
                    title: s.title,
                    binds: s
                        .binds
                        .into_iter()
                        .map(|b| KeyBind {
                            line: b.line,
                            combo: b.combo(),
                            desc: b.desc,
                            action: b.action,
                        })
                        .collect(),
                })
                .collect()
        })
    })
    .await
    .unwrap_or_default()
}

/// DDC/CI brightness for one monitor, or `null` when the panel cannot be asked.
///
/// Slow enough to feel — `ddcutil` talks to the monitor over I²C — so the page
/// asks per monitor after it has drawn rather than blocking on the whole set.
#[tauri::command]
async fn tz_brightness(name: String) -> Option<i32> {
    tauri::async_runtime::spawn_blocking(move || {
        backend::without_echo(|| backend::brightness(&name))
    })
    .await
    .ok()
    .flatten()
}

/// Whether a binary is on PATH — the pages use it to hide controls for tools
/// that are not installed rather than offering a button that cannot work.
#[tauri::command]
fn tz_has(bin: String) -> bool {
    backend::has(&bin)
}

/// Front-end diagnostics onto this process's stderr.
///
/// A webview's console is invisible to whoever launched the binary, so a
/// scripting error looks exactly like the window failing to open. This puts it
/// where `tezca settings` was run from.
#[tauri::command]
fn tz_log(level: String, message: String) {
    eprintln!("tezca-settings [{level}] {message}");
}

/// Run one of the hypr/scripts helpers.
///
/// The name is a bare file name by contract; anything with a separator in it is
/// refused so a caller cannot walk out of the scripts directory.
#[tauri::command]
fn tz_script(name: String, args: Vec<String>) -> bool {
    if name.contains('/') || name.contains("..") || name.is_empty() {
        return false;
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    backend::run_script(&name, &refs);
    true
}

/// Work around webkit2gtk's DMABUF renderer on the NVIDIA proprietary driver.
///
/// Without this the webview never reaches the compositor: GTK dies with
/// `Error 71 (Protocol error) dispatching to Wayland display` before the window
/// is mapped, so `tezca settings` looks like it silently did nothing. It is a
/// known webkit2gtk/NVIDIA interaction, not something this app can fix on its
/// own side.
///
/// Applied only when the NVIDIA module is actually loaded, because the DMABUF
/// path is the faster one everywhere else, and never over a value the user set
/// deliberately.
fn nvidia_webkit_workaround() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(VAR).is_some() {
        return;
    }
    if std::path::Path::new("/sys/module/nvidia").exists() {
        std::env::set_var(VAR, "1");
    }
}

fn main() {
    nvidia_webkit_workaround();

    let mut builder = tauri::Builder::default();

    builder = builder.invoke_handler(tauri::generate_handler![
        tz_tokens,
        tz_page,
        tz_run,
        tz_run_stdin,
        tz_announce,
        tz_read,
        tz_boot,
        tz_wallpapers,
        tz_displays,
        tz_override,
        tz_snap,
        tz_fit,
        tz_tidy,
        tz_layout_status,
        tz_hypr,
        tz_bar_config,
        tz_dock_config,
        tz_custom_modules,
        tz_keybinds,
        tz_brightness,
        tz_has,
        tz_log,
        tz_script,
    ]);

    builder
        .setup(|app| {
            // Every mutating command reports here from now on; the footer is
            // listening before the first page has drawn.
            backend::set_echo_sink(app.handle().clone());
            if let Some(w) = app.get_webview_window("main") {
                // Shown by the front end once the palette is applied, so the
                // window never flashes unstyled — the theme arrives over IPC,
                // which is a round trip after the webview is technically ready.
                let _ = w.emit("tezca://ready", ());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tezca-settings failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn page_is_read_in_both_spellings_and_aliases_resolve() {
        assert_eq!(page_from(&args(&["tezca-settings", "--page", "bar"])).as_deref(), Some("bar"));
        assert_eq!(page_from(&args(&["tezca-settings", "--page=dock"])).as_deref(), Some("dock"));
        // `desktop` was folded into Appearance but stays routable, because it is
        // in shipped keybinds.
        assert_eq!(
            page_from(&args(&["tezca-settings", "--page", "desktop"])).as_deref(),
            Some("appearance")
        );
    }

    #[test]
    fn an_unknown_page_opens_nothing_rather_than_guessing() {
        // Opening Appearance because you typed `dispalys` hides the typo.
        assert_eq!(page_from(&args(&["tezca-settings", "--page", "dispalys"])), None);
        assert_eq!(page_from(&args(&["tezca-settings"])), None);
        assert_eq!(page_from(&args(&["tezca-settings", "--page"])), None);
    }
}
