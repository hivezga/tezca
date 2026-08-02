//! AI provider usage — the bar's `[󰚩 41%]` module (DESIGN.md §6).
//!
//! Shows how much of your AI subscription's rate-limit window you've burned,
//! per provider, with a glass popover breaking it down. Runs on one background
//! thread that feeds the same `async-channel`→glib bridge as `hypr::subscribe`
//! and `tray::spawn`, so the GTK main loop never blocks on a network call.
//!
//! ## Privacy & safety posture
//!
//! This is the only module that talks to the internet, so the rules are strict
//! and enforced in code, not just documented:
//!
//!   * **Opt-in.** Nothing runs unless `ai_enabled` is set *and* the provider is
//!     listed in `ai_providers`. A provider whose credentials are absent goes
//!     [`Status::Absent`] and its row is hidden — never an error popup.
//!   * **We never store a credential.** Tezca has no keyring entry, no config
//!     field, and no cache of its own. We *read* the credential file the
//!     provider's own CLI already wrote (mode 0600, owned by the user) and use
//!     it in memory for one request.
//!   * **Never on the command line.** A header on `argv` is world-readable via
//!     `/proc/<pid>/cmdline` — any local process could scrape the token. So the
//!     token is handed to curl through a config file on **stdin** (`curl -K -`),
//!     which never touches argv or the filesystem. See [`curl_get`].
//!   * **Hardcoded host allowlist.** [`ALLOWED_HOSTS`] is checked before every
//!     request, and curl is pinned to `--proto =https` with redirects off, so a
//!     compromised response can't redirect us to an exfiltration endpoint.
//!   * **Redacted errors.** [`redact`] scrubs anything token-shaped out of error
//!     text before it can reach a tooltip, a popover, or stderr.
//!   * **No telemetry.** The only bytes that leave this machine are the request
//!     to the provider's own documented host. Nothing is reported to Tezca.
//!   * **Offline mode.** `ai_live = false` disables the network path entirely;
//!     the module then reports only what it computes from local logs.
//!
//! ## Data sources
//!
//! | Provider  | Live (network)                          | Local (offline)                     |
//! |-----------|-----------------------------------------|-------------------------------------|
//! | anthropic | `api.anthropic.com/api/oauth/usage`      | `~/.claude/projects/**/*.jsonl`     |
//! | openai    | `codex app-server` JSON-RPC (localhost)  | —                                   |
//! | google    | —                                        | `~/.gemini/tmp/**/chats/*.json`     |
//!
//! The Anthropic endpoint is **undocumented and beta-versioned** — it is what
//! Claude Code itself calls. It can change or vanish without notice, so every
//! field is parsed defensively and a parse miss degrades to [`Status::Error`]
//! rather than panicking. The OpenAI and Google adapters are written against
//! published third-party findings but are **unverified on this machine** (no
//! `~/.codex`, no Gemini CLI); they auto-hide until those tools are installed.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

// ===========================================================================
// Safety rails
// ===========================================================================

/// Every host this module may contact, ever. Checked before each request; a URL
/// whose host is not in this list is refused rather than fetched. Keeping it a
/// `const` means the allowlist is auditable in one place and cannot be widened
/// by config.
const ALLOWED_HOSTS: &[&str] = &["api.anthropic.com"];

/// True only when `url`'s origin is exactly one of [`ALLOWED_HOSTS`] over https.
fn allowlisted(url: &str) -> bool {
    ALLOWED_HOSTS.iter().any(|h| {
        let origin = format!("https://{h}");
        // The origin alone, or the origin followed by a path separator — never
        // `https://api.anthropic.com.evil.test/`.
        url == origin || url.starts_with(&format!("{origin}/"))
    })
}

/// Hard ceiling on how long a single request may take, seconds. The bar must
/// never appear to hang because a provider is slow.
const HTTP_TIMEOUT: u32 = 10;

/// Never poll faster than this (seconds), whatever the config says. The
/// Anthropic usage endpoint is aggressively rate-limited and a tight loop earns
/// a persistent 429 for the whole machine — including Claude Code itself.
const MIN_INTERVAL: u32 = 60;

/// Longest backoff after repeated 429s, seconds.
const MAX_BACKOFF: u32 = 1800;

/// Scrub anything token-shaped out of text before it can be displayed or
/// logged. Errors from curl/serde can quote the input that failed, and the
/// input contains a bearer token — so this runs on every error string.
///
/// The rule is deliberately blunt: any run of 20+ token-ish characters becomes
/// `<redacted>`. Over-redacting an error message is harmless; under-redacting
/// leaks a credential into a tooltip.
fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let tokenish = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.';
    for c in s.chars() {
        if tokenish(c) {
            run.push(c);
        } else {
            flush_run(&mut out, &mut run);
            out.push(c);
        }
    }
    flush_run(&mut out, &mut run);
    // Also drop any explicit header echo, belt and braces.
    out.replace("Bearer", "").trim().to_string()
}

fn flush_run(out: &mut String, run: &mut String) {
    if run.len() >= 20 {
        out.push_str("<redacted>");
    } else {
        out.push_str(run);
    }
    run.clear();
}

// ===========================================================================
// Rate-limit hold, shared across processes
// ===========================================================================

/// One provider's throttling state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Hold {
    /// Unix seconds before which we will not make a live request.
    until: i64,
    /// Current backoff length in seconds, doubled on each further 429.
    backoff: u32,
    /// When we last actually sent a request.
    last_attempt: i64,
}

/// Throttling state for every provider, persisted to disk.
///
/// On disk because the state has to be shared between processes, not just across
/// iterations of the poll loop. The resident bar and a one-shot
/// `tezca-bar --ai-dump` hit the *same* endpoint, and for Anthropic that endpoint
/// shares its 429 bucket with Claude Code itself — so a 429 earned by one must hold
/// the other back too. In-memory state could not do that, which is why `--ai-dump`
/// used to be able to poll straight through a live backoff.
#[derive(Debug, Default)]
struct Holds {
    map: HashMap<String, Hold>,
}

impl Holds {
    /// `~/.cache/tezca/ai-hold`. Cache, not config: it is derived state that is
    /// safe to lose (losing it only means we retry sooner than we would have).
    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| home().map(|h| h.join(".cache")))?;
        Some(base.join("tezca").join("ai-hold"))
    }

    /// `provider \t until \t backoff \t last_attempt` per line. A malformed or
    /// missing file simply means "no holds".
    fn load() -> Self {
        let mut map = HashMap::new();
        if let Some(text) = Self::path().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                let mut f = line.split('\t');
                let (Some(name), Some(until), Some(backoff), Some(last)) =
                    (f.next(), f.next(), f.next(), f.next())
                else {
                    continue;
                };
                let (Ok(until), Ok(backoff), Ok(last)) =
                    (until.parse(), backoff.parse(), last.parse())
                else {
                    continue;
                };
                map.insert(name.to_string(), Hold { until, backoff, last_attempt: last });
            }
        }
        Holds { map }
    }

    fn save(&self) {
        let Some(p) = Self::path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let body: String = self
            .map
            .iter()
            .map(|(k, h)| format!("{k}\t{}\t{}\t{}\n", h.until, h.backoff, h.last_attempt))
            .collect();
        let _ = std::fs::write(&p, body);
    }

    fn get(&self, provider: &str) -> Hold {
        self.map.get(provider).copied().unwrap_or_default()
    }

    /// Note that we are about to send a request.
    fn record_attempt(&mut self, provider: &str, now: i64) {
        self.map.entry(provider.to_string()).or_default().last_attempt = now;
    }

    /// Register a 429 and return the unix time we will retry.
    ///
    /// Doubling, deliberately, rather than reading `Retry-After`: it never
    /// under-waits and cannot be gamed by a bad header.
    fn back_off(&mut self, provider: &str, now: i64, floor: u32) -> i64 {
        let h = self.map.entry(provider.to_string()).or_default();
        h.backoff = if h.backoff == 0 { floor } else { (h.backoff * 2).min(MAX_BACKOFF) };
        h.until = now + h.backoff as i64;
        h.until
    }

    /// A successful request clears the backoff but keeps `last_attempt`, which is
    /// what enforces the minimum interval.
    fn succeeded(&mut self, provider: &str) {
        let h = self.map.entry(provider.to_string()).or_default();
        h.until = 0;
        h.backoff = 0;
    }
}

// ===========================================================================
// Public model
// ===========================================================================

/// Why a provider row looks the way it does.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Live data in hand.
    Ok,
    /// Provider not listed in `ai_providers` — we never looked.
    Disabled,
    /// Listed, but its CLI/credentials aren't on this machine. Row hides.
    Absent,
    /// `ai_live = false`; only local numbers are available.
    LocalOnly,
    /// Credentials exist but the stored OAuth session is dead — expired locally,
    /// or rejected by the endpoint. Refreshing it is the provider CLI's job, so
    /// the popover offers a button that launches it rather than an error.
    NeedsLogin,
    /// Provider said "slow down". Carries the unix time we'll retry.
    RateLimited { until: i64 },
    /// *We* are holding off, not the provider: the last live request was less than
    /// [`MIN_INTERVAL`] ago. Distinct from [`Status::RateLimited`] on purpose — one
    /// means the endpoint refused us, the other means we declined to ask.
    Cooldown { until: i64 },
    /// Anything else. Text is already [`redact`]ed.
    Error(String),
}

