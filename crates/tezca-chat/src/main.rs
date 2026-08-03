//! tezca-chat — the SUPER+I local-model chat panel.
//!
//! A real Hyprland window, not a layer surface. The GTK build reserved an
//! exclusive zone on the right edge, which shrinks the tiled area but does not
//! let the compositor lay windows out around the panel; a normal window joins
//! the layout, which is what the design asks for and what makes the slide-in
//! entry possible at all.
//!
//! It owns no model state of its own. Which backend is up, which models are
//! resident and how much VRAM each holds all come from `tezca-llm`, the same
//! crate the bar module polls, so the two surfaces cannot disagree. The rate it
//! measures while streaming is pushed back to the bar over
//! [`tezca_llm::rate`] — see that module for why it is a datagram.

mod config;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager};
use tezca_llm as llm;

/// Set while a completion is streaming. The stop button flips it; the reader
/// thread checks it between chunks and drops the connection.
static STREAMING: AtomicBool = AtomicBool::new(false);

/// Bumped on every send. A chunk carrying a stale id belongs to a turn that was
/// stopped or regenerated, and is discarded rather than appended to whatever
/// replaced it.
static TURN: AtomicU64 = AtomicU64::new(0);

static CONFIG: OnceLock<Mutex<llm::LlmConfig>> = OnceLock::new();

fn cfg() -> llm::LlmConfig {
    CONFIG.get_or_init(|| Mutex::new(config::load())).lock().map(|c| c.clone()).unwrap_or_default()
}

/// Work around webkit2gtk's DMABUF renderer on the NVIDIA proprietary driver:
/// without it GTK dies with `Error 71 (Protocol error)` before the window is
/// mapped. Same reasoning as `tezca-settings`; see its `main.rs`.
fn nvidia_webkit_workaround() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(VAR).is_none() && std::path::Path::new("/sys/module/nvidia").exists() {
        std::env::set_var(VAR, "1");
    }
}

// ---------------------------------------------------------------------------
// Rate reporting
// ---------------------------------------------------------------------------

