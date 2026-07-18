//! `tezca settings` — launch the tezca-settings GTK control center.
//!
//! The panel is a separate GTK4 binary (`tezca-settings`, crates/tezca-settings)
//! so its GTK deps never touch this std-only CLI — same split as `tezca dock`.
//! It is a single-instance GtkApplication: a second launch just raises the open
//! window. Any extra args (e.g. `--page keybinds`) pass straight through.

use crate::term;
use std::process::Command;

const BIN: &str = "tezca-settings";

pub fn run(args: &[&str]) -> i32 {
    match Command::new(BIN).args(args).spawn() {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{} could not launch {BIN}: {e}", term::red("error:"));
            eprintln!(
                "  {}",
                term::dim("build it with ./install.sh (or cargo build -p tezca-settings --release)")
            );
            1
        }
    }
}