impl Status {
    /// A row is worth rendering when it's showing real numbers or a real
    /// problem — not when the provider simply isn't installed.
    pub fn visible(&self) -> bool {
        !matches!(self, Status::Disabled | Status::Absent)
    }
}

/// One rate-limit window (e.g. the rolling 5-hour session limit).
#[derive(Clone, Debug)]
pub struct Window {
    /// The window itself, model included where it's scoped to one —
    /// `Session · 5h`, `Week`, `Week · Fable`.
    pub label: String,
    /// Plain-English statement of what the percentage covers, e.g. `all models
    /// on your plan` or `this model only`. A bare `37%` is unreadable without
    /// it, so every window carries one.
    pub scope: String,
    /// Percent of the window consumed, 0..=100.
    pub pct: f64,
    /// Unix seconds at which the window resets, if the provider told us.
    pub resets_at: Option<i64>,
    /// The provider says this is the window currently constraining you — of
    /// several overlapping limits, the one you'll actually hit.
    pub active: bool,
}

/// A pay-as-you-go credit balance: money committed, not throttling.
///
/// Deliberately *not* a [`Window`]. Credits and rate limits are different
/// things with different remedies, and folding spend into the window list would
/// let a topped-up balance turn the bar red for a reason that has nothing to do
/// with being throttled.
#[derive(Clone, Debug)]
pub struct Spend {
    /// Used, in the currency's major unit (dollars, not cents).
    pub used: f64,
    pub limit: f64,
    pub currency: String,
    /// Percent of the limit committed, 0..=100.
    pub pct: f64,
}

impl Spend {
    fn new(used_minor: f64, limit_minor: f64, currency: &str, exponent: u32) -> Self {
        let scale = 10f64.powi(exponent as i32);
        let (used, limit) = (used_minor / scale, limit_minor / scale);
        let pct = if limit > 0.0 { (used / limit * 100.0).clamp(0.0, 100.0) } else { 0.0 };
        Spend { used, limit, currency: currency.to_string(), pct }
    }

    /// `$20.01` — a symbol for the currencies we can render, the ISO code
    /// otherwise, so an unexpected currency reads correctly rather than being
    /// silently mislabelled as dollars.
    pub fn money(&self, amount: f64) -> String {
        match self.currency.as_str() {
            "USD" => format!("${amount:.2}"),
            "EUR" => format!("€{amount:.2}"),
            "GBP" => format!("£{amount:.2}"),
            other => format!("{amount:.2} {other}"),
        }
    }
}

/// What we computed from local logs — always available, never leaves the box.
#[derive(Clone, Debug, Default)]
pub struct Local {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// API-equivalent cost in USD. On a subscription you don't pay this — it's
    /// what the same traffic would have cost through the API.
    pub cost_usd: f64,
    pub messages: u64,
}

impl Local {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Clone, Debug)]
pub struct Provider {
    /// Display name.
    pub name: &'static str,
    /// Plan/tier if the provider volunteered one ("max", "pro", …).
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    /// Credit balance, shown apart from `windows`. See [`Spend`].
    pub spend: Option<Spend>,
    pub local: Option<Local>,
    /// Unix seconds at which the stored OAuth session stops working, when the
    /// credential file says. Surfaced so a pending re-login is visible *before*
    /// it starts failing, not after.
    pub session_expires: Option<i64>,
    pub status: Status,
}

impl Provider {
    fn new(name: &'static str) -> Self {
        Provider {
            name,
            plan: None,
            windows: Vec::new(),
            spend: None,
            local: None,
            session_expires: None,
            status: Status::Disabled,
        }
    }

    /// The window closest to its limit — what the bar label shows.
    pub fn peak(&self) -> Option<&Window> {
        self.windows.iter().max_by(|a, b| a.pct.total_cmp(&b.pct))
    }
}

/// Everything the bar knows about AI usage right now.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub providers: Vec<Provider>,
    /// Unix seconds of the last refresh.
    pub updated: i64,
    /// Whether the network path was enabled for this refresh. Reported so an empty
    /// dump reads as "switched off" rather than "broken".
    pub live: bool,
}

impl Snapshot {
    /// Highest window utilisation across every provider — drives the bar label
    /// and its warn/critical colour.
    pub fn peak_pct(&self) -> Option<f64> {
        self.peak_window().map(|w| w.pct)
    }

    /// When the window the bar is showing resets — the second half of the
    /// module's answer. "68%" alone does not tell you whether to slow down;
    /// "68%, 4h 12m left" does.
    pub fn peak_resets_at(&self) -> Option<i64> {
        self.peak_window().and_then(|w| w.resets_at)
    }

    /// The single window `peak_pct` is reporting.
    fn peak_window(&self) -> Option<&Window> {
        self.providers
            .iter()
            .filter(|p| p.status.visible())
            .filter_map(|p| p.peak())
            .max_by(|a, b| a.pct.total_cmp(&b.pct))
    }

    /// True when there is nothing worth showing — the module hides itself.
    pub fn is_empty(&self) -> bool {
        !self.providers.iter().any(|p| p.status.visible())
    }
}

// ===========================================================================
// Config
// ===========================================================================

#[derive(Clone, Debug)]
pub struct AiConfig {
    pub enabled: bool,
    /// Provider keys to poll, in display order.
    pub providers: Vec<String>,
    /// Poll interval, seconds (clamped to >= [`MIN_INTERVAL`]).
    pub interval: u32,
    /// Allow network requests at all.
    pub live: bool,
    /// Compute today's token/cost totals from local logs.
    pub local: bool,
    /// Percent thresholds for the amber / red label colours.
    pub warn: f64,
    pub critical: f64,
}

impl Default for AiConfig {
    fn default() -> Self {
        // Off by default: this is the one module that can make a network
        // request, so a fresh install never does until the user says so.
        AiConfig {
            enabled: false,
            providers: vec!["anthropic".to_string()],
            interval: 300,
            live: true,
            local: true,
            warn: 60.0,
            critical: 85.0,
        }
    }
}

// ===========================================================================
// Poll thread
// ===========================================================================

/// Start polling. Returns immediately; updates arrive on `tx`.
///
/// One thread, one blocking sleep between rounds — no tokio, no async runtime,
/// matching the rest of the bar. Providers are polled in config order and a
/// failure in one never stops the others.
pub fn spawn(cfg: AiConfig, tx: async_channel::Sender<Snapshot>) {
    if !cfg.enabled || cfg.providers.is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("tezca-ai".into())
        .spawn(move || {
            let interval = cfg.interval.max(MIN_INTERVAL);
            // The Claude Code version string for the User-Agent; resolved once
            // (it costs a process spawn) and reused for the process lifetime.
            let ua = claude_code_ua();
            // Incremental state for the local log analytics, so a poll re-reads only
            // what was appended since the last one.
            let mut cache = LocalCache::default();

            loop {
                // Reload each round: a `--ai-dump` in another process may have
                // recorded an attempt or earned a 429 since we last looked.
                let mut holds = Holds::load();
                let mut snap =
                    Snapshot { providers: Vec::new(), updated: now_unix(), live: cfg.live };
                for key in &cfg.providers[..] {
                    snap.providers
                        .push(poll_provider(key, &cfg, &ua, interval, &mut holds, &mut cache));
                }
                holds.save();

                if tx.send_blocking(snap).is_err() {
                    return; // bar gone
                }
                std::thread::sleep(Duration::from_secs(interval as u64));
            }
        })
        .ok();
}

/// Poll every configured provider exactly once, on the calling thread.
///
/// Backs `tezca-bar --ai-dump`, which is how you check what this module can
/// actually see without restarting the bar or reading a popover. Honours the
/// same config — in particular `ai_live = false` keeps it entirely offline.
pub fn poll_once(cfg: &AiConfig) -> Snapshot {
    let ua = claude_code_ua();
    // A dump is a debugging tool, so it still reports local numbers when the module
    // is switched off — but it must not reach the network on behalf of someone who
    // never opted in. `ai_enabled` gates the network path here exactly as it gates
    // the resident poll thread.
    let effective = AiConfig { live: cfg.live && cfg.enabled, ..cfg.clone() };
    // Honour the same shared hold the resident bar writes. A dump used to bypass it
    // entirely, so running one in a loop could earn a 429 in the bucket this
    // module's own rules exist to protect — including Claude Code's share of it.
    let mut holds = Holds::load();
    let mut cache = LocalCache::default();
    let snap = Snapshot {
        providers: effective
            .providers
            .iter()
            .map(|k| poll_provider(k, &effective, &ua, MIN_INTERVAL, &mut holds, &mut cache))
            .collect(),
        updated: now_unix(),
        live: effective.live,
    };
    holds.save();
    snap
}