/// Tell the bar what we are generating at. Fire and forget by design — nothing
/// listening is the normal case when the bar is not running.
fn report_rate(tps: f64) {
    use std::os::unix::net::UnixDatagram;
    let Ok(sock) = UnixDatagram::unbound() else { return };
    let _ = sock
        .send_to(llm::rate::encode(llm::rate::Rate { tps }).as_bytes(), llm::rate::socket_path());
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// What the panel needs before it can draw: the palette, the backend, and what
/// is loaded.
#[derive(Serialize)]
struct Boot {
    tokens: config::Tokens,
    status: Status,
    settings: Settings,
}

/// [`llm::Status`] flattened for the front end. The Rust type is not
/// `Serialize` and should not become so — it is the bar's working type, and a
/// wire shape that follows it would break the panel every time it changed.
#[derive(Serialize, Default)]
struct Status {
    up: bool,
    backend: String,
    /// Models with weights in memory right now, most relevant first.
    resident: Vec<Model>,
    /// Pulled but not loaded.
    available: Vec<Model>,
    tooltip: String,
}

#[derive(Serialize, Default)]
struct Model {
    name: String,
    short: String,
    /// `18.9G`, or empty when the backend does not say.
    size: String,
    /// `CUDA` · `CPU` · `split`, or empty when the backend publishes no split.
    accel: String,
    /// Whether that accelerator reading is a warning — CPU-only or a partial
    /// offload. Same rule as the bar module's gold badge.
    degraded: bool,
    /// Percent of the model's bytes that are in VRAM, when known.
    vram_pct: Option<u32>,
    /// `128k ctx`, or empty.
    ctx: String,
    quant: String,
}

fn model_of(r: &llm::Resident) -> Model {
    Model {
        short: r.name.split(':').next().unwrap_or(&r.name).to_string(),
        name: r.name.clone(),
        size: r.size_text(),
        accel: r.accel().unwrap_or("").to_string(),
        degraded: r.accel_degraded(),
        vram_pct: r.offload_pct(),
        ctx: if r.context > 0 {
            format!("{} ctx", llm::ctx_short(r.context))
        } else {
            String::new()
        },
        quant: r.quant.clone(),
    }
}

fn status_now() -> Status {
    let c = cfg();
    let st = llm::poll_once(&c);
    Status {
        up: st.up,
        backend: st.backend.map(|b| b.label().to_string()).unwrap_or_default(),
        resident: st.resident.iter().map(model_of).collect(),
        available: st
            .available
            .iter()
            .map(|name| Model {
                short: name.split(':').next().unwrap_or(name).to_string(),
                name: name.clone(),
                ..Default::default()
            })
            .collect(),
        tooltip: st.tooltip(),
    }
}

/// The tunables the settings drawer edits, persisted through `tezca bar set`
/// so the bar reads the same values on its next poll.
#[derive(Serialize, Default, Clone)]
struct Settings {
    system: String,
    model: String,
    port: u16,
    backend: String,
}

#[tauri::command]
async fn ai_boot() -> Boot {
    tauri::async_runtime::spawn_blocking(|| {
        let c = cfg();
        Boot {
            tokens: config::tokens(),
            status: status_now(),
            settings: Settings {
                system: c.system.clone(),
                model: c.model.clone(),
                port: c.port,
                backend: c.backend_name(),
            },
        }
    })
    .await
    .unwrap_or_else(|_| Boot {
        tokens: config::Tokens::default(),
        status: Status::default(),
        settings: Settings::default(),
    })
}

#[tauri::command]
async fn ai_status() -> Status {
    tauri::async_runtime::spawn_blocking(status_now).await.unwrap_or_default()
}

/// Persist one `llm_*` key through the CLI, so the bar picks it up too.
#[tauri::command]
async fn ai_set(key: String, value: String) -> bool {
    let ok = tauri::async_runtime::spawn_blocking(move || config::set(&key, &value))
        .await
        .unwrap_or(false);
    if ok {
        if let Some(slot) = CONFIG.get() {
            if let Ok(mut c) = slot.lock() {
                *c = config::load();
            }
        }
    }
    ok
}

/// One turn of the conversation, as the front end holds it.
#[derive(serde::Deserialize)]
struct Turn {
    role: String,
    content: String,
}

/// Start a completion. Chunks arrive as `ai://chunk` events rather than a
/// return value — the whole point is that the first token lands long before the
/// last one.
#[tauri::command]
fn ai_send(app: tauri::AppHandle, model: String, history: Vec<Turn>) -> u64 {
    let turn = TURN.fetch_add(1, Ordering::SeqCst) + 1;
    STREAMING.store(true, Ordering::SeqCst);

    let c = cfg();
    let Some((backend, port)) = llm::resolve_public(&c) else {
        STREAMING.store(false, Ordering::SeqCst);
        let _ = app.emit("ai://chunk", Chunk::error(turn, "no local model server is listening"));
        return turn;
    };

    let history: Vec<llm::Message> =
        history.into_iter().map(|t| llm::Message { role: t.role, content: t.content }).collect();
    let (tx, rx) = async_channel::unbounded::<llm::Chunk>();
    llm::stream(&c, backend, port, model, history, tx);

    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut chars = 0usize;
        while let Ok(chunk) = rx.recv_blocking() {
            // A stop or a regenerate has moved on; drop the tail of the old turn
            // rather than appending it to whatever replaced it.
            if TURN.load(Ordering::SeqCst) != turn || !STREAMING.load(Ordering::SeqCst) {
                break;
            }
            let out = match chunk {
                llm::Chunk::Token(t) => {
                    chars += t.chars().count();
                    // A live estimate, so the bar has something to show before
                    // the backend reports its own measured rate at the end.
                    let secs = started.elapsed().as_secs_f64().max(0.05);
                    report_rate((chars as f64 / 4.0) / secs);
                    Chunk { turn, kind: "token", text: t, ..Default::default() }
                }
                llm::Chunk::Reasoning(t) => {
                    Chunk { turn, kind: "reasoning", text: t, ..Default::default() }
                }
                llm::Chunk::Done { tps, tokens } => {
                    report_rate(0.0);
                    STREAMING.store(false, Ordering::SeqCst);
                    Chunk {
                        turn,
                        kind: "done",
                        tps,
                        tokens,
                        secs: started.elapsed().as_secs_f64(),
                        ..Default::default()
                    }
                }
                llm::Chunk::Error(e) => {
                    report_rate(0.0);
                    STREAMING.store(false, Ordering::SeqCst);
                    Chunk::error(turn, &e)
                }
            };
            let done = out.kind != "token" && out.kind != "reasoning";
            let _ = app.emit("ai://chunk", out);
            if done {
                return;
            }
        }
        // Fell out because the turn was abandoned: the panel already knows, but
        // the bar does not.
        report_rate(0.0);
    });
    turn
}

/// Stop the current completion, keeping whatever has arrived.
#[tauri::command]
fn ai_stop() {
    STREAMING.store(false, Ordering::SeqCst);
    TURN.fetch_add(1, Ordering::SeqCst);
    report_rate(0.0);
}

#[derive(Serialize, Default, Clone)]
struct Chunk {
    turn: u64,
    kind: &'static str,
    text: String,
    tps: f64,
    tokens: u64,
    secs: f64,
}

impl Chunk {
    fn error(turn: u64, msg: &str) -> Chunk {
        Chunk { turn, kind: "error", text: msg.to_string(), ..Default::default() }
    }
}

/// Front-end diagnostics onto stderr — a webview console is invisible to
/// whoever launched the binary.
#[tauri::command]
fn ai_log(level: String, message: String) {
    eprintln!("tezca-chat [{level}] {message}");
}

fn main() {
    nvidia_webkit_workaround();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ai_boot, ai_status, ai_set, ai_send, ai_stop, ai_log
        ])
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.emit("ai://ready", ());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tezca-chat failed to start");
}
