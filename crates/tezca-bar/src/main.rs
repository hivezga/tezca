//! tezca-bar — the bespoke gtk4-rs top menubar for Project:Tezca (DESIGN §6).
//!
//! A GTK4 + layer-shell bar replacing Waybar: obsidian glass, per-monitor
//! workspaces + per-app label, centred now-playing, and a right cluster of live
//! metrics/controls/indicators/clock/power — with expandable glass popovers and
//! the four-Tezcatlipoca theming. Wired to the Tezca theme engine: SIGUSR2
//! reloads the palette after `tezca theme`; SIGUSR1 toggles visibility.

mod ai;
mod bar;
mod bluetooth;
mod camera;
mod config;
mod custom;
mod draw;
mod hypr;
mod mic;
mod notify;
mod nowplaying;
mod osd;
mod popovers;
mod session;
mod sysinfo;
mod theme;
mod tray;

use gtk4::gdk::Display;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::Application;
use signal_hook::consts::{SIGUSR1, SIGUSR2};

const APP_ID: &str = "dev.tezca.bar";

/// Put the directory this binary lives in onto `PATH`.
///
/// The bar is launched by uwsm / systemd `--user` at login, whose inherited
/// PATH does NOT include `~/.local/bin` — where install.sh puts `tezca`,
/// `tezca-settings`, `claude`, etc. (the very reason autostart.conf spells the
/// bar's own path out in full). Without this, every shell-out to a sibling tool
/// silently dies with "command not found": the mirror menu's Settings item, the
/// AI module's `claude --version` probe, the "Sign in with claude" terminal.
/// Deriving the dir from the running exe keeps it correct wherever it's installed.
///
/// Called once at the top of `main`, before any thread is spawned, so the
/// `set_var` is race-free.
fn ensure_sibling_bins_on_path() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent().map(|p| p.to_path_buf()) else { return };
    let cur = std::env::var_os("PATH").unwrap_or_default();
    if std::env::split_paths(&cur).any(|p| p == dir) {
        return; // already reachable — don't shuffle the user's PATH
    }
    let mut paths = vec![dir];
    paths.extend(std::env::split_paths(&cur));
    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

fn main() -> glib::ExitCode {
    ensure_sibling_bins_on_path();

    // `--ai-dump`: poll the AI usage providers once, print what we can see, and
    // exit without opening a window. This is the supported way to debug the
    // module (or to check what it would send) without restarting a live bar.
    // It honours the same config, so `ai_live = false` keeps it fully offline.
    if std::env::args().any(|a| a == "--ai-dump") {
        let cfg = config::Config::load();
        print!("{}", ai::dump(&ai::poll_once(&cfg.ai)));
        return glib::ExitCode::SUCCESS;
    }

    // `--custom-dump`: discover the custom (community/user exec) modules, run
    // each once, and print what they emit — the way to verify a module's exec +
    // output parsing without opening a window or restarting the live bar.
    if std::env::args().any(|a| a == "--custom-dump") {
        print!("{}", custom::dump(&custom::load()));
        return glib::ExitCode::SUCCESS;
    }

    // `--camera-dump`: scan `/proc` once for who (if anyone) has the webcam open
    // and print it — the way to check the privacy indicator's detection without
    // opening a window. Prints "Camera idle" when nothing holds `/dev/video*`.
    if std::env::args().any(|a| a == "--camera-dump") {
        println!("{}", camera::poll().tooltip());
        return glib::ExitCode::SUCCESS;
    }

    // `--mic-dump`: ask PipeWire/PulseAudio who (if anyone) is recording and
    // print it — the windowless way to check the mic indicator. Prints
    // "Microphone idle" when nothing holds a live capture stream.
    if std::env::args().any(|a| a == "--mic-dump") {
        println!("{}", mic::poll().tooltip());
        return glib::ExitCode::SUCCESS;
    }

    // `--bt-dump`: print the Bluetooth module's view — adapter present/powered,
    // connected devices and their battery — without opening a window.
    if std::env::args().any(|a| a == "--bt-dump") {
        bluetooth::dump();
        return glib::ExitCode::SUCCESS;
    }

    // `--session-dump`: print the keep-awake / night-light / recording state the
    // bar would show, without opening a window.
    if std::env::args().any(|a| a == "--session-dump") {
        session::dump();
        return glib::ExitCode::SUCCESS;
    }

    // `--osd-demo <volume|brightness>`: flash one OSD pill on the primary monitor
    // for a couple of seconds, then quit. The way to eyeball the OSD's look (e.g.
    // the brightness variant on a machine with no backlight to trigger it) without
    // the whole bar. Uses its own app-id, so it can't disturb a running bar.
    if let Some(kind) = osd_demo_kind() {
        return run_osd_demo(kind);
    }

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.run()
}

/// Parse `--osd-demo <volume|brightness>` (defaulting to volume) from argv.
fn osd_demo_kind() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--osd-demo")?;
    Some(args.get(i + 1).cloned().unwrap_or_else(|| "volume".to_string()))
}