/// Render a snapshot as plain text for `--ai-dump`. Contains no credential
/// material — the most it ever names is the plan tier the provider reported.
pub fn dump(snap: &Snapshot) -> String {
    let mut s = String::new();
    if snap.providers.is_empty() {
        return "no providers configured (set ai_enabled + ai_providers)\n".to_string();
    }
    if !snap.live {
        s.push_str(
            "note: the network path is off (ai_enabled = false or ai_live = false), \
             so only local numbers are shown.\n\n",
        );
    }
    for p in &snap.providers {
        let plan = p.plan.as_deref().map(|x| format!(" · {x}")).unwrap_or_default();
        s.push_str(&format!("{}{}\n", p.name, plan));
        s.push_str(&format!("  status    {:?}\n", p.status));
        if let Some(t) = p.session_expires {
            let when = if t <= now_unix() {
                "expired".to_string()
            } else {
                format!("expires in {}", until(t))
            };
            s.push_str(&format!("  session   {when}\n"));
        }
        for w in &p.windows {
            // `>` flags the window that is actually constraining you, when the
            // provider says which — otherwise several percentages look equally
            // important and the binding one is easy to miss.
            let mark = if w.active { '>' } else { ' ' };
            let r = w.resets_at.map(|t| format!("  resets in {}", until(t))).unwrap_or_default();
            s.push_str(&format!("{mark} {:<20} {:>5.1}%{}\n", w.label, w.pct, r));
            s.push_str(&format!("    {}\n", w.scope));
        }
        if let Some(sp) = &p.spend {
            // Money, not throttling — listed apart from the windows above and
            // excluded from `peak`.
            s.push_str(&format!(
                "  {:<20} {:>5.1}%  {} of {}\n",
                "Extra credits",
                sp.pct,
                sp.money(sp.used),
                sp.money(sp.limit)
            ));
        }
        if let Some(l) = &p.local {
            s.push_str(&format!(
                "  local         {} tok  (in {} · out {} · cache-r {} · cache-w {})\n",
                compact_count(l.total_tokens()),
                compact_count(l.input_tokens),
                compact_count(l.output_tokens),
                compact_count(l.cache_read_tokens),
                compact_count(l.cache_write_tokens),
            ));
            s.push_str(&format!(
                "  api-equiv     ${:.2} across {} messages\n",
                l.cost_usd, l.messages
            ));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "peak: {}\n",
        snap.peak_pct().map(|p| format!("{p:.0}%")).unwrap_or("—".into())
    ));
    s
}

fn blank_provider(key: &str) -> Provider {
    match key {
        "anthropic" => Provider::new("Claude"),
        "openai" => Provider::new("Codex"),
        "google" => Provider::new("Gemini"),
        _ => Provider::new("Unknown"),
    }
}

fn poll_provider(
    key: &str,
    cfg: &AiConfig,
    ua: &str,
    interval: u32,
    holds: &mut Holds,
    cache: &mut LocalCache,
) -> Provider {
    match key {
        "anthropic" => anthropic(cfg, ua, interval, holds, cache),
        "openai" => openai(cfg),
        "google" => google(cfg),
        _ => blank_provider(key),
    }
}

// ===========================================================================
// Anthropic
// ===========================================================================

const ANTHROPIC_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Required by the endpoint; without it the request 401s.
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";

fn anthropic(
    cfg: &AiConfig,
    ua: &str,
    interval: u32,
    holds: &mut Holds,
    cache: &mut LocalCache,
) -> Provider {
    let mut p = Provider::new("Claude");

    if cfg.local {
        p.local = anthropic_local(cache).ok();
    }

    if !cfg.live {
        p.status = if p.local.is_some() { Status::LocalOnly } else { Status::Absent };
        return p;
    }

    // Two gates before any request, both driven by the on-disk hold so they apply
    // across processes.
    let now = now_unix();
    let hold = holds.get("anthropic");
    if now < hold.until {
        p.status = Status::RateLimited { until: hold.until };
        return p;
    }
    if hold.last_attempt > 0 && now - hold.last_attempt < MIN_INTERVAL as i64 {
        p.status = Status::Cooldown { until: hold.last_attempt + MIN_INTERVAL as i64 };
        return p;
    }

    let creds = match anthropic_credentials() {
        Some(c) => c,
        // No Claude Code login on this machine — nothing to show, no error.
        None => {
            p.status = if p.local.is_some() { Status::LocalOnly } else { Status::Absent };
            return p;
        }
    };
    p.plan = creds.plan;
    p.session_expires = creds.expires_at;

    // An expired access token 401s with certainty, so sending the request buys
    // nothing — and it lands in the same rate-limit bucket as Claude Code, which
    // means a bar polling a dead session can earn *your CLI* a 429. Skip it and
    // say what to do instead.
    if creds.expires_at.is_some_and(|t| t <= now_unix()) {
        p.status = Status::NeedsLogin;
        return p;
    }

    let headers = [
        format!("Authorization: Bearer {}", creds.token),
        format!("anthropic-beta: {ANTHROPIC_BETA}"),
    ];
    holds.record_attempt("anthropic", now);
    match curl_get(ANTHROPIC_USAGE_URL, &headers, ua) {
        // Compute the retry time here, where it can be reported. It used to be
        // filled in by the caller *after* the snapshot was built, so the popover
        // showed `until: 0` — i.e. "retry now" — on the very round a 429 landed.
        Ok((429, _)) => {
            let until = holds.back_off("anthropic", now, interval);
            p.status = Status::RateLimited { until };
        }
        // An expiry the file didn't warn us about — same remedy either way, and
        // refreshing it is Claude Code's job: we never write its credential file.
        Ok((401, _)) | Ok((403, _)) => p.status = Status::NeedsLogin,
        Ok((code, _)) if !(200..300).contains(&code) => {
            p.status = Status::Error(format!("HTTP {code}"));
        }
        Ok((_, body)) => match serde_json::from_str::<Value>(&body) {
            Ok(v) => {
                holds.succeeded("anthropic");
                p.windows = parse_usage(&v);
                p.spend = parse_spend(&v);
                if p.plan.is_none() {
                    p.plan =
                        v.get("subscription_type").and_then(|s| s.as_str()).map(str::to_string);
                }
                p.status = if p.windows.is_empty() {
                    // Endpoint answered in a shape we don't recognise — say so
                    // rather than silently rendering an empty row.
                    Status::Error("unrecognised response shape".into())
                } else {
                    Status::Ok
                };
            }
            Err(e) => p.status = Status::Error(redact(&e.to_string())),
        },
        Err(e) => p.status = Status::Error(e),
    }
    p
}

/// Turn the usage payload into windows.
///
/// The endpoint states the same facts twice: a modern `limits: [...]` array and
/// the legacy top-level `five_hour` / `seven_day` / `seven_day_<model>` objects
/// it grew out of. The array is authoritative — it is the only form that says
/// *what* a window covers (`scope.model`) and which of several overlapping
/// limits is currently binding (`is_active`) — so when it is present we build
/// from it alone. Reading both is what made the popover render every number
/// twice, three of the rows labelled only "Limit".
///
/// Money is deliberately excluded. `extra_usage` and `spend` describe a
/// pay-as-you-go credit balance rather than a rate limit; they are parsed
/// separately by [`parse_spend`] so they can never drive the bar's colour.
fn parse_usage(v: &Value) -> Vec<Window> {
    let Some(obj) = v.as_object() else { return Vec::new() };

    if let Some(arr) = obj.get("limits").and_then(|l| l.as_array()) {
        let out: Vec<Window> = arr.iter().filter_map(window_from_limit).collect();
        if !out.is_empty() {
            return out;
        }
    }
    legacy_windows(obj)
}

