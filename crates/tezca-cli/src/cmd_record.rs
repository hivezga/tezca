//! `tezca record` — screen recording, the missing half of the screenshot flow.
//!
//!   start [--region] [--output NAME] [--audio] [--no-cursor]
//!   stop
//!   toggle [flags…]
//!   status [--machine]
//!
//! Recordings land in `~/Videos/Tezca/<timestamp>.mp4`. The running recorder's
//! PID is tracked in `~/.cache/tezca/record.pid` so `stop` kills exactly the
//! process we started — never `pkill wf-recorder`, which would also kill a
//! recording somebody else's tooling started.
//!
//! **Stop must be a clean SIGINT.** wf-recorder finalises the container on
//! interrupt; SIGKILL leaves an unplayable file with no moov atom. That is the
//! single most important line in this module.
//!
//! On NVIDIA, software x264 cannot keep up with 3440x1440 at 165 Hz, so hardware
//! encoding is selected when the encoder is present.

use crate::{repo, term, util};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn run(args: &[&str]) -> i32 {
    let r = match args.first().copied() {
        None | Some("status") => cmd_status(args.get(1..).unwrap_or(&[])),
        Some("start") => cmd_start(&args[1..]),
        Some("stop") => cmd_stop(),
        Some("toggle") => cmd_toggle(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown record subcommand: {other}\n  try: start · stop · toggle · status"
        )),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{} {e}", term::red("error:"));
            1
        }
    }
}

fn pid_path() -> Result<PathBuf, String> {
    Ok(repo::cache_home()?.join("tezca").join("record.pid"))
}

fn file_path() -> Result<PathBuf, String> {
    Ok(repo::cache_home()?.join("tezca").join("record.path"))
}

/// The live recorder's PID, if one is ours and still running.
pub fn active_pid() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_path().ok()?).ok()?.trim().parse().ok()?;
    std::path::Path::new(&format!("/proc/{pid}")).exists().then_some(pid)
}

/// The recorder binary to use, preferring the one that can keep up.
fn recorder() -> Option<&'static str> {
    ["wl-screenrec", "wf-recorder"].into_iter().find(|b| util::has(b))
}

fn cmd_status(args: &[&str]) -> Result<(), String> {
    let pid = active_pid();
    let path = std::fs::read_to_string(file_path()?).unwrap_or_default().trim().to_string();
    if args.iter().any(|a| *a == "--machine" || *a == "-m") {
        println!("recording={}", pid.is_some());
        println!("pid={}", pid.map(|p| p.to_string()).unwrap_or_default());
        println!("path={}", if pid.is_some() { path } else { String::new() });
        println!("recorder={}", recorder().unwrap_or(""));
        return Ok(());
    }
    println!("{}", term::header("tezca record"));
    println!();
    match pid {
        Some(p) => println!("  {} recording  {}", term::green("●"), term::dim(&format!("{path} (pid {p})"))),
        None => println!("  {} not recording", term::dim("○")),
    }
    if recorder().is_none() {
        println!("  {} {}", term::yellow("!"), term::dim("no recorder installed (`paru -S wf-recorder`)"));
    }
    println!();
    Ok(())
}

fn cmd_start(args: &[&str]) -> Result<(), String> {
    if let Some(pid) = active_pid() {
        return Err(format!("already recording (pid {pid}) — `tezca record stop` first"));
    }
    let bin = recorder()
        .ok_or("no screen recorder found — install one (`paru -S wf-recorder`)")?;

    let region = args.contains(&"--region");
    let audio = args.contains(&"--audio");
    let no_cursor = args.contains(&"--no-cursor");
    let output = args
        .iter()
        .position(|a| *a == "--output")
        .and_then(|i| args.get(i + 1))
        .copied();

    // ~/Videos/Tezca/2026-08-01_14-32-05.mp4
    let stamp = Command::new("date")
        .args(["+%Y-%m-%d_%H-%M-%S"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or("could not read the current time")?;
    let home = std::env::var_os("HOME").ok_or("$HOME is not set")?;
    let dir = PathBuf::from(home).join("Videos").join("Tezca");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let out = dir.join(format!("{stamp}.mp4"));

    let mut cmd = Command::new(bin);
    cmd.arg("-f").arg(&out);
    if no_cursor {
        // Both recorders spell this the same way.
        cmd.arg("--no-cursor");
    }
    if let Some(name) = output {
        cmd.arg("-o").arg(name);
    }
    if region {
        let geom = Command::new("slurp")
            .output()
            .map_err(|e| format!("slurp is needed to select a region: {e}"))?;
        let geom = String::from_utf8_lossy(&geom.stdout).trim().to_string();
        if geom.is_empty() {
            return Err("region selection cancelled".into());
        }
        cmd.arg("-g").arg(geom);
    }
    if audio {
        cmd.arg("--audio");
    }
    // Software x264 cannot keep up with an ultrawide at 165 Hz; use the GPU when
    // the encoder is there. wl-screenrec picks hardware encoding by itself.
    if bin == "wf-recorder" && has_nvenc() {
        cmd.args(["-c", "h264_nvenc"]);
    }

    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start {bin}: {e}"))?;

    let pid_file = pid_path()?;
    if let Some(parent) = pid_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&pid_file, format!("{}\n", child.id()))
        .map_err(|e| format!("could not record the pid: {e}"))?;
    std::fs::write(file_path()?, format!("{}\n", out.display()))
        .map_err(|e| format!("could not record the output path: {e}"))?;

    println!("  {} recording → {}", term::green("●"), term::bold(&out.display().to_string()));
    Ok(())
}

fn has_nvenc() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("h264_nvenc"))
        .unwrap_or(false)
}

fn cmd_stop() -> Result<(), String> {
    let Some(pid) = active_pid() else {
        return Err("not recording".into());
    };
    // SIGINT, not SIGKILL: the recorder finalises the container on interrupt.
    // Killing it outright leaves a file with no moov atom — unplayable, and the
    // recording is gone.
    let _ = Command::new("kill").args(["-INT", &pid.to_string()]).status();
    for _ in 0..100 {
        if active_pid().is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let path = std::fs::read_to_string(file_path()?).unwrap_or_default().trim().to_string();
    let _ = std::fs::remove_file(pid_path()?);

    if util::has("notify-send") && !path.is_empty() {
        let _ = Command::new("notify-send")
            .args(["-a", "Tezca", "Recording saved", &path])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    println!("  {} saved {}", term::green("✓"), term::bold(&path));
    Ok(())
}

fn cmd_toggle(args: &[&str]) -> Result<(), String> {
    if active_pid().is_some() {
        cmd_stop()
    } else {
        cmd_start(args)
    }
}

fn print_help() {
    println!("{}", term::header("tezca record"));
    println!();
    for (c, d) in [
        ("start", "record every output to ~/Videos/Tezca"),
        ("start --region", "select a region with slurp first"),
        ("start --output DP-1", "one monitor only"),
        ("start --audio", "include audio"),
        ("stop", "finish and save (SIGINT, so the file is playable)"),
        ("toggle", "start, or stop if already recording"),
        ("status", "what is being recorded, and where"),
    ] {
        println!("  {:<22} {}", term::cyan(c), term::dim(d));
    }
}