/// Show a single OSD pill on the primary monitor, then quit — a visual preview.
fn run_osd_demo(kind: String) -> glib::ExitCode {
    let app = Application::builder().application_id("dev.tezca.osd-demo").build();
    app.connect_activate(move |app| {
        let display = Display::default().expect("no display");
        // Keep the stack alive for the app's lifetime so the pill is themed.
        let css = theme::CssStack::install(&display);
        std::mem::forget(css);
        let monitors = display.monitors();
        let Some(obj) = monitors.item(0) else { return };
        let Ok(monitor) = obj.downcast::<gtk4::gdk::Monitor>() else { return };
        let osd = osd::Osd::build(app, &monitor, 1800);
        match kind.as_str() {
            "brightness" | "bright" => osd.show_brightness(72),
            _ => osd.show(45, false),
        }
        let app2 = app.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
            app2.quit();
        });
        let hold = app.hold();
        std::mem::forget(hold);
    });
    app.run_with_args(&["tezca-bar"])
}

fn activate(app: &Application) {
    let cfg = config::Config::load();
    let ai_cfg = cfg.ai.clone();
    let cfg_osd_enabled = cfg.osd_enabled;
    let palette = theme::Palette::load();
    let display = Display::default().expect("no display");
    let css = theme::CssStack::install(&display);

    // System-tray channels: updates come in from the D-Bus thread, click
    // commands go back out. Wired before the bar so it holds the command sender.
    let (tray_upd_tx, tray_upd_rx) = async_channel::unbounded::<tray::TrayUpdate>();
    let (tray_cmd_tx, tray_cmd_rx) = async_channel::unbounded::<tray::TrayCmd>();

    // Community/user exec modules, discovered from ~/.config/tezca-bar/modules.
    // Loaded before the bar so each `custom:<name>` slot has a widget to bind.
    let customs = custom::load();

    let bar = bar::Bar::build(app, cfg, palette, css, tray_cmd_tx, &customs);

    tray::spawn(tray_upd_tx, tray_cmd_rx);
    glib::spawn_future_local(glib::clone!(
        #[strong]
        bar,
        async move {
            while let Ok(update) = tray_upd_rx.recv().await {
                bar.apply_tray(update);
            }
        }
    ));

    // AI provider usage. No-ops unless `ai_enabled`; the poll thread owns the
    // slow work (network + log parsing) so the GTK loop only ever applies a
    // finished snapshot. See ai.rs for the privacy posture.
    let (ai_tx, ai_rx) = async_channel::unbounded::<ai::Snapshot>();
    ai::spawn(ai_cfg, ai_tx);
    glib::spawn_future_local(glib::clone!(
        #[strong]
        bar,
        async move {
            while let Ok(snap) = ai_rx.recv().await {
                bar.apply_ai(snap);
            }
        }
    ));

    // Custom exec modules: one poll thread per module owns the shell-out; the
    // GTK loop only applies a finished Output. No-op when none are installed.
    if !customs.is_empty() {
        let (custom_tx, custom_rx) = async_channel::unbounded::<custom::Output>();
        custom::spawn(customs, custom_tx);
        glib::spawn_future_local(glib::clone!(
            #[strong]
            bar,
            async move {
                while let Ok(out) = custom_rx.recv().await {
                    bar.apply_custom(out);
                }
            }
        ));
    }

    // Volume on-screen display. A background thread watches PipeWire/PulseAudio
    // for default-sink changes and pings the GTK loop, which flashes the glass
    // volume pill on every monitor. No-op if `osd_enabled = false` or `pactl` is
    // absent. See osd.rs.
    if cfg_osd_enabled {
        let (osd_tx, osd_rx) = async_channel::unbounded::<()>();
        osd::subscribe(osd_tx);
        glib::spawn_future_local(glib::clone!(
            #[strong]
            bar,
            async move {
                while let Ok(()) = osd_rx.recv().await {
                    // Coalesce a burst (dragging a slider fires many events) into
                    // a single re-read + flash.
                    while osd_rx.try_recv().is_ok() {}
                    bar.show_osd();
                }
            }
        ));
    }

    // Live Hyprland updates → refresh workspaces / app label / submap.
    let (tx, rx) = async_channel::unbounded::<hypr::Event>();
    hypr::subscribe(tx);
    glib::spawn_future_local(glib::clone!(
        #[strong]
        bar,
        async move {
            while let Ok(ev) = rx.recv().await {
                match ev {
                    hypr::Event::Refresh => {
                        // Coalesce bursts (opening a window fires several events).
                        while let Ok(hypr::Event::Refresh) = rx.try_recv() {}
                        bar.refresh_hypr();
                    }
                    hypr::Event::Submap(name) => bar.set_submap(&name),
                }
            }
        }
    ));

    // Control signals, delivered on a background thread → the GTK main loop.
    // (glib 0.22 has no unix-signal source, so we use signal-hook, like the dock.)
    let (sig_tx, sig_rx) = async_channel::unbounded::<i32>();
    if let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGUSR1, SIGUSR2]) {
        std::thread::spawn(move || {
            for sig in signals.forever() {
                if sig_tx.send_blocking(sig).is_err() {
                    break;
                }
            }
        });
    }
    glib::spawn_future_local(glib::clone!(
        #[strong]
        bar,
        async move {
            while let Ok(sig) = sig_rx.recv().await {
                match sig {
                    SIGUSR1 => bar.toggle_visibility(),
                    SIGUSR2 => bar.reload_palette(),
                    _ => {}
                }
            }
        }
    ));

    // Keep the app alive even if a bar window is hidden (SIGUSR1 toggle).
    let hold = app.hold();
    std::mem::forget(hold);
}
