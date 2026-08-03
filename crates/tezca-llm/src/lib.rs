//! Local AI — the Ollama client behind the `llm` bar module and the SUPER+I panel.
//!
//! # Why this one is not like the others
//!
//! `ai` and `weather` reach the internet, and both are opt-in for that reason.
//! This module talks to **`127.0.0.1` only**: Ollama is a daemon on your own
//! machine, the model weights are on your own disk, and nothing typed into the
//! panel leaves the loopback interface. That is the whole point of it, so the
//! posture here is the mirror image of the other two — the address is pinned to
//! loopback in [`endpoint`] and cannot be repointed at a remote host by config.
//! If you want a remote Ollama, that is a different feature with a different
//! privacy conversation, and this is deliberately not it.
//!
//! Everything else follows the house style: the daemon is optional (absent →
//! the module hides itself), all the slow work happens off the GTK thread, and
//! the UI only ever applies a finished value.

pub mod rate;

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// The loopback address the client is pinned to.
///
/// A `const` rather than a config key, on purpose: making this settable would
/// turn a module that provably cannot leak your prompts into one that might.
const HOST: &str = "127.0.0.1";

/// Which local server is answering.
///
/// The two speak different protocols — Ollama has its own JSON API and streams
/// newline-delimited objects; llama.cpp is OpenAI-shaped and streams SSE — so
/// this is not a cosmetic distinction and every request path branches on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Ollama,
    /// `llama serve` from llama.cpp (llama.app's unified binary).
    LlamaCpp,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Backend> {
        Some(match s.trim().to_lowercase().as_str() {
            "ollama" => Backend::Ollama,
            "llamacpp" | "llama.cpp" | "llama-cpp" | "llama" | "llamaserve" => Backend::LlamaCpp,
            _ => return None,
        })
    }

    /// The port each ships listening on.
    pub fn default_port(self) -> u16 {
        match self {
            Backend::Ollama => 11434,
            Backend::LlamaCpp => 8080,
        }
    }

    /// The path that answers cheaply when the server is up.
    fn health_path(self) -> &'static str {
        match self {
            Backend::Ollama => "/api/ps",
            Backend::LlamaCpp => "/health",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Ollama => "ollama",
            Backend::LlamaCpp => "llama.cpp",
        }
    }
}

/// Probed in this order when the backend is left on `auto`. llama.cpp first
/// only because its port is the less commonly occupied of the two; either way
/// the first one that answers wins and the other is never contacted.
const AUTODETECT: [Backend; 2] = [Backend::LlamaCpp, Backend::Ollama];

fn endpoint(port: u16, path: &str) -> String {
    format!("http://{HOST}:{port}{path}")
}

// ===========================================================================
// Config
// ===========================================================================

#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub enabled: bool,
    /// `None` = probe for whichever is running. Set it explicitly to skip the
    /// probe and to stop a second server on the other port from being picked up.
    pub backend: Option<Backend>,
    /// `0` = whatever the resolved backend's default is, so the common case
    /// needs no port line at all.
    pub port: u16,
    /// Seconds between status polls. Cheap — a loopback GET against a daemon
    /// that answers from memory.
    pub interval: u32,
    /// Model the panel opens with. Empty = whichever is already resident.
    pub model: String,
    /// Prepended to every conversation. Empty = none.
    pub system: String,
}

impl LlmConfig {
    /// `auto`, or the backend the user pinned — what the settings drawer shows.
    pub fn backend_name(&self) -> String {
        self.backend.map(|b| b.label().to_string()).unwrap_or_else(|| "auto".into())
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: None,
            port: 0,
            interval: 5,
            model: String::new(),
            system: String::new(),
        }
    }
}

// ===========================================================================
// Status
// ===========================================================================