/// One entry of the `limits: [...]` array:
/// `{kind, group, percent, resets_at, scope: {model: {display_name}}, is_active}`.
fn window_from_limit(item: &Value) -> Option<Window> {
    let o = item.as_object()?;
    let pct = o.get("percent").or_else(|| o.get("utilization")).and_then(|x| x.as_f64())?;
    // `name` is the older spelling of `kind`; accept either.
    let kind = o.get("kind").or_else(|| o.get("name")).and_then(|x| x.as_str()).unwrap_or("");
    let group = o.get("group").and_then(|x| x.as_str()).unwrap_or("");

    let scope_obj = o.get("scope");
    let model = scope_obj
        .and_then(|s| s.get("model"))
        .and_then(|m| m.get("display_name"))
        .and_then(|d| d.as_str());
    let surface = scope_obj.and_then(|s| s.get("surface")).and_then(|x| x.as_str());

    // The model belongs in the title — two rows both reading "Week" with
    // different numbers looks like the duplication bug this replaced.
    let mut label = limit_label(kind, group);
    if let Some(m) = model {
        label.push_str(" · ");
        label.push_str(m);
    }

    Some(Window {
        label,
        scope: describe_scope(model, surface),
        pct: pct.clamp(0.0, 100.0),
        resets_at: o.get("resets_at").and_then(|x| x.as_str()).and_then(parse_rfc3339),
        active: o.get("is_active").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

/// A name for a `limits[]` entry. `kind` is the specific bucket
/// (`weekly_scoped`), `group` the coarse window (`weekly`). We lead with the
/// window *length*, because "how long until this clears" is the first thing you
/// want from a limit you've just hit.
fn limit_label(kind: &str, group: &str) -> String {
    match kind {
        "session" => "Session · 5h".to_string(),
        "weekly_all" | "weekly_scoped" | "weekly" => "Week".to_string(),
        "daily_all" | "daily_scoped" | "daily" => "Day".to_string(),
        "monthly_all" | "monthly_scoped" | "monthly" => "Month".to_string(),
        // Unknown bucket: fall back to the group so a new limit type still says
        // how long its window is, and never render an empty title.
        _ => match (group, titleize(kind)) {
            (_, t) if !t.is_empty() => t,
            ("session", _) => "Session".to_string(),
            ("weekly", _) => "Week".to_string(),
            ("daily", _) => "Day".to_string(),
            _ => "Limit".to_string(),
        },
    }
}

/// Plain English for what a window measures. `scope: null` means the limit
/// applies to everything on the plan; a scoped entry names the model (and
/// sometimes the surface) it is restricted to.
fn describe_scope(model: Option<&str>, surface: Option<&str>) -> String {
    match (model, surface) {
        (Some(_), Some(s)) => format!("this model on {s} only"),
        (Some(_), None) => "this model only".to_string(),
        (None, Some(s)) => format!("{s} only"),
        (None, None) => "all models on your plan".to_string(),
    }
}

/// The pre-`limits[]` shape: one object per window at the top level. Kept as a
/// fallback so an account still on the old response — or a rollback of the
/// endpoint — keeps rendering, and so a window type we have no name for shows
/// up with a tidied key instead of disappearing.
fn legacy_windows(obj: &serde_json::Map<String, Value>) -> Vec<Window> {
    /// Key, display name, and what it covers — in the order we show them.
    const KNOWN: &[(&str, &str, &str)] = &[
        ("five_hour", "Session · 5h", "all models on your plan"),
        ("seven_day", "Week", "all models on your plan"),
        ("seven_day_opus", "Week · Opus", "this model only"),
        ("seven_day_sonnet", "Week · Sonnet", "this model only"),
    ];
    /// Money-shaped keys. They carry a `utilization` field and would otherwise
    /// be mistaken for rate-limit windows. See [`parse_spend`].
    const NOT_WINDOWS: &[&str] = &["extra_usage", "spend"];

    let mut out = Vec::new();
    for (key, label, scope) in KNOWN {
        if let Some((pct, resets)) = window_from(obj.get(*key)) {
            out.push(Window {
                label: (*label).to_string(),
                scope: (*scope).to_string(),
                pct,
                resets_at: resets,
                active: false,
            });
        }
    }
    for (k, val) in obj {
        if KNOWN.iter().any(|(n, _, _)| n == k) || NOT_WINDOWS.contains(&k.as_str()) {
            continue;
        }
        if let Some((pct, resets)) = window_from(Some(val)) {
            out.push(Window {
                label: titleize(k),
                // We don't know what an unrecognised key covers, and guessing
                // "all models" could be a lie. Say so.
                scope: "scope not reported".to_string(),
                pct,
                resets_at: resets,
                active: false,
            });
        }
    }
    out
}

/// Pay-as-you-go credit balance, if the account has one enabled.
///
/// The payload states it twice: `spend` in minor units with an explicit
/// `exponent`, and `extra_usage` in the same minor units with `decimal_places`.
/// We prefer `spend` (its units are self-describing) and fall back.
fn parse_spend(v: &Value) -> Option<Spend> {
    if let Some(s) = v.get("spend").and_then(|s| s.as_object()) {
        // Absent `enabled` is treated as enabled — the amounts are the real
        // evidence, and an explicit `false` is what we're looking to skip.
        if s.get("enabled").and_then(|e| e.as_bool()) != Some(false) {
            let amount = |k: &str| s.get(k)?.get("amount_minor")?.as_f64();
            if let (Some(used), Some(limit)) = (amount("used"), amount("limit")) {
                let unit = |k: &str| s.get("used").and_then(|u| u.get(k));
                let exp = unit("exponent").and_then(|x| x.as_u64()).unwrap_or(2) as u32;
                let cur = unit("currency").and_then(|x| x.as_str()).unwrap_or("USD");
                return Some(Spend::new(used, limit, cur, exp));
            }
        }
    }

    let e = v.get("extra_usage")?.as_object()?;
    if e.get("is_enabled").and_then(|x| x.as_bool()) != Some(true) {
        return None;
    }
    let used = e.get("used_credits").and_then(|x| x.as_f64())?;
    let limit = e.get("monthly_limit").and_then(|x| x.as_f64())?;
    let exp = e.get("decimal_places").and_then(|x| x.as_u64()).unwrap_or(2) as u32;
    let cur = e.get("currency").and_then(|x| x.as_str()).unwrap_or("USD");
    Some(Spend::new(used, limit, cur, exp))
}

/// `(percent, resets_at)` if this value looks like a usage window.
fn window_from(v: Option<&Value>) -> Option<(f64, Option<i64>)> {
    let o = v?.as_object()?;
    let pct = o
        .get("utilization")
        .or_else(|| o.get("percent"))
        .or_else(|| o.get("used_percent"))
        .and_then(|x| x.as_f64())?;
    let resets = o
        .get("resets_at")
        .or_else(|| o.get("reset_at"))
        .and_then(|x| x.as_str())
        .and_then(parse_rfc3339);
    Some((pct.clamp(0.0, 100.0), resets))
}

fn titleize(k: &str) -> String {
    let mut s = String::new();
    for (i, part) in k.split('_').enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let mut c = part.chars();
        if let Some(f) = c.next() {
            s.extend(f.to_uppercase());
            s.push_str(c.as_str());
        }
    }
    s
}

struct Creds {
    token: String,
    plan: Option<String>,
    /// Unix seconds, normalised from whatever unit the file used.
    expires_at: Option<i64>,
}

/// Read the OAuth token Claude Code already stores. We only ever read this
/// file — never write, never copy, never cache it anywhere.
fn anthropic_credentials() -> Option<Creds> {
    let path = home()?.join(".claude").join(".credentials.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;

    // Walk for the access token rather than hardcoding the nesting — the file's
    // shape has changed across Claude Code versions.
    let token = find_str(&v, &|k| {
        let k = k.to_ascii_lowercase();
        k.contains("access") && k.contains("token")
    })?;
    // A token with a quote or newline would break out of the curl config file
    // we're about to build. Refuse rather than construct something unsafe.
    if token.is_empty() || token.contains('"') || token.contains('\n') || token.contains('\\') {
        return None;
    }
    // Prefer the rate-limit tier (`default_claude_max_5x`) over the coarser
    // subscription type (`max`): the tier is what the percentages below are a
    // percentage *of*, so it's the more informative label to sit next to them.
    let plan = find_str(&v, &|k| k.eq_ignore_ascii_case("ratelimittier") || k == "rate_limit_tier")
        .map(|t| pretty_tier(&t))
        .filter(|t| !t.is_empty())
        .or_else(|| {
            find_str(&v, &|k| {
                let k = k.to_ascii_lowercase();
                k.contains("subscription") || k == "plan" || k == "tier"
            })
        });
    // Exact key match, so the much later `refreshTokenExpiresAt` can't be
    // mistaken for the access token's own (far nearer) expiry.
    let expires_at = find_num(&v, &|k| {
        let k = k.to_ascii_lowercase();
        k == "expiresat" || k == "expires_at"
    })
    .map(to_unix_secs);
    Some(Creds { token, plan, expires_at })
}

/// `default_claude_max_5x` → `max 5x`. The raw tier carries a bucket prefix and
/// a vendor name that earn nothing in a 280px popover.
fn pretty_tier(raw: &str) -> String {
    raw.split('_')
        .filter(|p| !matches!(*p, "default" | "claude" | ""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Credential timestamps are milliseconds in every version of the file we've
/// seen, but the field is undocumented — so infer the unit from the magnitude
/// rather than trusting it. The threshold is year ~5138 in seconds, which no
/// real expiry will reach and no millisecond value will fall below.
fn to_unix_secs(n: i64) -> i64 {
    if n > 100_000_000_000 {
        n / 1000
    } else {
        n
    }
}

/// Depth-first search for the first string value whose key matches `pred`.
fn find_str(v: &Value, pred: &dyn Fn(&str) -> bool) -> Option<String> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if pred(k) {
                    if let Some(s) = val.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
            map.values().find_map(|val| find_str(val, pred))
        }
        Value::Array(a) => a.iter().find_map(|val| find_str(val, pred)),
        _ => None,
    }
}

/// Depth-first search for the first integer value whose key matches `pred`.
fn find_num(v: &Value, pred: &dyn Fn(&str) -> bool) -> Option<i64> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if pred(k) {
                    if let Some(n) = val.as_i64() {
                        return Some(n);
                    }
                }
            }
            map.values().find_map(|val| find_num(val, pred))
        }
        Value::Array(a) => a.iter().find_map(|val| find_num(val, pred)),
        _ => None,
    }
}

