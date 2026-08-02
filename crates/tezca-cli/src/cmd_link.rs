//! `tezca link` — put `config/*` in place under `~/.config`.
//!
//! Two strategies, because the entries are not all the same kind of thing:
//!
//!   * **symlink** (the default) for config that Tezca ships and owns. A symlink
//!     means `git pull` updates your desktop with no re-link step.
//!   * **seed** for directories the *user* and the tools write into
//!     (`tezca-bar/`, `tezca-dock/`: their `config.toml` and your dropped-in bar
//!     modules). These become real directories, with shipped files copied in only
//!     when absent. Symlinking them put user state inside the git checkout — every
//!     `tezca bar set` dirtied the tree, and a downstream clone could not pull
//!     without conflicting on it.
//!
//! Non-destructive and reversible: any pre-existing target that is not already the
//! correct symlink is renamed to `<name>.bak.<epoch>` before we link. `--force`
//! skips that only for symlinks, which hold no data of their own — anything with
//! contents is still backed up, because the installer promises the originals are
//! recoverable.

use crate::{atomic, cmd_keybind, cmd_startup, managed, repo, term};
use std::fs;
use std::os::unix::fs as unixfs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Entries under `config/` that hold user-writable state and must therefore be
/// real directories rather than symlinks into the repo.
///
/// `tezca-bar/` holds the bar's `config.toml` (rewritten by `tezca bar set`) and
/// `modules/`, where custom module manifests are dropped by hand. `tezca-dock/`
/// holds `dock.toml` (rewritten by `tezca dock set`).
const SEEDED: &[&str] = &["tezca-bar", "tezca-dock"];

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
    println!("  {} {}", term::dim("source:"), src_dir.display());
    println!("  {} {}", term::dim("target:"), cfg.display());
    if opts.dry_run {
        println!("  {}", term::yellow("dry-run — no changes will be made"));
    }
    println!();

    if !opts.dry_run {
        fs::create_dir_all(&cfg).map_err(|e| format!("cannot create {}: {e}", cfg.display()))?;
        // Drop a repo pointer so `tezca theme …` works from ANY cwd (the GUI and
        // keybinds run the installed ~/.local/bin/tezca, whose exe dir is not
        // under the repo, so the .tezca-root walk-up can't find it). link() runs
        // from inside the repo, so root is known here. See repo::root().
        let tezca_dir = cfg.join("tezca");
        if fs::create_dir_all(&tezca_dir).is_ok() {
            let _ = atomic::write(&tezca_dir.join("repo"), &format!("{}\n", root.display()));
        }
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(&src_dir)
        .map_err(|e| format!("cannot read {}: {e}", src_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).map(|n| !n.starts_with('.')).unwrap_or(false)
        })
        .collect();
    entries.sort();

    let mut linked = 0;
    let mut skipped = 0;
    let mut backed_up = 0;
    let mut seeded = 0;

    for src in entries {
        let name = src.file_name().unwrap().to_string_lossy().into_owned();
        let target = cfg.join(&name);
        if SEEDED.contains(&name.as_str()) {
            let (copied, kept) = seed_one(&src, &target, &opts)?;
            if copied > 0 {
                println!(
                    "  {} {name} {}",
                    term::green("+"),
                    term::dim(&format!("(seeded {copied} file(s), kept {kept} of yours)"))
                );
                seeded += 1;
            } else {
                println!(
                    "  {} {}",
                    term::green("✓"),
                    term::dim(&format!("{name} (yours, {kept} file(s) kept)"))
                );
                skipped += 1;
            }
            continue;
        }
        match link_one(&src, &target, &opts)? {
            Action::AlreadyLinked => {
                println!(
                    "  {} {}",
                    term::green("✓"),
                    term::dim(&format!("{name} (already linked)"))
                );
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

    // Generated files that hyprland.lua loads. Unlike hyprlang, which merely
    // logged a missing `source`, a Lua load error is not survivable — so these
    // must exist. An older install also has state to move out of the repo (and
    // off hyprlang) first, so migrate before seeding or the seed masks it.
    if !opts.dry_run {
        managed::ensure_migrated()?;
        if managed::seed()? {
            println!(
                "  {} {}",
                term::green("+"),
                term::dim("tezca/overrides.lua (machine overrides)")
            );
        }
        if cmd_keybind::seed()? {
            println!(
                "  {} {}",
                term::green("+"),
                term::dim("tezca/keybinds.lua (keybind overrides)")
            );
        }
        if cmd_startup::seed()? {
            println!(
                "  {} {}",
                term::green("+"),
                term::dim("tezca/startup.lua (your startup apps)")
            );
        }
    }

    println!();
    println!(
        "  {} {} linked · {} seeded · {} already ok · {} backed up",
        term::bold("done:"),
        linked,
        seeded,
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
        // `--force` is "don't litter the directory with backups". That is only safe
        // for a symlink, which holds no data — removing one cannot lose anything.
        // A real file or directory is backed up either way: `install.sh` and the
        // README both promise the originals are recoverable as `*.bak.*`, and
        // silently `remove_dir_all`-ing someone's existing ~/.config/hypr would
        // make that a lie.
        if opts.force && meta.file_type().is_symlink() {
            act(opts, &format!("remove existing symlink {}", target.display()), || {
                fs::remove_file(target).map_err(|e| e.to_string())
            })?;
        } else {
            let backup = backup_path(target);
            act(opts, &format!("back up {} → {}", target.display(), backup.display()), || {
                fs::rename(target, &backup).map_err(|e| e.to_string())
            })?;
            backed_up = true;
        }
    }

    act(opts, &format!("symlink {} → {}", target.display(), src.display()), || {
        unixfs::symlink(src, target).map_err(|e| e.to_string())
    })?;

    Ok(Action::Linked { backed_up })
}

/// Make `target` a real directory seeded from `src`, without ever overwriting a
/// file the user already has. Returns `(copied, kept)`.
fn seed_one(src: &Path, target: &Path, opts: &Opts) -> Result<(usize, usize), String> {
    // An install from before the split has this as a symlink into the repo. Remove
    // just the link — never the directory it points at — then seed from that same
    // directory, which carries whatever the user had already customised.
    let mut was_symlink = false;
    if let Ok(meta) = fs::symlink_metadata(target) {
        if meta.file_type().is_symlink() {
            was_symlink = true;
            act(
                opts,
                &format!("replace the symlink {} with a real directory", target.display()),
                || fs::remove_file(target).map_err(|e| e.to_string()),
            )?;
        }
    }
    if !opts.dry_run {
        fs::create_dir_all(target)
            .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
    }
    // A dry run leaves the symlink in place, so `exists()` on the destination would
    // resolve through it and every shipped file would look like one the user
    // already has — reporting "kept" where the real run copies. Treat the
    // destination as the empty directory it is about to become.
    let assume_empty = opts.dry_run && was_symlink;
    copy_missing(src, target, opts, assume_empty)
}

/// Recursively copy every file in `src` that is absent from `dst`. Existing files
/// are left exactly as they are — that is the whole point: shipped defaults appear
/// (including ones added by a later update), and your edits are never clobbered.
fn copy_missing(
    src: &Path,
    dst: &Path,
    opts: &Opts,
    assume_empty: bool,
) -> Result<(usize, usize), String> {
    let mut copied = 0;
    let mut kept = 0;
    let rd = match fs::read_dir(src) {
        Ok(rd) => rd,
        Err(e) => return Err(format!("cannot read {}: {e}", src.display())),
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let (s, d) = (entry.path(), dst.join(&name));
        if s.is_dir() {
            if !opts.dry_run {
                fs::create_dir_all(&d)
                    .map_err(|e| format!("cannot create {}: {e}", d.display()))?;
            }
            let (c, k) = copy_missing(&s, &d, opts, assume_empty)?;
            copied += c;
            kept += k;
        } else if !assume_empty && d.exists() {
            kept += 1;
        } else {
            act(opts, &format!("copy {} → {}", s.display(), d.display()), || {
                fs::copy(&s, &d).map(|_| ()).map_err(|e| e.to_string())
            })?;
            copied += 1;
        }
    }
    Ok((copied, kept))
}

/// Run `f` unless dry-run; in dry-run, just narrate the intended action.
fn act<F: FnOnce() -> Result<(), String>>(opts: &Opts, what: &str, f: F) -> Result<(), String> {
    if opts.dry_run {
        println!("    {} {}", term::yellow("would"), term::dim(what));
        Ok(())
    } else {
        f().map_err(|e| format!("{what}: {e}"))
    }
}

/// `<name>.bak.<epoch>`, uniquified.
///
/// The epoch is in whole seconds, so two backups of the same target inside one
/// second would otherwise collide and the second `rename` would silently destroy
/// the first.
fn backup_path(target: &Path) -> PathBuf {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = target.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut candidate = target.with_file_name(format!("{name}.bak.{secs}"));
    let mut n = 1;
    while candidate.exists() {
        candidate = target.with_file_name(format!("{name}.bak.{secs}-{n}"));
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tezca-link-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn opts() -> Opts {
        Opts { dry_run: false, force: false }
    }

    #[test]
    fn seeding_copies_shipped_files_and_never_overwrites_yours() {
        let d = scratch("seed");
        let (src, dst) = (d.join("src"), d.join("dst"));
        fs::create_dir_all(src.join("modules")).unwrap();
        fs::write(src.join("config.toml"), "shipped = 1\n").unwrap();
        fs::write(src.join("modules/example.toml"), "exec = echo\n").unwrap();

        // First run: nothing exists yet, so everything is copied.
        let (copied, kept) = seed_one(&src, &dst, &opts()).unwrap();
        assert_eq!((copied, kept), (2, 0));
        assert_eq!(fs::read_to_string(dst.join("config.toml")).unwrap(), "shipped = 1\n");

        // The user edits their copy, and upstream adds a new module.
        fs::write(dst.join("config.toml"), "mine = 2\n").unwrap();
        fs::write(src.join("modules/added-later.toml"), "exec = date\n").unwrap();

        let (copied, kept) = seed_one(&src, &dst, &opts()).unwrap();
        assert_eq!(copied, 1, "only the newly shipped file is copied");
        assert_eq!(kept, 2, "the user's file and the existing module are kept");
        assert_eq!(
            fs::read_to_string(dst.join("config.toml")).unwrap(),
            "mine = 2\n",
            "re-linking must never clobber an edited config"
        );
        assert!(dst.join("modules/added-later.toml").is_file());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn seeding_converts_a_legacy_symlink_into_a_real_directory_keeping_its_contents() {
        let d = scratch("convert");
        let (src, dst) = (d.join("src"), d.join("dst"));
        fs::create_dir_all(&src).unwrap();
        // The pre-split topology: the user's live settings sit in the repo, and
        // ~/.config/<name> is a symlink pointing at them.
        fs::write(src.join("config.toml"), "margin_top = 3\n").unwrap();
        unixfs::symlink(&src, &dst).unwrap();

        seed_one(&src, &dst, &opts()).unwrap();

        assert!(!fs::symlink_metadata(&dst).unwrap().file_type().is_symlink());
        assert!(dst.is_dir());
        assert_eq!(
            fs::read_to_string(dst.join("config.toml")).unwrap(),
            "margin_top = 3\n",
            "the settings that were living in the repo must survive the move"
        );
        // The repo copy is untouched: removing a symlink must not touch its target.
        assert_eq!(fs::read_to_string(src.join("config.toml")).unwrap(), "margin_top = 3\n");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn force_removes_a_symlink_but_still_backs_up_real_content() {
        let d = scratch("force");
        let (src, other) = (d.join("src"), d.join("other"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&other).unwrap();
        let forced = Opts { dry_run: false, force: true };

        // A foreign symlink: --force may remove it, since it holds no data.
        let link = d.join("as-link");
        unixfs::symlink(&other, &link).unwrap();
        link_one(&src, &link, &forced).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), src);

        // A real directory with the user's data: --force must NOT delete it.
        let real = d.join("as-dir");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("precious.conf"), "do not lose me\n").unwrap();
        link_one(&src, &real, &forced).unwrap();
        assert_eq!(fs::read_link(&real).unwrap(), src, "the link should now be in place");

        let backup = fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("as-dir.bak."))
            .expect("--force must still back up a real directory");
        assert_eq!(fs::read_to_string(backup.join("precious.conf")).unwrap(), "do not lose me\n");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_second_backup_in_the_same_second_does_not_overwrite_the_first() {
        let d = scratch("collide");
        let target = d.join("hypr");
        fs::write(&target, "x").unwrap();

        let first = backup_path(&target);
        fs::write(&first, "first").unwrap();
        let second = backup_path(&target);

        assert_ne!(first, second, "the epoch is only second-resolution, so it must uniquify");
        assert!(second.file_name().unwrap().to_string_lossy().contains("-1"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "first");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dry_run_reports_without_touching_anything() {
        let d = scratch("dry");
        let (src, dst) = (d.join("src"), d.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("config.toml"), "a = 1\n").unwrap();

        let dry = Opts { dry_run: true, force: false };
        let (copied, _) = seed_one(&src, &dst, &dry).unwrap();
        assert_eq!(copied, 1, "it should still report what it would copy");
        assert!(!dst.exists(), "dry-run must not create anything");

        // With a legacy symlink still in place, `exists()` on the destination
        // resolves through it — so a dry run must not mistake the shipped files for
        // files the user already has, or it reports 0 copies where the real run
        // makes 1.
        let linked = d.join("as-link");
        unixfs::symlink(&src, &linked).unwrap();
        let (copied, kept) = seed_one(&src, &linked, &dry).unwrap();
        assert_eq!((copied, kept), (1, 0), "a dry run must report the real work");
        assert!(fs::symlink_metadata(&linked).unwrap().file_type().is_symlink());
        fs::remove_dir_all(&d).unwrap();
    }
}
