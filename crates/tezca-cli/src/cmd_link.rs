//! `tezca link` — symlink `config/<name>` into `~/.config/<name>`.
//!
//! Non-destructive and reversible: any pre-existing target that is not already
//! the correct symlink is renamed to `<name>.bak.<epoch>` before we link.

use crate::{repo, term};
use std::fs;
use std::os::unix::fs as unixfs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Opts {
    pub dry_run: bool,
    pub force: bool,
}

pub fn run(opts: Opts) -> Result<(), String> {
    let root = repo::root()?;
    let src_dir = root.join("config");
    if !src_dir.is_dir() {
        return Err(format!("{} does not exist", src_dir.display()));
    }
    let cfg = repo::config_home()?;

    println!("{}", term::header("tezca link"));
    println!(
        "  {} {}",
        term::dim("source:"),
        src_dir.display()
    );
    println!("  {} {}", term::dim("target:"), cfg.display());
    if opts.dry_run {
        println!("  {}", term::yellow("dry-run — no changes will be made"));
    }
    println!();

    if !opts.dry_run {
        fs::create_dir_all(&cfg)
            .map_err(|e| format!("cannot create {}: {e}", cfg.display()))?;
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(&src_dir)
        .map_err(|e| format!("cannot read {}: {e}", src_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| !n.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();

    let mut linked = 0;
    let mut skipped = 0;
    let mut backed_up = 0;

    for src in entries {
        let name = src.file_name().unwrap().to_string_lossy().into_owned();
        let target = cfg.join(&name);
        match link_one(&src, &target, &opts)? {
            Action::AlreadyLinked => {
                println!("  {} {}", term::green("✓"), term::dim(&format!("{name} (already linked)")));
                skipped += 1;
            }
            Action::Linked { backed_up: bk } => {
                if bk {
                    backed_up += 1;
                }
                println!("  {} {}", term::green("→"), name);
                linked += 1;
            }
        }
    }

    println!();
    println!(
        "  {} {} linked · {} already ok · {} backed up",
        term::bold("done:"),
        linked,
        skipped,
        backed_up
    );
    Ok(())
}

enum Action {
    AlreadyLinked,
    Linked { backed_up: bool },
}

fn link_one(src: &Path, target: &Path, opts: &Opts) -> Result<Action, String> {
    // Already pointing at the right place?
    if let Ok(existing) = fs::read_link(target) {
        if existing == *src {
            return Ok(Action::AlreadyLinked);
        }
    }

    let mut backed_up = false;
    // symlink_metadata: does NOT follow the link, so a dangling/foreign symlink counts.
    if let Ok(meta) = fs::symlink_metadata(target) {
        let _ = meta;
        if opts.force {
            act(opts, &format!("remove existing {}", target.display()), || {
                remove_any(target)
            })?;
        } else {
            let backup = backup_path(target);
            act(
                opts,
                &format!("back up {} → {}", target.display(), backup.display()),
                || fs::rename(target, &backup).map_err(|e| e.to_string()),
            )?;
            backed_up = true;
        }
    }

    act(
        opts,
        &format!("symlink {} → {}", target.display(), src.display()),
        || unixfs::symlink(src, target).map_err(|e| e.to_string()),
    )?;

    Ok(Action::Linked { backed_up })
}

/// Run `f` unless dry-run; in dry-run, just narrate the intended action.
fn act<F: FnOnce() -> Result<(), String>>(
    opts: &Opts,
    what: &str,
    f: F,
) -> Result<(), String> {
    if opts.dry_run {
        println!("    {} {}", term::yellow("would"), term::dim(what));
        Ok(())
    } else {
        f().map_err(|e| format!("{what}: {e}"))
    }
}

fn remove_any(p: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(p).map_err(|e| e.to_string())?;
    if meta.file_type().is_dir() {
        fs::remove_dir_all(p).map_err(|e| e.to_string())
    } else {
        fs::remove_file(p).map_err(|e| e.to_string())
    }
}

fn backup_path(target: &Path) -> PathBuf {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = target.file_name().unwrap().to_string_lossy();
    target.with_file_name(format!("{name}.bak.{secs}"))
}
