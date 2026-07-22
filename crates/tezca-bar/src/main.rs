//! tezca-bar — the bespoke gtk4-rs top menubar for Project:Tezca (DESIGN §6).
//!
//! A GTK4 + layer-shell bar replacing Waybar: obsidian glass, per-monitor
//! workspaces + per-app label, centred now-playing, and a right cluster of live
//! metrics/controls/indicators/clock/power — with expandable glass popovers and
//! the four-Tezcatlipoca theming. Wired to the Tezca theme engine: SIGUSR2
//! reloads the palette after `tezca theme`; SIGUSR1 toggles visibility.

mod bar;
mod config;
mod draw;
mod hypr;
mod nowplaying;
mod notify;
mod popovers;
mod sysinfo;
mod theme;
mod tray;

use gtk4::gdk::Display;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::Application;
use signal_hook::consts::{SIGUSR1, SIGUSR2};

const APP_ID: &str = "dev.tezca.bar";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &Application) {
    let cfg = config::Config::load();
    let palette = theme::Palette::load();
    let display = Display::default().expect("no display");
    let css = theme::CssStack::install(&display);

    // System-tray channels: updates come in from the D-Bus thread, click
    // commands go back out. Wired before the bar so it holds the command sender.
    let (tray_upd_tx, tray_upd_rx) = async_channel::unbounded::<tray::TrayUpdate>();
    let (tray_cmd_tx, tray_cmd_rx) = async_channel::unbounded::<tray::TrayCmd>();

    let bar = bar::Bar::build(app, cfg, palette, css, tray_cmd_tx);

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
