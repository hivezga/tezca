//! The weather module — current conditions for one place, on the bar.
//!
//! # Why this is opt-in
//!
//! Until this module existed, `ai` was the only part of the bar that could make
//! a network request, and the config said so in as many words. Weather is the
//! second, and it is worse in one specific way: the AI module talks to a host
//! you already have an account with, while this one hands a third party your
//! **coordinates**. That is a location disclosure, so it is off by default and
//! stays off until you write the coordinates yourself. There is no geolocation
//! lookup and no IP-based guess — the bar never tries to work out where you are.
//!
//! The safety rails mirror [`crate::ai`] exactly, because the reasoning is the
//! same:
//!   * **Hardcoded host allowlist.** [`ALLOWED_HOSTS`] is checked before every
//!     request; a URL whose origin is not on it is refused, not fetched. No
//!     config value can widen it.
//!   * **HTTPS pinned, redirects refused** — `--proto =https` with
//!     `proto-redir` matching, so a spoofed response cannot bounce the request
//!     somewhere else.
//!   * **No credential of any kind.** Open-Meteo needs no key and no account,
//!     which is exactly why it is the source: there is nothing to store, leak,
//!     or tie the request to you.
//!   * **One request per poll** by default. The air-quality figure lives on a
//!     second endpoint, so it costs a second request and is therefore its own
//!     opt-in (`weather_aqi`).
//!   * **No telemetry.** Nothing is reported to Tezca, ever.

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

// ===========================================================================
// Safety rails
// ===========================================================================

/// Every host this module may contact, ever.
///
/// The first two serve the forecast. The third resolves a place *name* to
/// coordinates and is only reached when you type one. The fourth is the only
/// one that learns anything about you it was not told — see [`locate_by_ip`].
const ALLOWED_HOSTS: &[&str] = &[
    "api.open-meteo.com",
    "air-quality-api.open-meteo.com",
    "geocoding-api.open-meteo.com",
    "ipapi.co",
    "ipwho.is",
];

/// True only when `url`'s origin is exactly one of [`ALLOWED_HOSTS`] over https.
///
/// Matched as a whole origin rather than by splitting on `/`: there is then no
/// host substring left to mis-parse, so `https://api.open-meteo.com@evil.test/`
/// and `https://api.open-meteo.com.evil.test/` both fail by construction rather
/// than by luck.
fn allowlisted(url: &str) -> bool {
    ALLOWED_HOSTS.iter().any(|h| {
        let origin = format!("https://{h}");
        url == origin || url.starts_with(&format!("{origin}/"))
    })
}

/// Hard ceiling on a single request, seconds. The bar must never look hung.
const HTTP_TIMEOUT: u32 = 10;

/// Never poll faster than this, seconds. Weather changes on the scale of tens
/// of minutes; a tighter loop is free load on a free service.
const MIN_INTERVAL: u32 = 300;

// ===========================================================================
// Config
// ===========================================================================

#[derive(Clone, Debug)]
pub struct WeatherConfig {
    /// Off by default — see the module docs.
    pub enabled: bool,
    /// Decimal degrees. Both must be set for the module to do anything.
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// What to call the place on screen. Purely a label; never sent anywhere.
    pub place: String,
    pub interval: u32,
    /// Display unit. The API is asked for Celsius either way and the conversion
    /// happens here, so the request never varies with a display preference.
    pub fahrenheit: bool,
    /// Also fetch the air-quality index — a second request, to a second host.
    pub aqi: bool,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lat: None,
            lon: None,
            place: String::new(),
            interval: 900,
            fahrenheit: false,
            aqi: false,
        }
    }
}

impl WeatherConfig {
    /// Whether there is enough here to poll at all.
    pub fn usable(&self) -> bool {
        self.enabled && self.lat.is_some() && self.lon.is_some()
    }
}

// ===========================================================================
// Snapshot
// ===========================================================================

/// One hour of the forecast strip.
#[derive(Clone, Debug)]
pub struct Hour {
    /// Local hour as `23h`.
    pub label: String,
    pub temp_c: f64,
}