/// A model Ollama currently holds in memory.
#[derive(Clone, Debug, Default)]
pub struct Resident {
    pub name: String,
    /// Parameter count as Ollama reports it, e.g. `70B`.
    pub params: String,
    pub quant: String,
    /// Total size, bytes.
    pub size: u64,
    /// How much of that is on the GPU, bytes. Meaningless unless
    /// `reports_offload`.
    pub size_vram: u64,
    pub context: u64,
    /// Whether the server actually publishes the GPU/CPU split. False for
    /// llama.cpp, which is why [`Self::accel`] returns an Option.
    pub reports_offload: bool,
}

impl Resident {
    /// Which device is actually doing the work, when the server says.
    ///
    /// Ollama reports the resident/VRAM split, so this is derived from it:
    /// nothing on the GPU means CPU, everything means GPU, and a partial
    /// offload is the case worth naming because it is the one that explains why
    /// generation suddenly got slow. llama.cpp publishes no equivalent, and
    /// `None` is the honest answer there — the bar shows nothing rather than
    /// asserting a device it cannot see.
    pub fn accel(&self) -> Option<&'static str> {
        if !self.reports_offload {
            return None;
        }
        Some(match (self.size_vram, self.size) {
            (0, _) => "CPU",
            (v, s) if v >= s => "GPU",
            _ => "split",
        })
    }

    /// Whether the accelerator badge should read as a warning.
    ///
    /// True for both states that leave work on the CPU: a fully CPU-resident
    /// model, and a partial offload. They are different problems — one is a
    /// model too big for the card, the other a card too full — but they have the
    /// same symptom, which is that generation is slower than the hardware
    /// implies, and the badge exists to say so. `None` (llama.cpp, which
    /// publishes no split) is not a warning; it is an absence of information.
    pub fn accel_degraded(&self) -> bool {
        matches!(self.accel(), Some("CPU" | "split"))
    }

    /// `18.9G`, or empty when the size is unknown.
    pub fn size_text(&self) -> String {
        if self.size == 0 {
            return String::new();
        }
        format!("{:.1}G", self.size as f64 / 1024.0 / 1024.0 / 1024.0)
    }

    /// `81% on GPU` — the offload fraction, or None when unknown.
    pub fn offload_pct(&self) -> Option<u32> {
        (self.reports_offload && self.size > 0)
            .then(|| (self.size_vram as f64 / self.size as f64 * 100.0).round() as u32)
    }
}

/// What the `llm` module knows right now.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// False when nothing answered on loopback — the module hides itself.
    pub up: bool,
    /// Which server answered, and on which port — resolved once per poll so
    /// every later request goes to the same place the probe found.
    pub backend: Option<Backend>,
    pub port: u16,
    pub resident: Vec<Resident>,
    /// Every model on disk, by name.
    pub available: Vec<String>,
}

impl Status {
    /// The model the bar module reports on: the largest resident one, which is
    /// the one occupying the memory you care about.
    pub fn primary(&self) -> Option<&Resident> {
        self.resident.iter().max_by_key(|r| r.size)
    }

    pub fn is_empty(&self) -> bool {
        !self.up
    }

    /// The module's tooltip: what is loaded, and where.
    pub fn tooltip(&self) -> String {
        let Some(b) = self.backend else {
            return "No local model server running".to_string();
        };
        if !self.up {
            return format!("{} is not running", b.label());
        }
        let Some(p) = self.primary() else {
            return format!("{} — no model loaded · {} available", b.label(), self.available.len());
        };
        let mut s = format!("{} — {}", b.label(), p.name);
        if !p.size_text().is_empty() {
            s.push_str(&format!(" · {}", p.size_text()));
        }
        // Only Ollama reports the GPU/CPU split. llama.cpp exposes no offload
        // figure over its API, so nothing is claimed for it rather than a
        // guess dressed up as a reading.
        //
        // Each state gets its own sentence rather than the bare word the badge
        // shows, because the badge's colour is the question the tooltip is
        // being opened to answer: two of these three render gold, and "CPU"
        // alone does not say why that matters.
        match (p.accel(), p.offload_pct()) {
            (Some("CPU"), _) => s.push_str(" · running on CPU — no GPU offload"),
            (Some("split"), Some(pct)) => {
                s.push_str(&format!(" · split offload — {pct}% on GPU"));
            }
            (Some("split"), None) => s.push_str(" · split offload"),
            (Some("GPU"), _) => s.push_str(" · fully resident on GPU"),
            _ => {}
        }
        if p.context > 0 {
            s.push_str(&format!(" · {} ctx", ctx_short(p.context)));
        }
        s
    }
}

