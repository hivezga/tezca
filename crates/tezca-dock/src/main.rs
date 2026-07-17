//! tezca-dock — the bespoke magnifying macOS dock for Project:Tezca (DESIGN §6).
//!
//! A GTK4 + layer-shell dock: autohiding, obsidian glass, macOS magnification,
//! pinned favourites + running apps, wired to the Tezca theme engine. Control
//! signals: SIGUSR1 toggles pinned-open (SUPER+D), SIGUSR2 reloads the palette
//! after `tezca theme`.

mod apps;
mod config;
mod dock;
mod hypr;
mod magnifier;
mod theme;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::Application;
use signal_hook::consts::{SIGUSR1, SIGUSR2};

const APP_ID: &str = "dev.tezca.dock";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &Application) {
    let cfg = config::Config::load();
    let palette = theme::Palette::load();
    let dock = dock::Dock::build(app, cfg, palette);

    // Live updates from Hyprland's event socket → rebuild the model.
    let (tx, rx) = async_channel::unbounded::<()>();
    hypr::subscribe(tx);
    glib::spawn_future_local(glib::clone!(
        #[strong] dock,
        async move {
            while rx.recv().await.is_ok() {
                // Coalesce bursts (opening a window fires several events).
                while rx.try_recv().is_ok() {}
                dock.rebuild();
            }
        }
    ));

    // Control signals, delivered on a background thread → the GTK main loop.
    // (glib 0.22 doesn't expose a unix-signal source, so we use signal-hook.)
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
        #[strong] dock,
        async move {
            while let Ok(sig) = sig_rx.recv().await {
                match sig {
                    SIGUSR1 => dock.toggle_pin(),
                    SIGUSR2 => dock.reload_palette(),
                    _ => {}
                }
            }
        }
    ));
}