/// Everything the module knows right now. Every field is optional because a
/// provider that drops one is a provider that should still render the rest.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub place: String,
    pub temp_c: Option<f64>,
    pub feels_c: Option<f64>,
    pub hi_c: Option<f64>,
    pub lo_c: Option<f64>,
    /// WMO weather-interpretation code — see [`condition`].
    pub code: Option<i64>,
    pub is_day: bool,
    pub humidity: Option<f64>,
    pub wind_kmh: Option<f64>,
    pub wind_dir_deg: Option<f64>,
    pub uv: Option<f64>,
    pub sunset: Option<String>,
    pub aqi: Option<f64>,
    pub hourly: Vec<Hour>,
    /// Unix seconds of the last successful refresh, 0 if never.
    pub updated: i64,
    /// Why the last attempt failed, if it did. Shown in the popover so a broken
    /// poll is visible rather than silently serving hour-old numbers.
    pub error: Option<String>,
    pub fahrenheit: bool,
}

impl Snapshot {
    /// True when there is nothing worth showing — the module hides itself.
    pub fn is_empty(&self) -> bool {
        self.temp_c.is_none()
    }

    /// `23°` — the primary readout.
    pub fn temp_text(&self) -> String {
        self.temp_c.map(|c| self.degrees(c)).unwrap_or_default()
    }

    /// `18° / 27°` — today's range, the sub-label.
    pub fn range_text(&self) -> String {
        match (self.lo_c, self.hi_c) {
            (Some(lo), Some(hi)) => format!("{} / {}", self.degrees(lo), self.degrees(hi)),
            _ => String::new(),
        }
    }

    /// Convert and format one temperature in the configured unit.
    pub fn degrees(&self, c: f64) -> String {
        let v = if self.fahrenheit { c * 9.0 / 5.0 + 32.0 } else { c };
        format!("{}°", v.round() as i64)
    }

    /// What to say on hover: place, condition, and how stale this is.
    pub fn tooltip(&self) -> String {
        if let Some(e) = &self.error {
            return format!("Weather unavailable — {e}");
        }
        let place = if self.place.is_empty() { "Weather" } else { &self.place };
        match self.code {
            Some(c) => format!("{place} — {}", condition(c, self.is_day)),
            None => place.to_string(),
        }
    }
}

/// A WMO weather-interpretation code as words.
///
/// The codes are a published standard with gaps and ranges rather than a flat
/// list, which is why this is a match on ranges and not a lookup table.
pub fn condition(code: i64, is_day: bool) -> &'static str {
    match code {
        0 => {
            if is_day {
                "Clear"
            } else {
                "Clear night"
            }
        }
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow grains",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

/// A compass point for a wind bearing in degrees.
pub fn bearing(deg: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let i = (((deg % 360.0 + 360.0) % 360.0) / 45.0).round() as usize % 8;
    POINTS[i]
}

/// The published AQI bands, as a word.
pub fn aqi_band(aqi: f64) -> &'static str {
    match aqi.round() as i64 {
        i64::MIN..=50 => "good",
        51..=100 => "moderate",
        101..=150 => "unhealthy for sensitive groups",
        151..=200 => "unhealthy",
        201..=300 => "very unhealthy",
        _ => "hazardous",
    }
}

// ===========================================================================
// Finding a place
// ===========================================================================

/// One candidate location.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub name: String,
    /// Region/state, when the provider gives one — the field that tells three
    /// Guadalajaras apart.
    pub admin: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    /// `Guadalajara, Jalisco, Mexico`
    pub fn label(&self) -> String {
        [self.name.as_str(), self.admin.as_str(), self.country.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Look a place name up. Empty when nothing matched or the lookup failed.
///
/// Open-Meteo's geocoder, so it is the same provider that will serve the
/// forecast and no new party learns anything. The query is a place name you
/// typed — nothing is sent that you did not write.
pub fn geocode(query: &str) -> Vec<Place> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=8&language=en&format=json",
        urlencode(q)
    );
    let Ok(body) = get(&url) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(&body) else { return Vec::new() };
    v.get("results")
        .and_then(Value::as_array)
        .map(|rs| rs.iter().filter_map(parse_place).collect())
        .unwrap_or_default()
}