/// `131072` → `128k` — a context window at a glance.
pub fn ctx_short(n: u64) -> String {
    if n >= 1024 {
        format!("{}k", n / 1024)
    } else {
        n.to_string()
    }
}

/// One status refresh.
///
/// Resolves the backend first (a single cheap GET per candidate when the config
/// says `auto`), then asks that server what it is holding.
pub fn poll_once(cfg: &LlmConfig) -> Status {
    let mut st = Status::default();
    let Some((backend, port)) = resolve(cfg) else { return st };
    st.up = true;
    st.backend = Some(backend);
    st.port = port;
    match backend {
        Backend::Ollama => fill_ollama(&mut st, port),
        Backend::LlamaCpp => fill_llamacpp(&mut st, port),
    }
    st
}

/// Which server is answering, and where.
///
/// An explicit `backend` skips the probe entirely — which matters when both are
/// installed and you want the bar pinned to one of them.
/// Which backend and port a request should go to, for callers outside this
/// crate — the AI panel resolves once and then streams to what it found.
pub fn resolve_public(cfg: &LlmConfig) -> Option<(Backend, u16)> {
    resolve(cfg)
}

fn resolve(cfg: &LlmConfig) -> Option<(Backend, u16)> {
    if let Some(b) = cfg.backend {
        let port = if cfg.port == 0 { b.default_port() } else { cfg.port };
        return get(&endpoint(port, b.health_path())).map(|_| (b, port));
    }
    for b in AUTODETECT {
        // A configured port applies to whichever candidate answers on it; with
        // no port set, each candidate is tried on its own default.
        let port = if cfg.port == 0 { b.default_port() } else { cfg.port };
        if get(&endpoint(port, b.health_path())).is_some() {
            return Some((b, port));
        }
    }
    None
}

fn fill_ollama(st: &mut Status, port: u16) {
    if let Some(body) = get(&endpoint(port, "/api/ps")) {
        if let Ok(v) = serde_json::from_str::<Value>(&body) {
            if let Some(models) = v.get("models").and_then(Value::as_array) {
                st.resident = models.iter().map(parse_resident).collect();
            }
        }
    }
    if let Some(body) = get(&endpoint(port, "/api/tags")) {
        if let Ok(v) = serde_json::from_str::<Value>(&body) {
            if let Some(models) = v.get("models").and_then(Value::as_array) {
                st.available = models
                    .iter()
                    .filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect();
            }
        }
    }
}