/// `claude-code/<version>` for the User-Agent. The endpoint buckets requests by
/// UA and an unrecognised one lands in an aggressively rate-limited pool, so
/// this matters even though it looks cosmetic.
fn claude_code_ua() -> String {
    let ver = Command::new("claude")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()
                .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .map(str::to_string)
        })
        // The version string is interpolated into the curl config file below, where
        // a quote would break the `user-agent = "…"` line. `split_whitespace`
        // already rules out a newline (so no directive can be injected), but apply
        // the same character check the bearer token gets rather than relying on
        // that. An odd version falls back to the default instead of shipping a
        // malformed config.
        .filter(|v| !v.contains('"') && !v.contains('\\'))
        .unwrap_or_else(|| "2.0.0".to_string());
    format!("claude-code/{ver}")
}

// --- local JSONL analytics -------------------------------------------------

/// How deep to walk under `~/.claude/projects` looking for session logs.
const MAX_LOG_DEPTH: usize = 8;

/// One assistant message's measured contribution.
///
/// Kept per file rather than pre-summed, because dedup is *global*: the same
/// message can appear in more than one session file (a resumed or forked session),
/// and a pre-summed per-file total gives no way to subtract the duplicate at merge
/// time.
#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    /// `message id : request id` — the dedup key. Empty when the log had neither,
    /// in which case the entry always counts (it cannot be matched against others).
    key: String,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost: f64,
}

/// What we have already read from one session log.
#[derive(Clone, Debug, Default)]
struct FileState {
    /// Bytes consumed so far. Only ever advanced to just past a newline, so a line
    /// still being written is re-read whole on the next poll instead of being parsed
    /// in half.
    offset: u64,
    entries: Vec<Entry>,
}

/// Incremental state for [`anthropic_local`].
///
/// Without this, every poll read each of today's session logs in full and JSON-parsed
/// every line — tens to hundreds of megabytes every interval for a heavy Claude Code
/// user. Session logs are append-only, so the tail is the only part that can be new.
#[derive(Debug, Default)]
pub struct LocalCache {
    /// The local midnight this cache describes. A day rollover invalidates it.
    day_start: i64,
    files: HashMap<PathBuf, FileState>,
}

/// Sum today's token usage from Claude Code's own session logs. Pure local
/// filesystem work — no network, no credentials, nothing leaves the machine.
/// This is the same data `ccusage` reads.
fn anthropic_local(cache: &mut LocalCache) -> Result<Local, ()> {
    let root = home().ok_or(())?.join(".claude").join("projects");
    let start = local_midnight_unix();

    // A new day means yesterday's entries are no longer "today's usage".
    if cache.day_start != start {
        cache.files.clear();
        cache.day_start = start;
    }

    let files = jsonl_files(&root);
    // Forget logs that have gone away, so the cache can't grow without bound across
    // a long-running bar process.
    let live: HashSet<&PathBuf> = files.iter().collect();
    cache.files.retain(|p, _| live.contains(p));

    for file in &files {
        let Ok(meta) = std::fs::metadata(file) else { continue };
        let touched_today = meta
            .modified()
            .map(|t| t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) >= start)
            .unwrap_or(false);
        // Only a file touched today can contain today's messages. Anything already
        // in the cache keeps its entries either way — `retain` above only dropped
        // files that no longer exist — so skipping here just means "nothing new to
        // read", not "forget what we read".
        if !touched_today {
            continue;
        }

        let len = meta.len();
        let st = cache.files.entry(file.clone()).or_default();
        // Shorter than what we read means the file was replaced or rotated, not
        // appended to — start it over rather than reading from a stale offset.
        if len < st.offset {
            *st = FileState::default();
        }
        if len == st.offset {
            continue; // nothing appended since last poll
        }
        if let Ok((entries, consumed)) = read_tail(file, st.offset, start) {
            st.entries.extend(entries);
            st.offset += consumed;
        }
    }

    // Merge every file's entries with a single global dedup pass.
    let mut local = Local::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for st in cache.files.values() {
        for e in &st.entries {
            if !e.key.is_empty() && !seen.insert(e.key.as_str()) {
                continue;
            }
            add(&mut local, e);
        }
    }
    Ok(local)
}

/// Read the part of `path` after `offset`, returning the entries found and how many
/// bytes were consumed.
///
/// Only whole lines are consumed. The writer appends continuously, so the tail
/// frequently ends mid-line; stopping at the last newline means that line is read
/// again — complete — on the next poll, rather than being parsed as truncated JSON
/// and dropped.
fn read_tail(path: &Path, offset: u64, start: i64) -> std::io::Result<(Vec<Entry>, u64)> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut raw = Vec::new();
    f.read_to_end(&mut raw)?;

    let Some(nl) = raw.iter().rposition(|b| *b == b'\n') else {
        return Ok((Vec::new(), 0)); // no complete line yet
    };
    let complete = &raw[..=nl];
    let text = String::from_utf8_lossy(complete);
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(e) = parse_entry(line, start) {
            out.push(e);
        }
    }
    Ok((out, complete.len() as u64))
}

/// One JSONL line → an [`Entry`], if it is a billable assistant message from today.
fn parse_entry(line: &str, start: i64) -> Option<Entry> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    let ts = v.get("timestamp").and_then(|t| t.as_str()).and_then(parse_rfc3339);
    if ts.is_none_or(|t| t < start) {
        return None;
    }
    let msg = v.get("message")?;
    let key = format!(
        "{}:{}",
        msg.get("id").and_then(|i| i.as_str()).unwrap_or(""),
        v.get("requestId").and_then(|i| i.as_str()).unwrap_or("")
    );
    let u = msg.get("usage")?;
    let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("");
    // A line carrying neither id keeps an empty key, which always counts: there is
    // nothing to match it against, so dropping it would under-report instead.
    Some(usage_entry(if key == ":" { String::new() } else { key }, u, model))
}

/// Price one `usage` object.
fn usage_entry(key: String, u: &Value, model: &str) -> Entry {
    let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let input = n("input_tokens");
    let output = n("output_tokens");
    let cache_read = n("cache_read_input_tokens");
    // Split the cache writes by TTL — the 1h tier is priced higher.
    let (w5, w1h) = match u.get("cache_creation") {
        Some(c) => (
            c.get("ephemeral_5m_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            c.get("ephemeral_1h_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        ),
        None => (n("cache_creation_input_tokens"), 0),
    };

    let p = price_for(model);
    let per_m = |t: u64, rate: f64| (t as f64) * rate / 1_000_000.0;
    Entry {
        key,
        input,
        output,
        cache_read,
        cache_write: w5 + w1h,
        cost: per_m(input, p.input)
            + per_m(output, p.output)
            + per_m(cache_read, p.input * 0.10)
            + per_m(w5, p.input * 1.25)
            + per_m(w1h, p.input * 2.00),
    }
}

fn add(local: &mut Local, e: &Entry) {
    local.input_tokens += e.input;
    local.output_tokens += e.output;
    local.cache_read_tokens += e.cache_read;
    local.cache_write_tokens += e.cache_write;
    local.cost_usd += e.cost;
    local.messages += 1;
}

/// USD per million tokens.
struct Price {
    input: f64,
    output: f64,
}

/// End of Claude Sonnet 5's introductory pricing: 2026-09-01T00:00:00Z.
///
/// Sonnet 5 lists at $3/$15 but bills at $2/$10 through 2026-08-31, so pricing it
/// at list over-reports Sonnet traffic by 50% while the intro rate is live. The
/// cutoff is computed from the civil date rather than hardcoded as an epoch so it
/// stays legible.
fn sonnet5_intro_ends() -> i64 {
    days_from_civil(2026, 9, 1) * 86_400
}

/// Published list prices, matched by model-family substring so a new dated
/// snapshot of a known family still prices correctly. Cache rates are derived:
/// reads are 0.1x input, 5-minute writes 1.25x, 1-hour writes 2x.
fn price_for(model: &str) -> Price {
    price_at(model, now_unix())
}

/// [`price_for`] with the clock injected, so the introductory window is testable.
fn price_at(model: &str, now: i64) -> Price {
    let m = model.to_ascii_lowercase();
    if m.contains("fable") || m.contains("mythos") {
        Price { input: 10.0, output: 50.0 }
    } else if m.contains("haiku") {
        Price { input: 1.0, output: 5.0 }
    } else if m.contains("sonnet") {
        // `sonnet-5`, not `sonnet` + `-5`: `claude-sonnet-4-5` also ends in `-5`
        // and is not on the introductory rate.
        if m.contains("sonnet-5") && now < sonnet5_intro_ends() {
            Price { input: 2.0, output: 10.0 }
        } else {
            Price { input: 3.0, output: 15.0 }
        }
    } else {
        // Opus tier, and the safe default for anything unrecognised.
        Price { input: 5.0, output: 25.0 }
    }
}

/// Every `*.jsonl` under `root`, walking subdirectories.
///
/// The previous version descended exactly one level while the module doc claimed
/// `projects/**/*.jsonl`, so a nested project directory was silently missed. Depth
/// is bounded and symlinked directories are skipped, so a loop cannot hang the poll
/// thread.
fn jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_logs(root, 0, &mut out);
    out
}