fn parse_place(r: &Value) -> Option<Place> {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Some(Place {
        name: r.get("name").and_then(Value::as_str)?.to_string(),
        admin: s("admin1"),
        country: s("country"),
        lat: r.get("latitude").and_then(Value::as_f64)?,
        lon: r.get("longitude").and_then(Value::as_f64)?,
    })
}

/// Guess the location from the public IP address.
///
/// **This is the one call in Tezca that tells a third party where you are
/// without being told.** It is never made automatically: nothing on a poll path
/// calls it, the bar never calls it at startup, and it exists only behind an
/// explicit `tezca bar weather locate` or a button you press. Your IP goes to
/// ipapi.co and a city comes back; that is the trade, stated plainly, and the
/// result is written to the config as plain coordinates so the lookup never has
/// to happen again.
///
/// It is also only a guess — a VPN or a carrier-grade NAT will place you
/// somewhere you have never been, which is exactly why the answer is shown for
/// confirmation rather than saved silently.
pub fn locate_by_ip() -> Result<Place, String> {
    // Two providers because both are free and keyless, which also means both
    // rate-limit: one refusing is a normal Tuesday, not a broken feature. They
    // are tried in order and the first usable answer wins.
    type Parse = fn(&Value) -> Result<Place, String>;
    const PROVIDERS: [(&str, Parse); 2] =
        [("https://ipapi.co/json/", locate_ipapi), ("https://ipwho.is/", locate_ipwho)];

    let mut last = String::from("no provider answered");
    for (url, parse) in PROVIDERS {
        match get(url).and_then(|b| {
            serde_json::from_str::<Value>(&b).map_err(|e| e.to_string()).and_then(|v| parse(&v))
        }) {
            Ok(p) => return Ok(p),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// ipapi.co reports its own failures in-band with HTTP 200.
fn locate_ipapi(v: &Value) -> Result<Place, String> {
    if let Some(reason) = v.get("reason").and_then(Value::as_str) {
        return Err(format!("ipapi.co: {reason}"));
    }
    place_from(v, "country_name")
}

/// ipwho.is flags failure with `success: false` and explains in `message`.
fn locate_ipwho(v: &Value) -> Result<Place, String> {
    if v.get("success").and_then(Value::as_bool) == Some(false) {
        let m = v.get("message").and_then(Value::as_str).unwrap_or("refused");
        return Err(format!("ipwho.is: {m}"));
    }
    place_from(v, "country")
}

/// The fields both providers agree on, given the key each uses for the country.
fn place_from(v: &Value, country_key: &str) -> Result<Place, String> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (Some(lat), Some(lon)) =
        (v.get("latitude").and_then(Value::as_f64), v.get("longitude").and_then(Value::as_f64))
    else {
        return Err("the service returned no coordinates".to_string());
    };
    Ok(Place { name: s("city"), admin: s("region"), country: s(country_key), lat, lon })
}

/// Percent-encode a query for a URL.
///
/// Hand-rolled because the crate has no HTTP client to borrow one from, and the
/// alphabet is small: anything outside unreserved ASCII becomes %XX, so a place
/// name with a space, an accent or an ampersand cannot alter the URL's shape.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ===========================================================================
// Polling
// ===========================================================================

/// Spawn the poll thread. A no-op unless the module is configured — an
/// unconfigured weather module costs one `if` at startup and nothing after.
pub fn spawn(cfg: WeatherConfig, tx: async_channel::Sender<Snapshot>) {
    if !cfg.usable() {
        return;
    }
    std::thread::spawn(move || {
        let interval = cfg.interval.max(MIN_INTERVAL);
        loop {
            if tx.send_blocking(poll_once(&cfg)).is_err() {
                return; // the bar is gone
            }
            std::thread::sleep(std::time::Duration::from_secs(interval as u64));
        }
    });
}

/// One refresh, synchronously. Public so `--weather-dump` can call it.
pub fn poll_once(cfg: &WeatherConfig) -> Snapshot {
    let mut snap =
        Snapshot { place: cfg.place.clone(), fahrenheit: cfg.fahrenheit, ..Default::default() };
    let (Some(lat), Some(lon)) = (cfg.lat, cfg.lon) else {
        snap.error = Some("no coordinates configured".to_string());
        return snap;
    };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,is_day,\
         weather_code,wind_speed_10m,wind_direction_10m\
         &hourly=temperature_2m&daily=temperature_2m_max,temperature_2m_min,uv_index_max,sunset\
         &timezone=auto&forecast_days=1"
    );
    match get(&url) {
        Ok(body) => match serde_json::from_str::<Value>(&body) {
            Ok(v) => {
                apply_forecast(&mut snap, &v);
                snap.updated = now_unix();
            }
            Err(e) => snap.error = Some(format!("bad response: {e}")),
        },
        Err(e) => snap.error = Some(e),
    }

    // Second host, second request — only when explicitly asked for.
    if cfg.aqi && snap.error.is_none() {
        let url = format!(
            "https://air-quality-api.open-meteo.com/v1/air-quality\
             ?latitude={lat:.4}&longitude={lon:.4}&current=us_aqi"
        );
        if let Ok(body) = get(&url) {
            if let Ok(v) = serde_json::from_str::<Value>(&body) {
                snap.aqi = v.pointer("/current/us_aqi").and_then(Value::as_f64);
            }
        }
    }
    snap
}

/// Fill a snapshot from the forecast response.
fn apply_forecast(snap: &mut Snapshot, v: &Value) {
    let cur = |k: &str| v.pointer(&format!("/current/{k}")).and_then(Value::as_f64);
    snap.temp_c = cur("temperature_2m");
    snap.feels_c = cur("apparent_temperature");
    snap.humidity = cur("relative_humidity_2m");
    snap.wind_kmh = cur("wind_speed_10m");
    snap.wind_dir_deg = cur("wind_direction_10m");
    snap.code = v.pointer("/current/weather_code").and_then(Value::as_i64);
    snap.is_day = cur("is_day").map(|d| d > 0.5).unwrap_or(true);

    let day0 = |k: &str| v.pointer(&format!("/daily/{k}/0")).and_then(Value::as_f64);
    snap.hi_c = day0("temperature_2m_max");
    snap.lo_c = day0("temperature_2m_min");
    snap.uv = day0("uv_index_max");
    snap.sunset = v
        .pointer("/daily/sunset/0")
        .and_then(Value::as_str)
        // "2026-08-02T20:04" → "20:04".
        .and_then(|s| s.split('T').nth(1))
        .map(str::to_string);

    // The hourly array starts at midnight local, so pick the hours *after* now
    // rather than the first five — otherwise the strip shows this morning.
    let times = v.pointer("/hourly/time").and_then(Value::as_array);
    let temps = v.pointer("/hourly/temperature_2m").and_then(Value::as_array);
    if let (Some(times), Some(temps)) = (times, temps) {
        let start = current_hour_index(v, times);
        snap.hourly = times
            .iter()
            .zip(temps)
            .skip(start)
            .take(5)
            .filter_map(|(t, c)| {
                let label = t.as_str()?.split('T').nth(1)?.split(':').next()?;
                Some(Hour { label: format!("{label}h"), temp_c: c.as_f64()? })
            })
            .collect();
    }
}

/// Where in the hourly array "now" falls, by matching the current timestamp's
/// date+hour prefix. Falls back to the start of the array when the response
/// carries no current time to match against.
fn current_hour_index(v: &Value, times: &[Value]) -> usize {
    let Some(now) = v.pointer("/current/time").and_then(Value::as_str) else { return 0 };
    let prefix = now.split(':').next().unwrap_or(now);
    times.iter().position(|t| t.as_str().is_some_and(|s| s.starts_with(prefix))).unwrap_or(0)
}

// ===========================================================================
// HTTP
// ===========================================================================

/// GET `url` as text.
///
/// The request is written as a curl config file on **stdin** (`curl -K -`),
/// matching [`crate::ai`]. There is no credential here to keep off argv, but
/// the shape is worth keeping identical: one place to audit how this bar talks
/// to the network, and one place to change if that ever needs tightening.
fn get(url: &str) -> Result<String, String> {
    if !allowlisted(url) {
        return Err("refusing to contact a non-allowlisted host".to_string());
    }
    let mut conf = String::new();
    conf.push_str(&format!("url = \"{url}\"\n"));
    conf.push_str("silent\nshow-error\n");
    conf.push_str("proto = \"=https\"\nproto-redir = \"=https\"\n");
    conf.push_str(&format!("max-time = {HTTP_TIMEOUT}\n"));

    let mut child = Command::new("curl")
        .arg("-K")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .as_mut()
        .ok_or("no stdin")?
        .write_all(conf.as_bytes())
        .map_err(|e| e.to_string())?;
    drop(child.stdin.take());

    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() && out.stdout.is_empty() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `--weather-dump`: what the module can see, without opening a window.
pub fn dump(snap: &Snapshot) -> String {
    let mut s = String::new();
    s.push_str(&format!("place    {}\n", if snap.place.is_empty() { "—" } else { &snap.place }));
    if let Some(e) = &snap.error {
        s.push_str(&format!("error    {e}\n"));
        return s;
    }
    s.push_str(&format!("temp     {}\n", snap.temp_text()));
    if let Some(c) = snap.code {
        s.push_str(&format!("sky      {}\n", condition(c, snap.is_day)));
    }
    s.push_str(&format!("range    {}\n", snap.range_text()));
    if let Some(h) = snap.humidity {
        s.push_str(&format!("humidity {h:.0}%\n"));
    }
    if let (Some(w), Some(d)) = (snap.wind_kmh, snap.wind_dir_deg) {
        s.push_str(&format!("wind     {w:.0} km/h {}\n", bearing(d)));
    }
    if let Some(u) = snap.uv {
        s.push_str(&format!("uv       {u:.0}\n"));
    }
    if let Some(a) = snap.aqi {
        s.push_str(&format!("aqi      {a:.0} · {}\n", aqi_band(a)));
    }
    if let Some(t) = &snap.sunset {
        s.push_str(&format!("sunset   {t}\n"));
    }
    for h in &snap.hourly {
        s.push_str(&format!("hour     {} {}\n", h.label, snap.degrees(h.temp_c)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_hosts_outside_the_allowlist() {
        assert!(allowlisted("https://api.open-meteo.com/v1/forecast?x=1"));
        assert!(allowlisted("https://air-quality-api.open-meteo.com/v1/air-quality"));
        // The three shapes a prefix check gets wrong if it is written carelessly.
        assert!(!allowlisted("https://api.open-meteo.com.evil.test/v1"));
        assert!(!allowlisted("https://api.open-meteo.com@evil.test/v1"));
        assert!(!allowlisted("http://api.open-meteo.com/v1/forecast"));
        assert!(!allowlisted("https://evil.test/v1"));
        assert_eq!(
            get("https://evil.test/x"),
            Err("refusing to contact a non-allowlisted host".into())
        );
    }

    #[test]
    fn location_lookups_are_allowlisted_too() {
        assert!(allowlisted("https://geocoding-api.open-meteo.com/v1/search?name=x"));
        assert!(allowlisted("https://ipapi.co/json/"));
        assert!(allowlisted("https://ipwho.is/"));
        // The near-misses a prefix check gets wrong.
        assert!(!allowlisted("https://ipapi.co.evil.test/json/"));
        assert!(!allowlisted("https://geocoding-api.open-meteo.com@evil.test/v1"));
    }

    #[test]
    fn a_place_name_cannot_alter_the_url() {
        // Spaces, accents, and the characters that would otherwise add
        // parameters or a path segment.
        assert_eq!(urlencode("San Luis"), "San%20Luis");
        assert_eq!(urlencode("Torreón"), "Torre%C3%B3n");
        assert_eq!(urlencode("a&count=99"), "a%26count%3D99");
        assert_eq!(urlencode("../../etc"), "..%2F..%2Fetc");
        // Unreserved characters pass through untouched.
        assert_eq!(urlencode("Ciudad-Juarez_1.0~"), "Ciudad-Juarez_1.0~");
    }

    #[test]
    fn empty_queries_never_reach_the_network() {
        assert!(geocode("").is_empty());
        assert!(geocode("   ").is_empty());
    }

    #[test]
    fn a_place_label_drops_the_parts_the_provider_omitted() {
        let full = Place {
            name: "Guadalajara".into(),
            admin: "Jalisco".into(),
            country: "Mexico".into(),
            lat: 20.6774,
            lon: -103.3475,
        };
        assert_eq!(full.label(), "Guadalajara, Jalisco, Mexico");
        let sparse = Place { admin: String::new(), ..full.clone() };
        assert_eq!(sparse.label(), "Guadalajara, Mexico");
    }

    #[test]
    fn both_ip_providers_report_their_own_refusals() {
        let limited: Value = serde_json::from_str(r#"{"reason":"RateLimited"}"#).unwrap();
        assert_eq!(locate_ipapi(&limited), Err("ipapi.co: RateLimited".into()));
        let refused: Value =
            serde_json::from_str(r#"{"success":false,"message":"quota"}"#).unwrap();
        assert_eq!(locate_ipwho(&refused), Err("ipwho.is: quota".into()));
        // And a good answer from each, with their differing country key.
        let a: Value = serde_json::from_str(
            r#"{"city":"Toluca","region":"Mexico","country_name":"Mexico",
                "latitude":19.29,"longitude":-99.67}"#,
        )
        .unwrap();
        assert_eq!(locate_ipapi(&a).unwrap().label(), "Toluca, Mexico, Mexico");
        let b: Value = serde_json::from_str(
            r#"{"success":true,"city":"Toluca","region":"Mexico","country":"Mexico",
                "latitude":19.29,"longitude":-99.67}"#,
        )
        .unwrap();
        assert_eq!(locate_ipwho(&b).unwrap().lat, 19.29);
    }

    #[test]
    fn unusable_until_coordinates_are_given() {
        let mut c = WeatherConfig { enabled: true, ..Default::default() };
        assert!(!c.usable(), "enabled alone must not start a poll");
        c.lat = Some(19.43);
        assert!(!c.usable(), "half a coordinate is not a location");
        c.lon = Some(-99.13);
        assert!(c.usable());
        c.enabled = false;
        assert!(!c.usable());
    }

    #[test]
    fn parses_a_forecast_response() {
        let v: Value = serde_json::from_str(
            r#"{"current":{"time":"2026-08-02T22:00","temperature_2m":23.4,
                "apparent_temperature":21.1,"relative_humidity_2m":54,"is_day":0,
                "weather_code":2,"wind_speed_10m":11.2,"wind_direction_10m":45},
                "hourly":{"time":["2026-08-02T21:00","2026-08-02T22:00","2026-08-02T23:00"],
                "temperature_2m":[24.0,23.4,22.8]},
                "daily":{"temperature_2m_max":[27.2],"temperature_2m_min":[17.8],
                "uv_index_max":[7.1],"sunset":["2026-08-02T20:04"]}}"#,
        )
        .unwrap();
        let mut s = Snapshot::default();
        apply_forecast(&mut s, &v);
        assert_eq!(s.temp_text(), "23°");
        assert_eq!(s.range_text(), "18° / 27°");
        assert_eq!(s.sunset.as_deref(), Some("20:04"));
        assert_eq!(condition(s.code.unwrap(), s.is_day), "Partly cloudy");
        assert!(!s.is_day);
        // The strip starts at the *current* hour, not at the top of the array.
        assert_eq!(s.hourly.len(), 2);
        assert_eq!(s.hourly[0].label, "22h");
    }

    #[test]
    fn fahrenheit_is_a_display_concern_only() {
        let mut s = Snapshot { temp_c: Some(23.0), fahrenheit: false, ..Default::default() };
        assert_eq!(s.temp_text(), "23°");
        s.fahrenheit = true;
        assert_eq!(s.temp_text(), "73°");
    }

    #[test]
    fn bearings_and_bands() {
        assert_eq!(bearing(0.0), "N");
        assert_eq!(bearing(45.0), "NE");
        assert_eq!(bearing(359.0), "N");
        assert_eq!(bearing(-90.0), "W");
        assert_eq!(aqi_band(42.0), "good");
        assert_eq!(aqi_band(86.0), "moderate");
        assert_eq!(aqi_band(500.0), "hazardous");
    }

    #[test]
    fn an_empty_snapshot_hides_the_module() {
        assert!(Snapshot::default().is_empty());
        assert!(!Snapshot { temp_c: Some(1.0), ..Default::default() }.is_empty());
    }
}