/// llama.cpp holds exactly one model per server, named in `/props`.
///
/// `/props` gives the alias, the on-disk path and the context window but no
/// size and no offload split, so the size is taken from the file itself — a
/// local `stat`, not a claim about what is in VRAM.
fn fill_llamacpp(st: &mut Status, port: u16) {
    if let Some(body) = get(&endpoint(port, "/props")) {
        if let Ok(v) = serde_json::from_str::<Value>(&body) {
            let name = v
                .get("model_alias")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    // No alias when started from a bare path: fall back to the
                    // filename, which is what the user typed anyway.
                    v.get("model_path")
                        .and_then(Value::as_str)
                        .and_then(|p| p.rsplit('/').next())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            let size = v
                .get("model_path")
                .and_then(Value::as_str)
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);
            let context = v
                .pointer("/default_generation_settings/n_ctx")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if !name.is_empty() {
                st.resident = vec![Resident {
                    name,
                    size,
                    context,
                    reports_offload: false,
                    ..Default::default()
                }];
            }
        }
    }
    // Note the key is `models`, not OpenAI's `data` — llama.cpp's list endpoint
    // is Ollama-shaped even though its completion endpoint is OpenAI-shaped.
    if let Some(body) = get(&endpoint(port, "/v1/models")) {
        if let Ok(v) = serde_json::from_str::<Value>(&body) {
            let arr = v
                .get("models")
                .or_else(|| v.get("data"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            st.available = arr
                .iter()
                .filter_map(|m| {
                    m.get("name")
                        .or_else(|| m.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect();
        }
    }
}

/// One entry of Ollama's `/api/ps`.
fn parse_resident(m: &Value) -> Resident {
    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let d = |k: &str| m.pointer(&format!("/details/{k}")).and_then(Value::as_str).unwrap_or("");
    Resident {
        name: s("name"),
        params: d("parameter_size").to_string(),
        quant: d("quantization_level").to_string(),
        size: m.get("size").and_then(Value::as_u64).unwrap_or(0),
        size_vram: m.get("size_vram").and_then(Value::as_u64).unwrap_or(0),
        context: m.get("context_length").and_then(Value::as_u64).unwrap_or(0),
        reports_offload: true,
    }
}

/// Spawn the status poll thread. A no-op unless enabled.
pub fn spawn(cfg: LlmConfig, tx: async_channel::Sender<Status>) {
    if !cfg.enabled {
        return;
    }
    std::thread::spawn(move || {
        let interval = cfg.interval.max(2) as u64;
        loop {
            if tx.send_blocking(poll_once(&cfg)).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(interval));
        }
    });
}

// ===========================================================================
// Chat
// ===========================================================================

/// One turn of a conversation.
#[derive(Clone, Debug)]
pub struct Message {
    /// `user` or `assistant`.
    pub role: String,
    pub content: String,
}

/// What a streaming completion emits.
pub enum Chunk {
    /// More answer text.
    Token(String),
    /// More of a reasoning model's scratchpad. Kept separate from [`Self::Token`]
    /// so the panel can label it rather than presenting it as the answer.
    Reasoning(String),
    /// The completion finished; carries its measured rate in tokens/second.
    Done {
        tps: f64,
        tokens: u64,
    },
    Error(String),
}

/// Stream a completion, sending chunks as they arrive.
///
/// Runs `curl -N` on its own thread and parses whatever the backend emits.
/// Not `--stream=false`: the whole point of the panel is that a large model's
/// first token arrives long before its last, and a spinner that sits still for
/// forty seconds is indistinguishable from a hang.
pub fn stream(
    cfg: &LlmConfig,
    backend: Backend,
    port: u16,
    model: String,
    history: Vec<Message>,
    tx: async_channel::Sender<Chunk>,
) {
    let mut messages: Vec<Value> = Vec::new();
    if !cfg.system.is_empty() {
        messages.push(serde_json::json!({"role": "system", "content": cfg.system}));
    }
    messages
        .extend(history.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})));

    let (url, body) = match backend {
        Backend::Ollama => (
            endpoint(port, "/api/chat"),
            serde_json::json!({ "model": model, "messages": messages, "stream": true }),
        ),
        Backend::LlamaCpp => (
            endpoint(port, "/v1/chat/completions"),
            // `timings_per_token` makes the server attach its own measured rate
            // to every chunk, so the panel reports llama.cpp's number rather
            // than timing the pipe and calling that throughput.
            serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": true,
                "timings_per_token": true
            }),
        ),
    };

    std::thread::spawn(move || {
        // The prompt goes on stdin, never argv: /proc/<pid>/cmdline is
        // world-readable, and what you type here is exactly the sort of thing
        // that should not be.
        let child = Command::new("curl")
            .args([
                "-sS",
                "-N",
                "--noproxy",
                "*",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
                &url,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send_blocking(Chunk::Error(e.to_string()));
                return;
            }
        };
        if let Some(si) = child.stdin.as_mut() {
            let _ = si.write_all(body.to_string().as_bytes());
        }
        drop(child.stdin.take());

        let Some(out) = child.stdout.take() else { return };
        let mut tokens = 0u64;
        let mut tps = 0.0;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Ollama streams bare JSON objects, one per line. llama.cpp streams
            // Server-Sent Events, so each payload is behind a `data: ` prefix
            // and the run ends with a literal `[DONE]` that is not JSON.
            let payload = match backend {
                Backend::Ollama => line,
                Backend::LlamaCpp => match line.strip_prefix("data:") {
                    Some(rest) => rest.trim(),
                    None => continue,
                },
            };
            if payload == "[DONE]" {
                break;
            }
            let Ok(v) = serde_json::from_str::<Value>(payload) else { continue };

            if let Some(err) = error_text(&v) {
                let _ = tx.send_blocking(Chunk::Error(err));
                return;
            }

            let (text, done) = match backend {
                Backend::Ollama => (
                    v.pointer("/message/content").and_then(Value::as_str).unwrap_or("").to_string(),
                    v.get("done").and_then(Value::as_bool) == Some(true),
                ),
                Backend::LlamaCpp => {
                    let d = v.pointer("/choices/0/delta");
                    // Reasoning models emit their scratchpad on a separate key.
                    // Dropping it silently would make the panel look frozen
                    // through the whole thinking phase, so it is streamed too —
                    // the panel labels it rather than passing it off as answer.
                    let content = d
                        .and_then(|d| d.get("content"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let reasoning = d
                        .and_then(|d| d.get("reasoning_content"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !reasoning.is_empty()
                        && tx.send_blocking(Chunk::Reasoning(reasoning.to_string())).is_err()
                    {
                        let _ = child.kill();
                        return;
                    }
                    let finished =
                        v.pointer("/choices/0/finish_reason").is_some_and(|f| !f.is_null());
                    (content, finished)
                }
            };

            // llama.cpp attaches timings to every chunk, so the rate is live
            // rather than only known at the end.
            if let Some(t) = v.get("timings") {
                if let Some(n) = t.get("predicted_n").and_then(Value::as_u64) {
                    tokens = n;
                }
                if let Some(r) = t.get("predicted_per_second").and_then(Value::as_f64) {
                    tps = r;
                }
            }

            if !text.is_empty() && tx.send_blocking(Chunk::Token(text)).is_err() {
                // The panel closed mid-generation. Stop the model too rather
                // than letting it finish into nowhere.
                let _ = child.kill();
                return;
            }

            if done {
                // Ollama only reports its counters on the final object.
                if backend == Backend::Ollama {
                    tokens = v.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
                    let ns = v.get("eval_duration").and_then(Value::as_u64).unwrap_or(0);
                    if ns > 0 {
                        tps = tokens as f64 / (ns as f64 / 1e9);
                    }
                }
                break;
            }
        }
        let _ = child.wait();
        let _ = tx.send_blocking(Chunk::Done { tps, tokens });
    });
}

/// An error reported inside a streamed object, in either backend's shape.
fn error_text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("error").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    v.pointer("/error/message").and_then(Value::as_str).map(str::to_string)
}

// ===========================================================================
// HTTP
// ===========================================================================

/// GET a loopback URL, or `None` when nothing is listening.
fn get(url: &str) -> Option<String> {
    // Belt and braces: `endpoint` is the only caller and always builds a
    // loopback URL, but this is the function that actually opens the socket,
    // so it is the one that has to be sure.
    if !url.starts_with(&format!("http://{HOST}:")) {
        return None;
    }
    // `--fail` is what makes this a *probe* rather than a liveness check on the
    // port. Without it curl succeeds on any response at all, so the health path
    // returning a 404 page counts as "the backend is up" — any HTTP server on
    // 8080 or 11434 gets reported as llama.cpp or Ollama, complete with an
    // accent status dot and a backend name. A static file server on the wrong
    // port is enough to do it.
    let out = Command::new("curl")
        .args(["-sS", "--fail", "--max-time", "4", "--noproxy", "*", url])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// `--llm-dump`: what the module can see, without opening a window.
pub fn dump(st: &Status) -> String {
    if !st.up {
        return "server   nothing answering on loopback\n".to_string();
    }
    let mut s = format!(
        "server   {} on 127.0.0.1:{}\n",
        st.backend.map(Backend::label).unwrap_or("?"),
        st.port
    );
    for r in &st.resident {
        s.push_str(&format!(
            "loaded   {} · {} · {} · {} · {}\n",
            r.name,
            if r.params.is_empty() { "?" } else { &r.params },
            if r.quant.is_empty() { "?" } else { &r.quant },
            if r.size_text().is_empty() { "?".into() } else { r.size_text() },
            r.accel().unwrap_or("offload not reported")
        ));
        if let Some(p) = r.offload_pct() {
            s.push_str(&format!("offload  {p}% on GPU\n"));
        }
    }
    if st.resident.is_empty() {
        s.push_str("loaded   nothing resident\n");
    }
    s.push_str(&format!("available {}\n", st.available.len()));
    for m in &st.available {
        s.push_str(&format!("  {m}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_is_loopback_and_stays_that_way() {
        let url = endpoint(11434, "/api/ps");
        assert_eq!(url, "http://127.0.0.1:11434/api/ps");
        // The guard in `get` is what actually protects the socket call, so it
        // has to refuse anything that is not the pinned host.
        assert_eq!(get("http://192.168.1.10:11434/api/ps"), None);
        assert_eq!(get("https://api.example.com/api/ps"), None);
    }

    #[test]
    fn accelerator_is_derived_from_the_offload_split() {
        let r = |vram| Resident {
            size: 1000,
            size_vram: vram,
            reports_offload: true,
            ..Default::default()
        };
        assert_eq!(r(0).accel(), Some("CPU"));
        assert_eq!(r(1000).accel(), Some("GPU"));
        assert_eq!(r(400).accel(), Some("split"));
        assert_eq!(r(400).offload_pct(), Some(40));
        // A model of unknown size has no meaningful fraction — not 0%.
        assert_eq!(Resident::default().offload_pct(), None);
        // llama.cpp publishes no split at all, so nothing may be asserted from
        // its numbers — not even "CPU", which a zero vram field would imply.
        let quiet =
            Resident { size: 1000, size_vram: 0, reports_offload: false, ..Default::default() };
        assert_eq!(quiet.accel(), None);
        assert_eq!(quiet.offload_pct(), None);
    }

    #[test]
    fn the_badge_warns_on_both_states_that_leave_work_on_the_cpu() {
        let r = |vram| Resident {
            size: 1000,
            size_vram: vram,
            reports_offload: true,
            ..Default::default()
        };
        // A model too big for the card and a card too full to take it are
        // different problems with the same symptom, so both go gold.
        assert!(r(0).accel_degraded());
        assert!(r(400).accel_degraded());
        assert!(!r(1000).accel_degraded());
        // No reading is not a warning.
        let quiet =
            Resident { size: 1000, size_vram: 0, reports_offload: false, ..Default::default() };
        assert!(!quiet.accel_degraded());
    }

    #[test]
    fn parses_a_ps_entry() {
        let v: Value = serde_json::from_str(
            r#"{"name":"llama3.1:70b","size":42000000000,"size_vram":42000000000,
                "context_length":131072,
                "details":{"parameter_size":"70.6B","quantization_level":"Q4_K_M"}}"#,
        )
        .unwrap();
        let r = parse_resident(&v);
        assert_eq!(r.name, "llama3.1:70b");
        assert_eq!(r.params, "70.6B");
        assert_eq!(r.quant, "Q4_K_M");
        assert_eq!(r.accel(), Some("GPU"));
        assert_eq!(r.size_text(), "39.1G");
        assert_eq!(r.context, 131072);
    }

    #[test]
    fn the_primary_model_is_the_largest_resident_one() {
        let st = Status {
            up: true,
            resident: vec![
                Resident { name: "embed".into(), size: 500, ..Default::default() },
                Resident { name: "big".into(), size: 42_000, ..Default::default() },
            ],
            ..Default::default()
        };
        assert_eq!(st.primary().map(|r| r.name.as_str()), Some("big"));
    }

    /// End-to-end against a real daemon, when there is one.
    ///
    /// Skips itself when nothing is listening on loopback, so it is a no-op on
    /// CI and on any machine without Ollama — but on a machine that has one it
    /// exercises the whole path the panel depends on: the POST, the
    /// newline-delimited stream, and the final `done` frame that carries the
    /// rate. That is the part no amount of parsing tests can stand in for.
    #[test]
    fn streams_a_completion_when_a_daemon_is_listening() {
        let cfg = LlmConfig { enabled: true, ..Default::default() };
        let st = poll_once(&cfg);
        if !st.up {
            return;
        }
        let Some(model) =
            st.primary().map(|r| r.name.clone()).or_else(|| st.available.first().cloned())
        else {
            return; // a daemon with no models has nothing to stream
        };

        let (backend, port) = (st.backend.expect("up implies a backend"), st.port);
        let (tx, rx) = async_channel::unbounded::<Chunk>();
        stream(
            &cfg,
            backend,
            port,
            model,
            vec![Message { role: "user".into(), content: "reply with the word ok".into() }],
            tx,
        );

        let mut text = String::new();
        let mut finished = false;
        // Bounded: a hung daemon must fail the test rather than hang the suite.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while std::time::Instant::now() < deadline {
            match rx.recv_blocking() {
                Ok(Chunk::Token(t)) => text.push_str(&t),
                // A reasoning model may think at length before answering; that
                // is progress, not silence, so it keeps the loop alive.
                Ok(Chunk::Reasoning(_)) => {}
                Ok(Chunk::Done { tps, .. }) => {
                    assert!(tps >= 0.0, "rate should be a number, got {tps}");
                    finished = true;
                    break;
                }
                Ok(Chunk::Error(e)) => panic!("stream reported an error: {e}"),
                Err(_) => break,
            }
        }
        assert!(finished, "the stream never sent a Done frame");
        assert!(!text.is_empty(), "the stream produced no tokens");
    }

    #[test]
    fn a_daemon_that_is_down_hides_the_module() {
        let st = Status::default();
        assert!(st.is_empty());
        // Nothing detected at all: the message must not name a server the user
        // may not even have installed.
        assert_eq!(st.tooltip(), "No local model server running");
        // Detected once, then gone — now it can be named.
        let down = Status { backend: Some(Backend::LlamaCpp), ..Default::default() };
        assert_eq!(down.tooltip(), "llama.cpp is not running");
        assert!(!Status { up: true, ..Default::default() }.is_empty());
    }

    /// A server that answers on the port but not on the health path is not a
    /// backend.
    ///
    /// Found by pointing the panel at a box with a static file server on 8080:
    /// its 404 page satisfied a bare `curl`, so the panel reported llama.cpp as
    /// running, with a live status dot and a backend name. Anything that speaks
    /// HTTP on either default port would have done it.
    #[test]
    fn a_port_that_answers_with_a_404_is_not_a_running_backend() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let body = "<html>404 not found</html>";
                let _ = write!(
                    s,
                    "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });

        let cfg = LlmConfig { backend: Some(Backend::LlamaCpp), port, ..Default::default() };
        assert_eq!(resolve(&cfg), None, "a 404 must not resolve as a live backend");
        assert!(!poll_once(&cfg).up);
    }
}