fn walk_logs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_LOG_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        // `file_type` on the DirEntry does not follow symlinks, which is what keeps
        // a symlinked directory from turning this walk into a cycle.
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            walk_logs(&path, depth + 1, out);
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

// ===========================================================================
// OpenAI / Codex  (unverified — no ~/.codex on this machine)
// ===========================================================================

/// Codex keeps its rate-limit state behind its own app-server rather than a
/// documented HTTP endpoint, so we ask the local `codex` binary over stdio
/// JSON-RPC. Nothing leaves the machine that Codex wouldn't send itself, and no
/// credential passes through us — the CLI reads its own `auth.json`.
///
/// Untested here (Codex isn't installed); auto-hides when it's absent.
fn openai(cfg: &AiConfig) -> Provider {
    let mut p = Provider::new("Codex");
    let home = match home() {
        Some(h) => h,
        None => return p,
    };
    let codex_home =
        std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or(home.join(".codex"));
    if !codex_home.join("auth.json").exists() || which("codex").is_none() {
        p.status = Status::Absent;
        return p;
    }
    if !cfg.live {
        p.status = Status::LocalOnly;
        return p;
    }

    match codex_rate_limits() {
        Ok(v) => {
            let rl = v.get("rateLimits").unwrap_or(&v);
            // Codex reports two windows and does not scope either to a model.
            for (key, label) in [("primary", "Session · 5h"), ("secondary", "Week")] {
                if let Some(o) = rl.get(key).and_then(|x| x.as_object()) {
                    let pct = o.get("usedPercent").and_then(|x| x.as_f64());
                    let resets = o.get("resetsAt").and_then(|x| x.as_str()).and_then(parse_rfc3339);
                    if let Some(pct) = pct {
                        p.windows.push(Window {
                            label: label.to_string(),
                            scope: "all models on your plan".to_string(),
                            pct: pct.clamp(0.0, 100.0),
                            resets_at: resets,
                            active: false,
                        });
                    }
                }
            }
            p.status = if p.windows.is_empty() {
                Status::Error("unrecognised response shape".into())
            } else {
                Status::Ok
            };
        }
        Err(e) => p.status = Status::Error(e),
    }
    p
}

/// Drive `codex app-server` far enough to read `account/rateLimits/read`.
fn codex_rate_limits() -> Result<Value, String> {
    let mut child = Command::new("codex")
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| redact(&e.to_string()))?;

    {
        let stdin = child.stdin.as_mut().ok_or("no stdin")?;
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"tezca-bar","version":"0.1.0"}}}"#;
        let read = r#"{"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":{}}"#;
        writeln!(stdin, "{init}").map_err(|e| redact(&e.to_string()))?;
        writeln!(stdin, "{read}").map_err(|e| redact(&e.to_string()))?;
        stdin.flush().ok();
    }
    // Closing stdin lets the server exit once it has answered, so `output()`
    // returns instead of blocking forever on a long-lived process.
    drop(child.stdin.take());

    let out = child.wait_with_output().map_err(|e| redact(&e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("id").and_then(|i| i.as_u64()) == Some(2) {
            return v.get("result").cloned().ok_or_else(|| "no result".to_string());
        }
    }
    Err("no response".into())
}

// ===========================================================================
// Google / Gemini  (approximate — local session files only)
// ===========================================================================

/// Google publishes no per-account quota endpoint, so this counts tokens out of
/// the Gemini CLI's own local chat logs. It is a **warning signal, not a
/// quota**: it can't see usage from other machines and it won't catch you at
/// exactly request 1000. Presented as a token count, never as a percentage, so
/// the UI never implies a precision we don't have.
fn google(cfg: &AiConfig) -> Provider {
    let mut p = Provider::new("Gemini");
    let Some(home) = home() else { return p };
    let root = home.join(".gemini").join("tmp");
    if !root.exists() {
        p.status = Status::Absent;
        return p;
    }
    let _ = cfg;

    let start = local_midnight_unix();
    let mut local = Local::default();
    let mut found = false;

    for chat in gemini_chat_files(&root) {
        let fresh = std::fs::metadata(&chat)
            .and_then(|m| m.modified())
            .map(|t| t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) >= start)
            .unwrap_or(false);
        if !fresh {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&chat) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let Some(msgs) = v.get("messages").and_then(|m| m.as_array()) else { continue };
        for m in msgs {
            if let Some(t) = m.get("tokens").and_then(|t| t.get("total")).and_then(|t| t.as_u64()) {
                local.input_tokens += t;
                local.messages += 1;
                found = true;
            }
        }
    }

    if found {
        p.local = Some(local);
        p.status = Status::LocalOnly;
    } else {
        p.status = Status::Absent;
    }
    p
}

fn gemini_chat_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else { return out };
    for d in dirs.flatten() {
        let chats = d.path().join("chats");
        let Ok(files) = std::fs::read_dir(chats) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
    out
}

// ===========================================================================
// HTTP (via curl, with the token kept off argv)
// ===========================================================================

/// GET `url` with `headers`, returning `(status, body)`.
///
/// The whole request — URL, headers, timeout, protocol pin — is written as a
/// curl config file to **stdin** (`curl -K -`). This is the security-critical
/// detail of the module: a header passed as `-H` would appear in
/// `/proc/<pid>/cmdline`, readable by every process running as this user, for
/// as long as the request lasts. On stdin it never touches argv or disk.
fn curl_get(url: &str, headers: &[String], ua: &str) -> Result<(u16, String), String> {
    // Allowlist check before anything else — no config value can widen this.
    //
    // Matched as an exact `https://<host>` prefix rather than by splitting the
    // string on '/'. Prefix-matching an allowlist is normally the fragile choice,
    // but here it is the strict one: it is the *whole* origin that has to match, so
    // there is no host string left to mis-parse. Splitting compared a substring that
    // could still carry userinfo or a port (`https://api.anthropic.com@evil/`), and
    // was case-sensitive. Both were refused in practice — the check fails closed —
    // but only by accident of the comparison, not by construction.
    if !allowlisted(url) {
        return Err("refusing to contact a non-allowlisted host".to_string());
    }

    let mut conf = String::new();
    conf.push_str(&format!("url = \"{url}\"\n"));
    for h in headers {
        conf.push_str(&format!("header = \"{h}\"\n"));
    }
    conf.push_str(&format!("user-agent = \"{ua}\"\n"));
    conf.push_str("silent\nshow-error\n");
    // Refuse plaintext and refuse to follow redirects: a compromised or spoofed
    // response cannot bounce our bearer token to another host.
    conf.push_str("proto = \"=https\"\nproto-redir = \"=https\"\n");
    conf.push_str(&format!("max-time = {HTTP_TIMEOUT}\n"));
    conf.push_str("write-out = \"\\n%{http_code}\"\n");

    let mut child = Command::new("curl")
        .arg("-K")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| redact(&e.to_string()))?;
    child
        .stdin
        .as_mut()
        .ok_or("no stdin")?
        .write_all(conf.as_bytes())
        .map_err(|e| redact(&e.to_string()))?;
    drop(child.stdin.take());

    let out = child.wait_with_output().map_err(|e| redact(&e.to_string()))?;
    if !out.status.success() && out.stdout.is_empty() {
        return Err(redact(&String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // The status code is the last line, appended by write-out.
    let (body, code) = match text.rsplit_once('\n') {
        Some((b, c)) => (b, c.trim().parse::<u16>().unwrap_or(0)),
        None => (text.as_ref(), 0),
    };
    Ok((code, body.to_string()))
}

// ===========================================================================
// Small helpers
// ===========================================================================

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(bin)).find(|p| p.is_file())
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Local midnight as unix seconds — the "today" boundary for local analytics.
fn local_midnight_unix() -> i64 {
    match gtk4::glib::DateTime::now_local() {
        Ok(now) => {
            gtk4::glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), 0, 0, 0.0)
                .map(|d| d.to_unix())
                .unwrap_or(0)
        }
        Err(_) => 0,
    }
}

/// Parse an RFC 3339 timestamp to unix seconds. Handles `Z` and `±HH:MM`
/// offsets and fractional seconds; returns `None` on anything else rather than
/// guessing.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);

    let mut secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec;

    // Timezone suffix: skip any fractional part first.
    let rest = &s[19..];
    let rest = rest.strip_prefix('.').map_or(rest, |f| {
        let n = f.chars().take_while(|c| c.is_ascii_digit()).count();
        &f[n..]
    });
    if let Some(sign) = rest.chars().next() {
        if sign == '+' || sign == '-' {
            let off = &rest[1..];
            let oh: i64 = off.get(0..2)?.parse().ok()?;
            let om: i64 = off.get(3..5).and_then(|m| m.parse().ok()).unwrap_or(0);
            let delta = oh * 3600 + om * 60;
            secs += if sign == '+' { -delta } else { delta };
        }
    }
    Some(secs)
}

