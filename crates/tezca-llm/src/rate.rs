//! The generation-rate channel between the AI panel and the bar module.
//!
//! The design wants `tok/s` owned in one place so the bar module and the panel
//! footer read the same number — "panel-local state means the bar lies". They
//! are separate processes, so "one place" has to be a wire.
//!
//! The panel is the writer, because it is where generation happens; the bar
//! listens. The alternative was having the bar poll Ollama itself, which gives
//! two independent measurements of one thing and lets them disagree mid-burst.
//!
//! Deliberately a datagram socket, not a stream: the rate is a *current value*,
//! not a log. A reader that misses a packet wants the next one, not the one it
//! missed, and neither side should block or need a reconnect loop. The bar
//! creates the socket and the panel sends to it, so the panel starting first,
//! the bar restarting, or nothing listening at all are all non-events.

use std::path::PathBuf;

/// The socket's file name inside the session runtime directory.
const NAME: &str = "tezca-bar.rate";

/// Where the bar binds. `$XDG_RUNTIME_DIR` because this is per-session state
/// that must not survive a reboot; `/tmp` keyed by uid when that is unset.
pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let uid = unsafe { libc_getuid() };
            PathBuf::from(format!("/tmp/tezca-{uid}"))
        });
    socket_path_in(dir)
}

/// The address inside a given runtime directory.
///
/// Split out from [`socket_path`] so it can be tested and so the tests can use
/// a directory of their own: the environment is process-global, and two tests
/// that both set `XDG_RUNTIME_DIR` race each other.
pub fn socket_path_in(dir: impl Into<PathBuf>) -> PathBuf {
    dir.into().join(NAME)
}

// The one libc call this crate needs; declaring it beats a dependency for a
// fallback path that only runs when the session manager has not set
// XDG_RUNTIME_DIR.
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// One rate report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// Tokens per second. Zero means "not generating", which is the panel
    /// closing or finishing, and is what puts the bar module back to idle.
    pub tps: f64,
}

/// `tps 42.5` — one line, so a stray packet can never be half-read.
pub fn encode(r: Rate) -> String {
    format!("tps {:.1}\n", r.tps)
}

/// Parse a packet, ignoring anything that is not a rate report.
pub fn decode(s: &str) -> Option<Rate> {
    let rest = s.trim().strip_prefix("tps ")?;
    let tps: f64 = rest.trim().parse().ok()?;
    (tps.is_finite() && tps >= 0.0).then_some(Rate { tps })
}

/// Bind the socket and report every rate that arrives, on a background thread.
///
/// The bar calls this once at startup. Re-binding removes a stale socket first:
/// a Unix socket file outlives the process that made it, so a bar that was
/// killed rather than closed would otherwise lock its successor out of its own
/// address forever.
///
/// Returns false when the address could not be taken, which is not fatal — it
/// means the bar module shows its own polled state and no live rate, exactly as
/// it did before the panel existed.
pub fn listen(tx: async_channel::Sender<Rate>) -> bool {
    listen_at(socket_path(), tx)
}

/// [`listen`] at an explicit address.
pub fn listen_at(path: PathBuf, tx: async_channel::Sender<Rate>) -> bool {
    use std::os::unix::net::UnixDatagram;

    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path);
    let Ok(sock) = UnixDatagram::bind(&path) else { return false };

    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            let Ok(n) = sock.recv(&mut buf) else { continue };
            let Ok(text) = std::str::from_utf8(&buf[..n]) else { continue };
            let Some(r) = decode(text) else { continue };
            if tx.send_blocking(r).is_err() {
                return; // the bar is gone
            }
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_survives_the_round_trip() {
        assert_eq!(decode(&encode(Rate { tps: 42.5 })), Some(Rate { tps: 42.5 }));
        // Zero is a real value — it is how the panel says "stopped".
        assert_eq!(decode(&encode(Rate { tps: 0.0 })), Some(Rate { tps: 0.0 }));
    }

    #[test]
    fn anything_that_is_not_a_rate_is_ignored_rather_than_guessed_at() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("tps"), None);
        assert_eq!(decode("tps abc"), None);
        assert_eq!(decode("hello 12"), None);
        // A negative rate is not a slow one, it is a bug at the other end.
        assert_eq!(decode("tps -3"), None);
        assert_eq!(decode("tps inf"), None);
    }

    #[test]
    fn the_socket_lives_in_the_session_runtime_directory() {
        assert_eq!(
            socket_path_in("/run/user/1000"),
            PathBuf::from("/run/user/1000/tezca-bar.rate")
        );
    }

    /// The whole point of the channel: what the panel sends is what the bar
    /// shows. A second measurement taken by the bar itself could differ, which
    /// is the failure the design calls out — "panel-local state means the bar
    /// lies" is equally true of bar-local state.
    #[test]
    fn a_rate_sent_over_the_socket_arrives_unchanged() {
        use std::os::unix::net::UnixDatagram;
        let dir = std::env::temp_dir().join(format!("tezca-rate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let addr = socket_path_in(&dir);

        let (tx, rx) = async_channel::unbounded();
        assert!(listen_at(addr.clone(), tx), "the listener should take a free address");

        let sock = UnixDatagram::unbound().unwrap();
        sock.send_to(encode(Rate { tps: 37.5 }).as_bytes(), &addr).unwrap();
        assert_eq!(rx.recv_blocking().unwrap(), Rate { tps: 37.5 });

        // Garbage on the wire is dropped, not surfaced as a rate.
        sock.send_to(b"nonsense", &addr).unwrap();
        sock.send_to(encode(Rate { tps: 0.0 }).as_bytes(), &addr).unwrap();
        assert_eq!(rx.recv_blocking().unwrap(), Rate { tps: 0.0 });

        std::fs::remove_dir_all(&dir).ok();
    }
}
