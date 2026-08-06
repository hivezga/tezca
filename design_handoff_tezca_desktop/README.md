# Handoff: Tezca desktop UI overhaul

> **Implementing this? Start with `START_HERE.md`,** not here. It carries the
> work order, the stack decision, the rules of engagement, and how to open the
> prototypes in a browser.
>
> **`GAP_AUDIT.md`** diffs every value below against the code in the working
> tree (spec value → value found → fix) and is the source of truth for numbers.
> This document remains the source of truth for intent.

## Overview

A redesign of the visible surfaces of **Tezca**, a Hyprland/Wayland desktop environment written in Rust (`hivezga/tezca`). Three areas:

1. **`tezca-settings`** — the GTK4 control center. 13 flat sidebar pages become 3 grouped sections plus a ⌘K command palette; every control echoes the `tezca` CLI command it runs; Displays becomes a drag-to-arrange canvas.
2. **`tezca-bar`** — the layer-shell status bar. Every module surfaces the data its popover already computes; four strategies for the crowded right cluster; a local-AI module with a lateral chat panel.
3. **Shell surfaces** — dock, launcher, notification toasts, lock screen and session dialog.

Everything is grounded in the real repo: control labels come from `pages.rs`, module lists from `config/tezca-bar/config.toml`, dock physics from `magnifier.rs`, launcher chrome from `config/walker/themes/tezca/`, toast chrome from `config/swaync/style.css`, lock geometry from `config/hypr/hyprlock.conf`. Where a design proposes something that does not exist yet, it is **explicitly tagged `PROPOSED`** in the prototype — see *Proposals* below.

## About the design files

The files in `design_files/` are **design references authored in HTML** — prototypes showing intended look and behaviour. They are **not production code to copy**. They are `.dc.html` files: a template plus a small logic class.

**They do open in a browser** — this bundle ships the runtime. Serve the bundle root over HTTP (`python3 -m http.server` from the folder containing this file) and open them; `file://` will not work. Run them before reading them: the markup carries the exact inline styles, but only the running page shows the rhythm and the interactions you are matching. See `START_HERE.md` §2.

Your task is to **recreate these designs in the target codebase**:

- **`tezca-settings`** is GTK4 + `style.css` driven by `@tz_*` theme tokens. Either port the layouts into GTK4 (see *Porting to GTK4* for the constraints) or migrate the app to a web stack — the design was drawn with a Tauri/webview migration in mind and the recommendation is below.
- **`tezca-bar`** should **stay GTK4 layer-shell**. It is a `wlr-layer-shell` surface needing exclusive-zone reservation, per-output placement and low idle cost. A webview per monitor is a heavy way to draw a 40px strip.
- **Dock / launcher / toasts / lock** are existing components (`tezca-dock`, `walker`, `swaync`, `hyprlock`, `wlogout`). Most changes here are **CSS and config**, not new code.

## Fidelity

**High-fidelity.** Final colours, typography, spacing, radii and interaction behaviour. Recreate pixel-accurately using the codebase's existing patterns. Every value in this document was measured from the rendered prototype, not estimated.

Two caveats:

- All numeric telemetry (percentages, temperatures, token counts, SSIDs, battery figures) is **representative sample data**. Wire to the real sources.
- Some panels contain **proposals** — new capability, tagged in-prototype. Do not ship them as though they exist.

---

## Recommendation: Tauri vs GTK4

You asked for a pros/cons breakdown. Short version: **migrate `tezca-settings` to Tauri, keep `tezca-bar` on GTK4.**

### `tezca-settings` → Tauri

**Pros**
- It is a form-heavy config UI — 80 controls across 13 pages. This is exactly what HTML/CSS is good at.
- These designs become directly implementable rather than re-derived. Grid, flexbox, `gap`, transitions and `color-mix()` all just work.
- The ⌘K palette, drag-to-arrange canvas and confirm-or-revert toast are substantially less work in a webview.
- Theming collapses to CSS custom properties; the five themes become one token file instead of five GTK stylesheets.
- Backend stays Rust — `backend.rs` becomes Tauri commands with almost no change in shape.