/// Days since the Unix epoch for a civil date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2h 14m" / "4d 3h" / "now" — a compact countdown for the popover.
pub fn until(unix: i64) -> String {
    let d = unix - now_unix();
    if d <= 0 {
        return "now".to_string();
    }
    let (days, hours, mins) = (d / 86_400, (d % 86_400) / 3600, (d % 3600) / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins:02}m")
    } else {
        format!("{mins}m")
    }
}

/// "just now" / "4m ago" / "2h ago" — how stale the snapshot is. Shown in the
/// popover so a frozen poll thread is visible rather than silently serving
/// numbers from an hour ago.
pub fn ago(unix: i64) -> String {
    let d = now_unix() - unix;
    if d < 60 {
        return "just now".to_string();
    }
    let (h, m) = (d / 3600, (d % 3600) / 60);
    if h > 0 {
        format!("{h}h {m:02}m ago")
    } else {
        format!("{m}m ago")
    }
}

/// "1.2M" / "847K" / "312" — compact token counts.
pub fn compact_count(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.0}K", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_token_shaped_runs() {
        let e = "failed: Bearer sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz012345";
        let r = redact(e);
        assert!(!r.contains("AbCdEfGhIjKlMnOpQrStUvWxYz"), "{r}");
        assert!(r.contains("<redacted>"), "{r}");
        // Short words survive, so the message is still useful.
        assert!(redact("HTTP 429 too many").contains("429"));
    }

    #[test]
    fn parses_rfc3339_forms() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339("2026-02-06T22:00:00+00:00"),
            parse_rfc3339("2026-02-06T22:00:00Z")
        );
        // +02:00 is two hours *ahead*, so the same wall clock is earlier in UTC.
        let z = parse_rfc3339("2026-02-06T22:00:00Z").unwrap();
        assert_eq!(parse_rfc3339("2026-02-07T00:00:00+02:00"), Some(z));
        assert_eq!(
            parse_rfc3339("2026-07-23T02:31:12.785Z"),
            parse_rfc3339("2026-07-23T02:31:12Z")
        );
        assert_eq!(parse_rfc3339("nope"), None);
    }

    /// Trimmed from a real response, 2026-07-23 — the legacy keys and the
    /// `limits` array describing the *same* two windows, plus a third that
    /// exists only in the array, plus the balance stated twice.
    const LIVE: &str = r#"{
        "five_hour":{"utilization":2.0,"resets_at":"2026-07-23T19:39:59.953630+00:00"},
        "seven_day":{"utilization":37.0,"resets_at":"2026-07-25T12:00:00.953652+00:00"},
        "seven_day_opus":null,"seven_day_sonnet":null,"tangelo":null,
        "extra_usage":{"is_enabled":true,"monthly_limit":9900,"used_credits":2001.0,
                       "utilization":20.21,"currency":"USD","decimal_places":2},
        "limits":[
          {"kind":"session","group":"session","percent":2,
           "resets_at":"2026-07-23T19:39:59.953630+00:00","scope":null,"is_active":false},
          {"kind":"weekly_all","group":"weekly","percent":37,
           "resets_at":"2026-07-25T12:00:00.953652+00:00","scope":null,"is_active":false},
          {"kind":"weekly_scoped","group":"weekly","percent":61,
           "resets_at":"2026-07-25T11:59:59.953987+00:00",
           "scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":true}],
        "spend":{"used":{"amount_minor":2001,"currency":"USD","exponent":2},
                 "limit":{"amount_minor":9900,"currency":"USD","exponent":2},
                 "percent":20,"enabled":true}
    }"#;

    #[test]
    fn prefers_the_limits_array_over_the_legacy_duplicate_keys() {
        let w = parse_usage(&serde_json::from_str(LIVE).unwrap());
        // Three windows, not the seven the old both-forms parse produced.
        assert_eq!(w.len(), 3, "{w:#?}");
        assert_eq!(w[0].label, "Session · 5h");
        assert_eq!(w[0].scope, "all models on your plan");
        assert_eq!(w[1].label, "Week");
        // The model-scoped weekly limit is named, not rendered as bare "Limit".
        assert_eq!(w[2].label, "Week · Fable");
        assert_eq!(w[2].scope, "this model only");
        assert_eq!(w[2].pct, 61.0);
        assert!(w[2].active && !w[0].active);
    }

    #[test]
    fn keeps_credit_spend_out_of_the_rate_limit_windows() {
        let v: Value = serde_json::from_str(LIVE).unwrap();
        // Both `extra_usage` and `spend` carry percent-shaped fields; neither
        // may become a window, or money would colour the bar.
        assert!(parse_usage(&v).iter().all(|w| w.pct != 20.0 && !w.label.contains("Spend")));

        let sp = parse_spend(&v).expect("balance");
        assert_eq!(sp.money(sp.used), "$20.01");
        assert_eq!(sp.money(sp.limit), "$99.00");
        assert!((sp.pct - 20.21).abs() < 0.01, "{}", sp.pct);
    }

    #[test]
    fn falls_back_to_the_legacy_shape_when_there_is_no_limits_array() {
        let v: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":35.0,"resets_at":"2026-02-06T22:00:00+00:00"},
                "seven_day":{"utilization":14.0,"resets_at":"2026-02-12T20:00:00+00:00"},
                "seven_day_opus":{"utilization":31.5}}"#,
        )
        .unwrap();
        let w = parse_usage(&v);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].label, "Session · 5h");
        assert_eq!(w[0].pct, 35.0);
        assert!(w[0].resets_at.is_some());
        assert_eq!(w[2].label, "Week · Opus");
        assert_eq!(w[2].scope, "this model only");
        assert_eq!(w[2].resets_at, None);
    }

    #[test]
    fn surfaces_unknown_windows_instead_of_dropping_them() {
        let v: Value = serde_json::from_str(r#"{"thirty_day_fable":{"utilization":9.0}}"#).unwrap();
        let w = parse_usage(&v);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].label, "Thirty Day Fable");
        // We don't know what it covers, so we don't claim to.
        assert_eq!(w[0].scope, "scope not reported");
    }

    #[test]
    fn names_a_scoped_window_even_when_the_kind_is_unfamiliar() {
        let v: Value = serde_json::from_str(
            r#"{"limits":[{"kind":"monthly_scoped","group":"monthly","percent":5,
                "scope":{"model":{"display_name":"Opus"},"surface":"web"},"is_active":false}]}"#,
        )
        .unwrap();
        let w = parse_usage(&v);
        assert_eq!(w[0].label, "Month · Opus");
        assert_eq!(w[0].scope, "this model on web only");
    }

    #[test]
    fn ignores_shapes_that_are_not_windows() {
        let v: Value =
            serde_json::from_str(r#"{"account_uuid":"x","five_hour":{"utilization":1.0}}"#)
                .unwrap();
        assert_eq!(parse_usage(&v).len(), 1);
    }

    #[test]
    fn prices_by_model_family() {
        let after = sonnet5_intro_ends();
        assert_eq!(price_at("claude-opus-4-8", after).input, 5.0);
        assert_eq!(price_at("claude-opus-5", after).output, 25.0);
        assert_eq!(price_at("claude-haiku-4-5", after).input, 1.0);
        assert_eq!(price_at("claude-fable-5", after).output, 50.0);
        // Unknown models fall back to Opus tier rather than to zero.
        assert_eq!(price_at("something-new", after).input, 5.0);
    }

    #[test]
    fn prices_sonnet_5_at_its_introductory_rate_until_the_cutoff() {
        let (before, after) = (sonnet5_intro_ends() - 1, sonnet5_intro_ends());
        // $2/$10 through 2026-08-31, $3/$15 list afterwards. Pricing it at list
        // during the window over-reported Sonnet traffic by 50%.
        assert_eq!(price_at("claude-sonnet-5", before).input, 2.0);
        assert_eq!(price_at("claude-sonnet-5", before).output, 10.0);
        assert_eq!(price_at("claude-sonnet-5", after).input, 3.0);
        assert_eq!(price_at("claude-sonnet-5", after).output, 15.0);
        // Only Sonnet 5. `claude-sonnet-4-5` also ends in "-5" and must not match.
        assert_eq!(price_at("claude-sonnet-4-5", before).input, 3.0);
        assert_eq!(price_at("claude-sonnet-4-6", before).input, 3.0);
    }

    #[test]
    fn prices_cache_tiers_at_their_own_rates() {
        let u: Value = serde_json::from_str(
            r#"{"input_tokens":1000000,"output_tokens":0,"cache_read_input_tokens":1000000,
                "cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":1000000}}"#,
        )
        .unwrap();
        let mut l = Local::default();
        add(&mut l, &usage_entry("m:r".into(), &u, "claude-opus-4-8"));
        // 1M input @ $5 + 1M cache-read @ $0.50 + 1M 1h-write @ $10.
        assert!((l.cost_usd - 15.5).abs() < 1e-9, "{}", l.cost_usd);
        assert_eq!(l.total_tokens(), 3_000_000);
    }

    #[test]
    fn a_message_in_two_session_files_is_counted_once() {
        // A resumed or forked session writes the same assistant message to a second
        // file. Entries are kept per file precisely so this dedup can happen once,
        // globally, at merge time.
        let u: Value = serde_json::from_str(r#"{"input_tokens":10,"output_tokens":5}"#).unwrap();
        let dup = usage_entry("msg_1:req_1".into(), &u, "claude-opus-5");
        let other = usage_entry("msg_2:req_2".into(), &u, "claude-opus-5");

        let mut local = Local::default();
        let mut seen: HashSet<&str> = HashSet::new();
        for e in [&dup, &dup, &other] {
            if !e.key.is_empty() && !seen.insert(e.key.as_str()) {
                continue;
            }
            add(&mut local, e);
        }
        assert_eq!(local.messages, 2, "the duplicate must not be counted twice");
        assert_eq!(local.total_tokens(), 30);
    }

    #[test]
    fn an_entry_with_no_ids_always_counts_rather_than_colliding() {
        // Two different messages that both lack ids must not dedup against each
        // other — an empty key means "unmatchable", not "the same message".
        let u: Value = serde_json::from_str(r#"{"input_tokens":1,"output_tokens":1}"#).unwrap();
        let a = usage_entry(String::new(), &u, "claude-opus-5");
        let b = usage_entry(String::new(), &u, "claude-opus-5");
        let mut local = Local::default();
        let mut seen: HashSet<&str> = HashSet::new();
        for e in [&a, &b] {
            if !e.key.is_empty() && !seen.insert(e.key.as_str()) {
                continue;
            }
            add(&mut local, e);
        }
        assert_eq!(local.messages, 2);
    }

    #[test]
    fn parses_only_billable_assistant_lines_from_today() {
        let start = 1_700_000_000;
        let line = |extra: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-07-29T12:00:00Z","requestId":"r1",
                    "message":{{"id":"m1","model":"claude-opus-5","usage":{{"input_tokens":7,"output_tokens":3}}}}{extra}}}"#
            )
        };
        // A well-formed assistant line from after `start` counts.
        let e = parse_entry(&line(""), start).expect("assistant line");
        assert_eq!((e.input, e.output), (7, 3));
        assert_eq!(e.key, "m1:r1");

        // A user line, a line with no usage, and one from before `start` do not.
        assert!(
            parse_entry(r#"{"type":"user","timestamp":"2026-07-29T12:00:00Z"}"#, start).is_none()
        );
        assert!(parse_entry(
            r#"{"type":"assistant","timestamp":"2026-07-29T12:00:00Z","message":{"id":"m"}}"#,
            start
        )
        .is_none());
        assert!(parse_entry(&line(""), i64::MAX).is_none(), "an older message is out of scope");
        assert!(parse_entry("not json at all", start).is_none());
    }

    #[test]
    fn reads_only_complete_lines_and_resumes_from_the_byte_offset() {
        // The append-only read is the load-bearing part of the incremental cache: an
        // offset that lands mid-line would drop a message permanently, and one that
        // fails to advance would double-count it.
        let dir = std::env::temp_dir().join(format!("tezca-ai-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");

        let msg = |id: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-07-29T12:00:00Z","requestId":"r{id}","message":{{"id":"m{id}","model":"claude-opus-5","usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            )
        };

        // Two whole lines plus a third still being written.
        let partial = format!("{}\n{}\n{}", msg("1"), msg("2"), &msg("3")[..40]);
        std::fs::write(&path, &partial).unwrap();

        let (entries, consumed) = read_tail(&path, 0, 0).unwrap();
        assert_eq!(entries.len(), 2, "the half-written line must not be parsed");
        assert_eq!(entries[0].key, "m1:r1");
        let expected = format!("{}\n{}\n", msg("1"), msg("2"));
        assert_eq!(consumed as usize, expected.len(), "must stop at the last newline");

        // The writer finishes that line and appends another.
        std::fs::write(&path, format!("{}\n{}\n{}\n", msg("1"), msg("2"), msg("3"))).unwrap();
        let (more, _) = read_tail(&path, consumed, 0).unwrap();
        assert_eq!(more.len(), 1, "resuming must yield the completed line exactly once");
        assert_eq!(more[0].key, "m3:r3");

        // Nothing appended → nothing returned, and no bytes consumed.
        let end = std::fs::metadata(&path).unwrap().len();
        assert_eq!(read_tail(&path, end, 0).unwrap(), (Vec::new(), 0));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_logs_nested_more_than_one_level_deep() {
        // The old walk descended exactly one level while the module doc claimed
        // `projects/**/*.jsonl`, so a nested project silently reported zero usage.
        let dir = std::env::temp_dir().join(format!("tezca-ai-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proj/nested/deeper")).unwrap();
        std::fs::write(dir.join("proj/a.jsonl"), "").unwrap();
        std::fs::write(dir.join("proj/nested/deeper/b.jsonl"), "").unwrap();
        std::fs::write(dir.join("proj/not-a-log.txt"), "").unwrap();

        let mut found: Vec<String> = jsonl_files(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, ["a.jsonl", "b.jsonl"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hold_round_trips_and_doubles_without_exceeding_the_cap() {
        let mut h = Holds::default();
        assert_eq!(h.get("anthropic"), Hold::default());

        // First 429 waits one interval; each further one doubles, up to MAX_BACKOFF.
        let until = h.back_off("anthropic", 1_000, 300);
        assert_eq!(until, 1_300);
        assert_eq!(h.back_off("anthropic", 1_000, 300), 1_600);
        for _ in 0..20 {
            h.back_off("anthropic", 1_000, 300);
        }
        assert_eq!(h.get("anthropic").backoff, MAX_BACKOFF);

        // Success clears the backoff but keeps last_attempt, which is what enforces
        // the minimum interval between live requests.
        h.record_attempt("anthropic", 2_000);
        h.succeeded("anthropic");
        let after = h.get("anthropic");
        assert_eq!((after.until, after.backoff), (0, 0));
        assert_eq!(after.last_attempt, 2_000);
    }

    #[test]
    fn prettifies_the_rate_limit_tier() {
        assert_eq!(pretty_tier("default_claude_max_5x"), "max 5x");
        assert_eq!(pretty_tier("default_claude_pro"), "pro");
        // Nothing recognisable left over reads as empty, so the caller can fall
        // back to `subscriptionType` rather than showing a blank chip.
        assert_eq!(pretty_tier("default_claude"), "");
    }

    #[test]
    fn normalises_credential_expiry_to_seconds() {
        // Milliseconds, as the file actually stores it.
        assert_eq!(to_unix_secs(1_784_846_814_159), 1_784_846_814);
        // Already seconds — left alone rather than divided down into 1970.
        assert_eq!(to_unix_secs(1_784_846_814), 1_784_846_814);
    }

    #[test]
    fn expiry_lookup_ignores_the_refresh_token_field() {
        // `refreshTokenExpiresAt` is months out; mistaking it for the access
        // token's expiry would defeat the whole pre-check.
        let v: Value = serde_json::from_str(
            r#"{"claudeAiOauth":{"accessToken":"x","expiresAt":1784846814159,
                "refreshTokenExpiresAt":9999999999999}}"#,
        )
        .unwrap();
        let got = find_num(&v, &|k| {
            let k = k.to_ascii_lowercase();
            k == "expiresat" || k == "expires_at"
        });
        assert_eq!(got, Some(1_784_846_814_159));
    }

    #[test]
    fn refuses_hosts_outside_the_allowlist() {
        let r = curl_get("https://evil.example.com/x", &[], "ua");
        assert!(r.unwrap_err().contains("non-allowlisted"));
    }

    #[test]
    fn the_allowlist_matches_the_whole_origin_and_nothing_adjacent_to_it() {
        assert!(allowlisted("https://api.anthropic.com/api/oauth/usage"));
        assert!(allowlisted("https://api.anthropic.com"));

        // Each of these was refused before too, but by the accident of a string
        // compare rather than by construction.
        assert!(!allowlisted("https://api.anthropic.com.evil.test/x"));
        assert!(!allowlisted("https://api.anthropic.com@evil.test/x"));
        assert!(!allowlisted("https://api.anthropic.com:8443/x"));
        assert!(!allowlisted("https://evil.test/api.anthropic.com"));
        assert!(!allowlisted("http://api.anthropic.com/x"), "plaintext is not allowed");
        assert!(!allowlisted("https://API.ANTHROPIC.COM/x"), "fails closed on casing");
        assert!(!allowlisted(""));
    }

    #[test]
    fn formats_countdowns_and_counts() {
        assert_eq!(until(now_unix() - 5), "now");
        assert_eq!(compact_count(312), "312");
        assert_eq!(compact_count(847_000), "847K");
        assert_eq!(compact_count(1_234_567), "1.2M");
    }
}