**Cons**
- ~40–80 MB extra install (webkit2gtk is likely already present via other apps).
- Slower cold start (roughly 200–400 ms vs GTK's ~80 ms) — acceptable for a settings app the user opens occasionally.
- A second UI toolkit in the project. Two idioms to maintain.
- Native file pickers and the GTK theme integration need explicit bridging.

### `tezca-bar` → stay GTK4

**Pros of staying**
- Layer-shell integration is first-class: `gtk4-layer-shell` handles exclusive zones and anchoring. In a webview this is fighting the platform.
- Idle cost matters — the bar runs forever on every output. A GTK widget tree polling at 3–5 s is far cheaper than a webview per monitor.
- The self-drawn parts (sparklines, equaliser bars, Mayan numerals) are cheap in cairo and already work.
- Per-monitor instances are trivial; N webviews are not.

**Cons of staying**
- These designs need porting by hand, with GTK4 CSS's limits (below).
- No flexbox `gap` — spacing is per-widget margins.
- Complex popover layouts are more verbose than the equivalent HTML.

**Will both look as good as the prototypes?** Settings in Tauri: yes, essentially identical. Bar in GTK4: yes for the strip and popovers — the designs deliberately avoid CSS the toolkit lacks. The two things needing real work are the **grouped/expand animation** (GTK has no `max-width` transition; use a `GtkRevealer`) and the **per-core heat grid** (a `GtkDrawingArea`, not 16 styled widgets).

### Porting to GTK4 — constraints the designs respect

| Not available in GTK4 CSS | Approach used |
| --- | --- |
| `var()` custom properties | GTK named colours: `@tz_accent`. Already the pattern in `bar.css`. |
| `color-mix()` | GTK's `alpha(@tz_accent, .14)` |
| Flexbox / `gap` | `GtkBox` + `spacing` property; margins for the rest |
| Pseudo-elements | Real widgets |
| `backdrop-filter` | Layer-shell translucency + compositor blur (`hyprland` blur rules) |
| Attribute selectors | Style classes toggled from Rust (`add_css_class`) |
| CSS grid | `GtkGrid` |

Every colour in this document exists as a `@tz_*` name in `themes/<name>/colors.css`. Do not introduce new hex values.

---

## Design tokens

From `design_files/tokens/styles.css`, mirroring `themes/<name>/colors.css`. Default set is **quetzalcoatl** (the theme in use).

### Colour — quetzalcoatl (default)

| Token | Hex | Use |
| --- | --- | --- |
| `--tz-base` | `#0B0E0F` | window background |
| `--tz-surface` | `#14191B` | cards, elevated rows |
| `--tz-text` | `#E8EAED` | primary text |
| `--tz-subtext` | `#C3C8CC` | secondary text, control labels |
| `--tz-muted` | `#8B9398` | tertiary text, inactive glyphs |
| `--tz-faint` | `#5A6166` | section labels, metadata |
| `--tz-accent` | `#D2E4E2` | selection, active state, meters |
| `--tz-accent-dim` | `#A6C0BD` | gradient partner, hover |
| `--tz-on-accent` | `#0B0E0F` | text on accent fill |
| `--tz-gold` | `#C9A24B` | warn, submap, AI spend, PROPOSED markers |
| `--tz-urgent` | `#E06C75` | critical, recording, close hover |
| `--tz-line` | `rgba(139,147,152,.16)` | hairlines, card borders |
| `--tz-line-strong` | `rgba(139,147,152,.24)` | window border, emphasis |

### Theme variants

**obsidian** — `--tz-accent:#3FB8AF`, `--tz-accent-dim:#2A8C86`. Everything else as default.

**cyber** — `--tz-base:#05080A`, `--tz-surface:#0A1618`, `--tz-text:#DFF7F2`, `--tz-subtext:#A9DDD5`, `--tz-muted:#5F8F89`, `--tz-faint:#3E6A66`, `--tz-accent:#2BE8C8`, `--tz-accent-dim:#17A78F`, `--tz-on-accent:#031312`, `--tz-gold:#F0C25A`, `--tz-urgent:#FF5C7A`, `--tz-line:rgba(43,232,200,.14)`, `--tz-line-strong:rgba(43,232,200,.28)`, `--tz-radius:4px`, `--tz-radius-lg:6px`, font becomes JetBrains Mono throughout. Adds scanline background and accent glows (`box-shadow:0 0 12px -2px var(--accent)`).

**smoke** (light) — `--tz-base:#F1F4F5`, `--tz-surface:#E3E9EA`, `--tz-text:#161A1C`, `--tz-subtext:#2E3538`, `--tz-muted:#5A6467`, `--tz-faint:#8A9497`, `--tz-accent:#2A8C86`, `--tz-on-accent:#F1F4F5`, `--tz-gold:#8A6A22`, `--tz-urgent:#B23A44`, `--tz-line:rgba(22,26,28,.14)`. Shadows soften to `rgba(22,26,28,.14)`.

### Typography

- **Sans**: Inter — 400/500/600/700/800
- **Mono**: JetBrains Mono — 400/500/600/700. **Every numeric readout uses mono with `font-feature-settings:"tnum" 1`** so values don't jitter as they update.

| Role | Size / weight / family |
| --- | --- |
| Page title | 23px / 700 / sans |
| Section label | 10.5px / 700 / sans, `letter-spacing:.14em`, uppercase, faint |
| Control label | 13.5px / 600 / sans, text |
| Control hint | 12px / 400 / sans, `line-height:1.45`, muted |
| Row value | 12px / 500 / mono, subtext |
| Metadata | 9.5–10.5px / 400–500 / mono, faint |
| Bar module value | 12px / 500 / mono |
| Bar module sublabel | 9.5px / 500 / mono, faint |
| Notification summary | 14px / 600 / sans |
| Notification body | 13px / 400 / sans, `line-height:1.45` |
| Launcher query | 19px / 400 / sans |
| Launcher result | 15px / 400 / sans; subtext 12px |

### Spacing, radius, motion

- Spacing scale: 2, 4, 6, 8, 10, 12, 14, 18, 22, 26, 34 px
- Radius: `--tz-radius:10px` (controls, rows), `--tz-radius-lg:14px` (cards, windows). cyber → 4/6. Pills 50%; cyber pills 2–3px.
- Transitions: `.13s`–`.16s` for hover/state; `.2s` for OSD/toast entry; `.22s cubic-bezier(.2,.8,.3,1)` for panel slide. Dock magnification is **not** transitioned — it tracks the pointer per frame.
- Shadows: cards `0 8px 32px rgba(0,0,0,.45)`; popovers `0 20px 50px rgba(0,0,0,.6)`; window `0 40px 120px -20px rgba(0,0,0,.85)`; toasts `0 6px 24px rgba(0,0,0,.45)`.

---

## Part 1 — `tezca-settings`

Source: `design_files/settings/`. Entry is `TezcaSettings.dc.html` (shell); pages split across `PageLook`, `PageDevices`, `PageDisplays`, `PageSystem`.

### Window

1180 × 788 max, centred, `--tz-radius-lg` corners, 1px `--tz-line-strong` border. Three bands: 52px title bar, content row, optional 34px CLI echo footer.

**Title bar** — 52px, `rgba(20,25,27,.62)`, bottom hairline. Left: 20px concentric-circle logo mark in accent, then `TEZCA` (14px/800, `letter-spacing:.14em`) and `SETTINGS` (11.5px/600, uppercase, muted). Centre: search field, max 520px, 32px tall, `--tz-radius`, `rgba(11,14,15,.6)` fill, 1px hairline, magnifier glyph + "Search all settings…" (12.5px muted) + a `⌘K` cap (10.5px mono, 5px radius, hairline border). Right: three 12px dots — two faint, close is `--tz-urgent`.

### Navigation — the core restructure

13 flat items become **3 groups**. This is the main organisational change.

| Group | Pages |
| --- | --- |
| **Look & feel** | Appearance, Bar, Dock |
| **Devices** | Displays, Sound, Input, Network, Power |
| **System** | Startup, Keybinds, Gaming, System |

Sidebar 224px, `rgba(20,25,27,.34)`, right hairline, `backdrop-filter:saturate(1.2) blur(18px)`. Group headers 10px/600 uppercase `letter-spacing:.13em` faint, padding `14px 12px 7px`. Rows: 11px gap, `8px 11px` padding, `--tz-radius`, 13px/600 subtext, 17px stroked icon (1.7 width). Hover → `rgba(139,147,152,.12)` + text. Selected → `rgba(210,228,226,.14)` + `inset 2px 0 0 0 var(--accent)`.

Sidebar footer card: `--tz-radius`, surface fill, hairline — a 6px accent dot with `0 0 8px` glow, theme name (11px/600), and `Hyprland 0.51 · 2 displays` (10.5px mono faint).

**Alternate shell** (`shell: rail`): a 64px icon rail of the three groups left of a 196px sidebar showing only the active group's pages. Same data, two presentations.

### Command palette (⌘K)

Full-window scrim `rgba(3,6,7,.62)` + `blur(3px)`. Panel 620px, top-aligned 118px down, max 440px tall, `--tz-radius-lg`, `rgba(17,22,24,.96)`, `0 30px 90px -12px rgba(0,0,0,.8)`, entry `tzpop .16s cubic-bezier(.2,.8,.3,1)` (fade + `translateY(8px)` + `scale(.985)`).

Header: 15px 17px padding, accent magnifier, 14.5px borderless input, placeholder "Jump to a setting, theme, or keybind…". Rows: `9px 11px`, three columns — page name (11px mono faint, 78px fixed), label (13.5px text), current value (11px mono muted). First result and hover both `rgba(210,228,226,.14)`. Footer: `↑↓ navigate  ↵ open  esc close` (10.5px mono faint) on `--tz-surface`.

Searches label + page + value across 22 entries. Filter is a case-insensitive substring over the concatenation; cap at 7 results.

### Control patterns

Six patterns, used consistently. Rows are `display:flex; align-items:center; gap:18px; padding:13px 0` with a top hairline (`:first-of-type` has none) — **compact density drops padding to 9px**. Label block flexes; control is fixed-width right.

1. **Switch** — 40 × 23px, `--tz-radius` 12px, 2.5px padding, track `rgba(139,147,152,.22)` + hairline; on → accent fill and border. Knob 17px circle, muted → `--tz-on-accent`. cyber → 3px radius, 2px knob radius, `0 0 12px -2px` accent glow. `.16s` transition.
2. **Segmented** — 3px padding, `--tz-radius`, surface, hairline. Segments `5px 12px`, 7px radius, 12px/600; selected → accent fill + `--tz-on-accent`.
3. **Slider** — native `input[type=range]`, `accent-color: var(--tz-accent)`, 18px tall, flexing inside a 250px group with a 62px right-aligned mono readout.
4. **Text field** — `7px 11px`, `--tz-radius`, `rgba(11,14,15,.6)`, hairline, 12px mono.
5. **Theme card** — column, 9px gap, 11px padding, `--tz-radius`, `--tz-surface`, hairline. Contains a 28px swatch strip (flex 2/1/1/1 bars, 4px radius). Selected → accent border + `0 0 0 3px color-mix(accent 22%, transparent)`; cyber → `0 0 16px -4px accent`.
6. **Drag chip** — for reorderable module lists. `6px 9px`, 7px radius, surface, hairline, 11.5px mono, `cursor:grab`, `⠿` handle.

### Pages

All labels below are the **real strings from `pages.rs`**. 80 controls; each has a `tezca` CLI command and appears in the footer echo on change.

**Appearance** — Theme (5 cards: obsidian, quetzalcoatl, huitzilopochtli, xipe, smoke), Follow wallpaper switch. Wallpaper: 250px preview rendering the real file, fit-mode segmented (fill→`cover`, fit→`contain`, stretch→`100% 100%`, tile→repeat at 22%) that actually drives the preview, plus a 2-up picker of installed wallpapers (74px cards). Windows & motion: inner gaps, outer gaps, border size, corner rounding, active opacity, inactive opacity, blur switch, blur size, blur passes, shadows, animations.

**Bar** — Shape (floating/edge), height, top margin, side margin. Clock: strftime format field (`%a %d %b   %H:%M`). Workspaces: numerals (arabic/mayan), "Show only used workspaces", compact. Indicators: OSD switch, OSD timeout. Metrics: CPU/memory/GPU/network poll intervals, compact-below-width. Modules: three region lists (Left · 6, Center · 1, Right · 21) as drag chips with separators as hairlines; the AI chip is accent-highlighted; each region ends with a dashed `+ add`.

**Dock** — position, icon size, magnification, magnify radius, icon gap, bottom margin, autohide delay, running dots. Pinned favourites list.

**Displays** — see below.

**Sound** — output device, volume, mic level, mic boost, pop removal.

**Input** — keyboard layout, variant, options, key repeat rate, repeat delay, numlock. Pointer: acceleration profile, sensitivity, force no acceleration, focus follows mouse. Touchpad: natural scroll, tap to click, scroll speed. Cursor: size, hide while typing, software cursors.

**Network** — Wi-Fi, Bluetooth, airplane mode, VPN.

**Power** — profile, sleep after, dim on idle, dim after, lock after, lid sleep, caffeine.

**Startup** — Tezca services (bar, dock, hypridle, swaync) as switches.

**Keybinds** — table from `keybinds.rs`.

**Gaming** — game mode, disable animations, silence notifications, gamescope.

**System** — auto-update, session actions.

### Displays page

Rebuilt from `arrange.rs`. **Drag-to-arrange canvas**: monitors as proportional rectangles positioned by real `x,y` offsets, scaled to fit. Dragging snaps edges to neighbours within a threshold. Each rectangle shows name, resolution and refresh; the primary carries an accent border.

Below: per-monitor controls — resolution, refresh rate, scale, transform, VRR, position, primary toggle, enabled toggle. Then **layout profiles** (save/apply named arrangements) and **confirm-or-revert**: applying a mode shows a countdown toast that reverts automatically unless confirmed. `hyprctl` semantics.

### CLI echo footer

34px, `rgba(20,25,27,.62)`, top hairline. `CLI` label (9.5px/700 `letter-spacing:.12em` faint), then the command in accent 11.5px mono, ellipsised, then `✓ applied` (11px mono faint).

Every control writes its command here on change: `tezca bar set height 40`, `tezca theme set quetzalcoatl`, `tezca desktop set decoration:blur:passes 2`. **This is the teaching mechanism** — it shows the CLI equivalent of every GUI action. Command strings live in the `CMD` map in `TezcaSettings.dc.html`'s logic class; they are the real command surface, verified against `pages.rs`.

---

## Part 2 — `tezca-bar`

Source: `design_files/bar/`. `TezcaBar` (shell + desktop frame), `BarStrip` (the strip), `BarSurfaces` (popovers + OSD), `LlmPanel` (lateral AI panel), `BarGallery` (state documentation), `llm-data.js` (model/backend data).

### The strip

Height 40px (compact 32, roomy 48), `rgba(11,14,15,.70)`, 1px `rgba(63,184,175,.18)` border, 15px radius, `backdrop-filter:saturate(1.3) blur(20px)`, `0 8px 32px rgba(0,0,0,.45)`. Floating: 6px top / 10px side margins. **Edge shape**: radius 0, no border except a bottom hairline, opacity up to `.86`.

**Module** — `display:flex; gap:6px; padding:0 9px`, height `bar - 12px`, 9px radius, subtext. Hover `rgba(63,184,175,.09)` + text. States: `.on` accent, `.warn` gold, `.crit` urgent. **Separator** 1px × 16px, `rgba(139,147,152,.16)`, 5px side margins.

**Submap active** — the whole bar border turns gold with `inset 0 0 0 1px rgba(201,162,75,.14)`, and the strip shows `resize` + `hjkl · esc`.

### Regions

**Left** — Tezca menu button (20px logo), separator, focused app name (700 text) + per-app menu (File/Edit/View, hidden when compact), separator, workspaces.

**Workspaces** — 26px min-width pills, 9px radius, 2px margins, `.18s cubic-bezier(.25,.1,.25,1)`. Empty muted; occupied subtext; **active** accent fill + `--tz-on-accent` + 600 + accent glow; **urgent** urgent fill + white.

**Mayan numerals** — drawn as geometry, not the Unicode Mayan block, so they need no font. Dots are 3px circles; bars are 13 × 2.5px rounded rects; stacked in rows with 2px gaps. 1–4 = that many dots; 5 = one bar; 6 = one dot above one bar. Scales with the pill.

**Centre** — now-playing pill: `3px 13px 3px 5px`, 12px radius, `rgba(232,234,237,.055)` fill. 26px cover art (real MPRIS `artUrl`, gradient fallback), title (11px/600) over artist + elapsed (9.5px mono), then a 4-bar equaliser — 2px bars, accent, `tzeq 1.1s ease-in-out infinite` staggered 0/.18/.36/.54s.

**Right** — 20 slots expanded, 16 grouped:

privacy cluster · caffeine · night light · cloud AI · local AI · tray · SYS cluster · ⎸ · network · bluetooth · volume · brightness · battery · ⎸ · notifications · clock · power

### Enriched modules

**This is the substance of the bar redesign.** `popovers.rs` already computes far more than the strip showed. The actionable half moves up:

| Module | Was | Now |
| --- | --- | --- |
| CPU | `31%` | sparkline + `31%` + `62°` |
| Memory | `18.4G` | sparkline + `18.4G` + `/32` |
| GPU | `44%` | sparkline + `44%` + `71° 168W` |
| Network | glyph | SSID `Coatlicue-5G` over `↓12.4 ↑1.1 MB/s` |
| Bluetooth | glyph | `MX 82%` — connected device battery |
| Battery | `62%` | `62%` + `3h 40m` |
| Cloud AI | `68%` | `68%` + `4h 12m` until window reset |
| Local AI | — | accel badge + model size + live `tok/s` |
| Weather | — | `23°` + `18° / 27°` |
| Recording | dot | dot + `12:04` elapsed |
| Camera | glyph | glyph + owning app (`zoom`) |

Sparklines are 26 × 13px inline SVG polylines, 1.3px accent stroke.

**Local AI accel badge** — 9.5px mono, uppercase, `letter-spacing:.06em`. Shows the active backend: `CUDA`, `ROCm`, `MLX`, `Vulkan`, `CPU`. CPU-only renders **gold** as a warning; hardware backends render accent. Tooltip names the device. Backend list and per-backend device strings are in `llm-data.js`.

### Right-cluster strategies

Four, selectable — the user can also group/ungroup each cluster by clicking:

1. **Show all** — 20 slots. Honest; needs an ultrawide.
2. **Grouped** — privacy → one `3 capturing` pill; CPU/mem/GPU → one `SYS 31% · 18.4G · 44% 71°` readout. 20 → 16 slots. Chevron marks expandability; clicking expands in place; a collapse button at the end of an expanded run regroups it.
3. **Hover reveal** — idle modules drop to `opacity:.34` + `grayscale(.6)`; hovering the bar restores them. Nothing moves.
4. **Priority tiers** — tier 3 (tray, brightness, GPU, caffeine, night light) hides below `compact_width`.

Implement as style classes toggled from Rust, plus a `GtkRevealer` for the expand animation.

### Popovers

Common: `top: bar + 10px`, `rgba(11,14,15,.96)`, 1px accent-tinted border, 14px radius, 16px padding, `0 20px 50px rgba(0,0,0,.6)`, `blur(20px)`, `tzpop .14s`.

**Positioning must be measured, not hardcoded.** Each trigger carries a module key; on open the panel measures its trigger's offset within the strip and centres itself under it, clamped 8px from either edge. Hardcoded offsets desync the moment a module is added.

Internals: title 13px/600; metadata 11px mono muted; **meter** 5px tall, 3px radius, `rgba(139,147,152,.14)` track, accent fill; **chip** 10px mono accent on `rgba(63,184,175,.12)`, 5px radius; hairline dividers `rgba(139,147,152,.14)`.

| Popover | Contents |
| --- | --- |
| **CPU** | utilisation + temperature meters; clock, load average, processes, uptime; **16-cell per-core heat grid**; top 4 processes with PID and share |
| **Memory** | used + swap meters; available, cached, buffers, DIMM temp; top processes |
| **GPU** | utilisation, **VRAM**, temperature, power meters; VRAM figure, core/mem clock, fan; top processes |
| **Network** | 24-bar throughput chart with peak; signal, IPv4, gateway, DNS, band, link speed, session total; top talkers; other networks with security |
| **Bluetooth** | per-device battery meter + **20-bar link-quality history** (dropouts visible), codec, latency, RSSI; paired-not-connected; adapter |
| **Volume** | per-app mixer with source metadata; output device list with codec; **28-bar live input meter**; gain, peak, latency, quantum |
| **Battery** | 30px charge figure + remaining; **24-bar 24h discharge curve**; draw, health, cycles, capacity, temperature, profile; biggest consumers |
| **Cloud AI** | 5-hour + weekly window meters; **30-bar spend chart with the cap drawn as a red rule**; per-model token split; total, projected month, last poll |
| **Weather** | 34px temperature, conditions, feels-like, low/high; 5-hour strip; humidity, wind, UV, AQI, sunset |
| **Clock** | month calendar with today filled accent; agenda with coloured spines; four timezones; uptime |
| **Notifications** | per-app rows with inline actions, DND toggle, clear all, unread/today count |
| **Now playing** | 64px art, title/artist/album, progress, transport, shuffle/repeat, output sink, up-next queue, position, stream quality |
| **Mirror** | Tezca menu — theme name, Hyprland version, then Settings, Launcher, Theme picker, Reload bar, About, each with its keybind |
| **Power** | Lock, Log out, Suspend, Reboot, Power off with keybinds |

Charts are flex rows of 1.5–2px-gapped bars with heights set from data — cheap in GTK as a `GtkDrawingArea`.

### OSD

Centred, 38px from bottom, min 240px, `14px 20px`, 18px radius, `rgba(11,14,15,.86)`, `blur(18px)`, `tzosd .2s`. 22px accent speaker glyph, 8px track with an `accent-dim → accent` gradient fill and `0 0 10px` glow, then the value in 13px mono, 44px right-aligned. Auto-dismisses at 2.6s.

### Local AI panel

**A lateral dock, not a popover.** 400px, full height, right-anchored, `border-left` hairline, `tzslide .22s cubic-bezier(.2,.8,.3,1)` from `translateX(100%)`. Bound to **`SUPER I`**; the bar module toggles it; the tiled area reflows to make room. This is a real Hyprland window, not a layer surface.

**Header** — model selector (accent status dot, name 12.5px/600, `Q4_K_M · resident · 128k ctx` metadata, chevron), then icon buttons for conversations, settings, close.

**Model menu** — three sections: **Resident** (accent dot, VRAM each), **Available locally** (hollow dot), **Pulling** (progress meter, `11.2 / 18.4 GB · 24 MB/s · 5m left`). Model list and per-model metadata in `llm-data.js`.

**Settings drawer** — system prompt in a bordered block, temperature and top-p sliders side by side, then keep-alive / num_ctx / gpu-layers chips.

**Transcript** — 16px gap. User messages: text on `rgba(139,147,152,.09)`, 10px radius, `9px 11px`. Assistant: plain 12.5px/1.62 subtext. Role label 10px mono uppercase faint. **Code blocks**: 11px mono, `rgba(4,8,9,.55)`, hairline, 9px radius, with a copy button that appears on hover and confirms with `copied` for 1.4s. **Footer per message**: `312 tok · 7.4s · 42 tok/s` plus a regenerate button.

**Streaming** — 3–7 chars per 34ms tick with a blinking 7 × 13px accent caret (`tzcaret 1s step-end infinite`). A stop button replaces send; stopping keeps the partial with a `stopped` footer.

**Composer** — chips for `selection` (pulls the focused window's selection), `attach`, `screenshot`; attachment renders as a removable row. Textarea auto-grows to 120px; `⏎` sends, `⇧⏎` newlines. Send is a 28px accent square.

**Status bar** — live `tok/s` + a 12-bar sparkline, context use, VRAM meter, and the `SUPER I` reminder.

**Generation state must be lifted.** `tok/s` is owned by the parent so the bar module and the panel footer read the same value — both show `idle` when nothing generates, the live rate in accent while streaming. Panel-local state means the bar lies.

---

## Part 3 — Shell surfaces

Source: `design_files/shell/`.

### Dock

From `magnifier.rs` + `dock.toml`, verified against `docs/screenshots/desktop.jpg`.

Bottom-centred pill, `rgba(11,14,15,.70)`, `blur(20px)`, hairline border. Icons 48px, gap 10px, bottom margin 8px.

**Magnification physics — reproduce exactly.** Influence radius **110px**; max scale **1.6**; falloff is **cos²**: for pointer distance `d`, `scale = 1 + (max_scale - 1) * cos²(π · d / (2 · influence))` for `d < influence`, else 1. Hotspot is a **6px** band at the very bottom edge — the dock only magnifies when the pointer is within it. Labels appear above the icon past **1.15×**. **Not** CSS-transitioned; it tracks the pointer per frame.

**Item model** from `apps.rs`: pinned items in `dock.toml` order first, then running-unpinned behind a divider. Labels are the `.desktop` `Name=` verbatim. Running items carry a dot below.

### Launcher (walker)

From `config/walker/`, corrected against `docs/screenshots/launcher.jpg`.

**600px card** (`width-request`), 20px radius, 16px padding, 10px gap, `alpha(@tz_base,.85)`, accent-tinted border, two-layer shadow `0 24px 60px rgba(0,0,0,.55), 0 8px 24px rgba(0,0,0,.35)`.

**Search field is a filled pill inside the card** — 14px radius, `alpha(@tz_surface,.7)`, accent-tinted border, `14px 16px`, 19px text, accent caret. Not a header row with a divider.

**The list is flat.** `layout.xml` makes it a single-column `GtkGridView` — results from all providers interleave in one ranked run with **no group headers**. Rows: 12px radius, `10px 12px`, 2px vertical margin, 32px icon, name 15px, optional subtext 12px muted (only when the `.desktop` entry has one). **Selected rows get `inset 3px 0 0 0 @tz_accent`** — a left accent bar — plus `alpha(@tz_accent,.14)`. List caps at 400px (`max-content-height`).

`F1`–`F4` quick-activate render as accent chips (`alpha(@tz_accent,.20)`, 6px radius) on the first four rows only.

**Footer** is right-aligned keybind pairs — `open+next ⇧↵`, `pin ^+p`, `start ↵` — each a bordered label cap over its lowercase bind, above an accent-tinted rule. There is no result count.

Prefix providers from `config.toml`: `=` calc, `?` websearch, `/` files, `:` emoji. Calc answers render 24px/600 accent.

### Theme picker

Grid of the five shipped themes, each a card with a live swatch strip read from `themes/<name>/colors.css`, the active one accent-bordered.

### Notification toasts (swaync)

From `config/swaync/`.

400px, 14px radius, 12px padding, 10px gap, `alpha(@tz_base,.82)`, **accent-tinted** border, `0 6px 24px rgba(0,0,0,.45)`, `blur`. Critical is **border-only** `alpha(@tz_urgent,.55)` — no extra ring. Entry `tzslide .2s`.

48px icon, 10px radius. **`.summary` is the notification title** — 14px/600 text. The app name and timestamp share a muted 12px line beside it (`Signal · now`). Body 13px/1.45 subtext.

Actions are **all identical** — 13px, 12px radius, `alpha(@tz_accent,.06)` on `alpha(@tz_accent,.18)` border, hover to `.18`/`.40`. There is no solid-fill primary. Every toast carries a close button top-right, muted, hovering to an urgent fill.

Timeouts from `config.json`: normal 8s, low 4s, critical 0 (never auto-dismisses).

### Lock screen (hyprlock)

From `config/hypr/hyprlock.conf` + `themes/*/colors-hyprlock.conf`. **92px clock**, Tezca wordmark, **340 × 56px** input field. Blurred wallpaper behind.

### Session dialog (wlogout)

From `config/wlogout/style.css`. **Four tiles**, 20px radius; **shutdown is gold**. Keybinds shown per tile.

---

## Interactions & behaviour

**Settings**
- Sidebar click switches page and writes `tezca settings --page <id>` to the echo.
- `⌘K` / `Ctrl+K` opens the palette; `Esc` closes; typing filters; clicking a result navigates and closes.
- Every control writes its command to the echo footer on change.
- Displays: dragging a monitor snaps to neighbour edges; applying a mode starts a revert countdown.

**Bar**
- Clicking a module toggles its popover; a scrim closes it; opening another swaps directly.
- Clicking a grouped pill expands in place; the collapse button regroups.
- Local AI module toggles the lateral panel; `SUPER I` does the same globally.
- Volume change shows the OSD for 2.6s.

**AI panel**
- `⏎` sends, `⇧⏎` newlines; send disabled while streaming.
- Streaming appends with a blinking caret and auto-scrolls unless the user has scrolled up.
- Stop keeps the partial response. Regenerate truncates to that point and re-streams.
- Copy confirms inline for 1.4s.

**Dock** — pointer within the 6px hotspot magnifies per frame; labels past 1.15×; leaving resets.

---

## State management

**Settings** — `page`, `paletteOpen`, `query`, plus one flat map of all 80 control values (`v`) and `cmd` for the echo. Derived per render: selection maps, handler maps, formatted readouts. Real implementation reads from `config.toml` / `hyprctl` and writes via the `tezca` CLI.

**Bar** — `openPopover`, `activeWorkspace`, `osdVisible`, `submapActive`, `compact`, `groupPrivacy`, `groupSys`, `llmPanelOpen`, `llmBackend`, `llmModel`, `llmTps`. Note `llmTps` and backend/model live at bar level, shared with the panel.

**AI panel** — `model`, `modelsMenuOpen`, `sessionsOpen`, `settingsOpen`, `messages[]`, `draft`, `streaming`, `liveText`, `attachment`, `copiedIndex`. Generation rate reports upward.

Data sources: `/proc` and `sysfs` for CPU/mem; `amdgpu`/`nvidia-smi` for GPU; NetworkManager for network; BlueZ for bluetooth; PipeWire for audio; UPower for battery; MPRIS for now-playing; Ollama's HTTP API for local models; the Anthropic usage API for cloud AI; a weather provider for weather.

---

## Assets

- `wallpapers/obsidian-teal.jpg`, `wallpapers/smoke-light.jpg` — from the repo, used in the Appearance preview and shell backgrounds.
- `docs/screenshots/launcher.jpg`, `docs/screenshots/desktop.jpg` — reference renders of the current launcher and dock.
- All icons are **inline SVG, 1.7px stroke, 24px viewBox**, drawn in the prototypes — no icon font. Port to whatever the codebase uses; keep the stroke weight.
- Album art is a drop target in the prototype; the real source is MPRIS `mpris:artUrl`.
- Fonts: Inter and JetBrains Mono, both already used by Tezca.

---

## Proposals — not in Tezca today

Tagged in-prototype. Do not present as existing behaviour.

1. **Drag-to-rearrange bar modules in place** — writes `layout_*` back to `config.toml`, the same keys Settings edits.
2. **Unified quick-settings popover** — one panel for Wi-Fi, Bluetooth, volume, brightness, night light, caffeine, replacing six. Modules stay for at-a-glance state.
3. **Per-monitor bar layouts** — `layout_right.DP-2`, so an ultrawide and a vertical secondary differ. Today `compact_width` is the only lever.
4. **Sparkline history on hover** — expand the 60s inline sparkline to 10 minutes before opening the popover.
5. **Toast countdown hairline** — the 2px progress line on the Signal toast. swaync renders no timeout indicator.
6. **Weather module** — no weather source exists in the bar yet.
7. **Local AI module and panel** — no Ollama integration exists yet. The largest new capability here.

---

## Files

```
design_files/
  settings/
    TezcaSettings.dc.html   shell: window, nav, ⌘K palette, CLI echo, 80-control state map, CMD map
    PageLook.dc.html        Appearance, Bar, Dock
    PageDevices.dc.html     Sound, Input, Network, Power
    PageDisplays.dc.html    Displays — arrange canvas, profiles, confirm-or-revert
    PageSystem.dc.html      Startup, Keybinds, Gaming, System
  bar/
    TezcaBar.dc.html        shell: desktop frame, theme tokens, right-cluster strategies, lifted AI state
    BarStrip.dc.html        the strip — all modules, workspaces, Mayan numerals, clusters
    BarSurfaces.dc.html     all popovers + OSD, measured anchoring, chart data
    LlmPanel.dc.html        lateral AI panel — chat, model selector, settings, streaming
    BarGallery.dc.html      state gallery + cluster strategies + proposals, with rationale
    llm-data.js             backends, devices, model catalogue
  shell/
    TezcaShell.dc.html      shell frame + the three themes incl. light smoke
    ShellDock.dc.html       dock with cos² magnification
    ShellLauncher.dc.html   walker launcher
    ShellNotify.dc.html     toasts, three urgencies
    ShellLock.dc.html       hyprlock + wlogout
  tokens/
    styles.css              the token set, all themes
```

`github.md` at the project root maps every screen to the repo files it was built from — read it alongside this document.
