# START HERE — Claude Code brief

**Objective: 1:1 parity between the prototypes in `design_files/` and what runs on the user's machine.** Not "in the spirit of." The prototypes are the acceptance criteria, and the user will be comparing them side by side on a live Hyprland session.

There was a previous implementation attempt. It got a lot right and failed on fidelity. `GAP_AUDIT.md` says exactly where and why. **Read it before touching anything** — most of what you need is already built, and rewriting working code is the failure mode to avoid here.

---

## 1. The three documents

Read in this order:

| File | What it is | Use it for |
| --- | --- | --- |
| `README.md` | The design spec | Intent, structure, rationale, what each surface is for |
| **`GAP_AUDIT.md`** | Spec value → value in the working tree → fix | **The source of truth for every number.** Your task list. |
| This file | Rules of engagement and work order | How to proceed, what not to touch, when to stop and ask |

Where the spec and the audit disagree on a number, the audit wins — it was measured against the tree.

## 2. Look at the prototypes before you write code

The files in `design_files/` are **not production code to copy**. They are running HTML prototypes and they **open in a browser** — this bundle includes the runtime. Serve the bundle root and open them:

```bash
cd design_handoff_tezca_desktop
python3 -m http.server 8080
# then open, e.g.:
#   localhost:8080/design_files/bar/TezcaBar.dc.html
#   localhost:8080/design_files/settings/TezcaSettings.dc.html
#   localhost:8080/design_files/shell/TezcaShell.dc.html
```

They must be served over HTTP, not opened as `file://` (the runtime fetches siblings). Each one has live interactions — open popovers, switch pages, hit the palette, drag a monitor. **Parity means matching what you see there, including behaviour.** Reading the markup is not a substitute for running it; the markup carries the exact inline styles, but only the running page shows the rhythm you are matching.

Entry points:

- `bar/TezcaBar.dc.html` — the shell. Contains the desktop frame, theme tokens, the four right-cluster strategies, and the Tweaks that switch them. Start here, then `BarStrip`, `BarSurfaces` (popovers + OSD), `LlmPanel`, `BarGallery` (every state, documented, with rationale).
- `settings/TezcaSettings.dc.html` — the window, nav, ⌘K palette, CLI echo, and the 80-control state map. Pages in `PageLook` / `PageDevices` / `PageDisplays` / `PageSystem`.
- `shell/TezcaShell.dc.html` — frame + themes. Then `ShellDock`, `ShellLauncher`, `ShellNotify`, `ShellLock`.

## 3. The stack decision — already made, do not relitigate

| Surface | Stack | Why |
| --- | --- | --- |
| `tezca-settings` | **Migrate to Tauri** | 80 controls, a command palette, a drag canvas. Opened occasionally, so cold start and idle cost don't matter. Every remaining gap in it is a CSS-expressiveness gap. |
| **AI panel** | **Build in Tauri**, alongside settings | Chat, markdown, code blocks, streaming, copy button — all far easier in a webview. It's a real window, not a layer surface. It is also barely built, so nothing is thrown away. |
| `tezca-bar` | **Stays GTK4** | Runs forever on every output; a webview per monitor to draw a 40px strip is the wrong trade. What GTK provides here is `wlr-layer-shell` (exclusive zones, per-output anchoring), not layout. Already at ~90% fidelity. |
| Dock / launcher / toasts / lock / session | **Stays as-is — theming only** | `tezca-dock` is yours and correct. `walker`, `swaync`, `hyprlock`, `wlogout` are upstream daemons you theme with CSS, currently ~95%. Five one-line deltas total. |

Backend stays Rust throughout. `backend.rs` becomes Tauri commands with almost no change in shape.

**Consequence for your work order:** do **not** spend time converting `pt` to `px` in `config/tezca-settings/style.css` (audit §0.1). That stylesheet is going away. §1 of the audit becomes the **spec for the Tauri build** — the numbers there are what the new CSS should say.

## 4. Rules of engagement

1. **The audit's numbers are exact.** `13.5px`, `alpha(@tz_accent, .14)`, `inset 2px`. Do not round, do not "improve", do not substitute a value that looks close. Fidelity failures in the last attempt were death-by-a-thousand-roundings.
2. **No new hex values, anywhere.** Every colour exists as a `@tz_*` token in `themes/<name>/colors.css` (GTK/config) or a CSS custom property (Tauri). Derive with `alpha()` / `mix()` in GTK, `color-mix()` in CSS. A literal hex in a diff is a bug.
3. **Do not "fix" the four deliberate departures.** Audit §2.6 lists them with reasoning: no `box-shadow` on `.bar` (clipped on a layer surface), the MEM sparkline being gold, native `GtkPopover` anchoring instead of manual measurement, and no swaync countdown hairline. They are correct calls and they are commented in the source. Reverting them is a regression.
4. **`UNVERIFIED` in the audit means check it at runtime, not rewrite it.** Roughly 15 items. Run the thing, confirm the value, tick it off. Most will pass.
5. **`PASS` means leave it alone.** The launcher, the dock physics, the popover inventory, the arrange canvas, the keybind table, the page/control/CLI-command inventory — all verified correct. Do not refactor them while you're nearby.
6. **Prefer editing to rebuilding.** Every mechanism the design needs already exists somewhere in the tree. The bar needs ~20 one-line changes, not a new bar.
7. **All telemetry in the prototypes is sample data.** Percentages, temperatures, SSIDs, token counts, battery figures. Wire to the real sources (listed in `README.md` ▸ *State management*). Never ship a hardcoded `31%`.
8. **Proposals are not features.** `README.md` ▸ *Proposals* lists seven things that don't exist in Tezca today and are tagged `PROPOSED` in the prototypes. Don't present them as existing. Build them only if the user asks.

## 5. Work order

Each phase is independently shippable and independently checkable. Don't start the next until the user has seen the current one running.

**Phase 1 — bar numeric parity.** Audit §2. About 20 one-line edits in `crates/tezca-bar/src/bar.css`, plus the module box `spacing: 6` in `bar.rs` (§2.2, currently unverified) and `.pop-big.tz-xl` for the weather popover. Also clear the §2 `UNVERIFIED` rows: sparkline constants, Mayan dot/bar geometry, equaliser period and stagger, chart bar counts, OSD 2.6s dismiss and 38px bottom margin, occupied-workspace colour, MPRIS `artUrl` actually loading.
*Done when:* `BarStrip` and `BarSurfaces` and the running bar are indistinguishable at every state in `BarGallery`.

**Phase 2 — shell deltas.** Audit §4. Five changes: dock pill `0.72` → `0.70`, swaync action `font-size: 13px`, swaync body line spacing `1.45`, the wlogout shutdown tile gold at rest, and confirm the 48px toast icon.
*Done when:* `ShellDock` / `ShellLauncher` / `ShellNotify` / `ShellLock` match live.

**Phase 3 — `tezca-settings` in Tauri.** The big one. Build to audit §1 (window, omnibox, sidebar, palette, echo footer, control patterns, pages) with §0.1–0.3 as the type scale, row pattern and section rhythm. Port `backend.rs` to Tauri commands; keep the CLI-echo mechanism exactly as it is — it's the teaching device and the `CMD` map is already verified against `pages.rs`.
Carry over from the current GTK build rather than re-deriving: the page and control inventory, the 80 CLI command strings, the palette's 22 index entries, the arrange-canvas snapping logic (`arrange.rs:458`, `:489`) and confirm-or-revert, and the keybind table.
*Done when:* every page matches its prototype, ⌘K behaves, the echo footer prints on every change, and Displays drags and snaps.

**Phase 4 — the AI panel in Tauri.** Real construction; audit §3 is the inventory of what's missing. Two things to resolve **before** you start, in §7 below.
*Done when:* `LlmPanel` and the running panel match, including streaming, the caret, code-block copy, the per-message footer, regenerate, the model menu's three sections, the settings drawer and the status bar.

**Phase 5 — remaining drift and the absent extras.** Palette header restructure (§1.4), then the rail alternate shell, compact density, the grouped-expand `GtkRevealer`, the AI-chip highlight in the module editor.

## 6. How to verify parity

Per surface, before calling it done:

1. Prototype open in a browser at the same scale, real thing running beside it.
2. Walk every state the prototype exposes. `BarGallery` documents them for the bar; the Tweaks panels expose the strategy and density variants.
3. Check the four things roundings hide: **type scale** (is the ratio between title and label right, not just each size), **hairlines** (present, and at the right alpha), **row rhythm** (padding and separator, not just content), **hover and selected states** (both, on every interactive element).
4. Switch themes — obsidian, quetzalcoatl, huitzilopochtli, xipe, and the light **smoke**. A hardcoded value shows up immediately as the one thing that didn't change. Smoke is the useful one: it inverts, so anything assuming a dark background breaks visibly.
5. Confirm nothing is hardcoded sample data.

## 7. Decide with the user, don't decide alone

Three open questions. Each changes what you build; none should be guessed.

1. **The AI panel's window type.** The spec says a real Hyprland window so the tiled area reflows when it opens. The current code (`llmpanel.rs:146`) makes it a layer surface with `set_exclusive_zone(WIDTH)`. Moving it to Tauri makes "real window" the natural answer — confirm, and if it stays a layer surface, amend the spec.
2. **Shared generation rate.** The spec requires `tok/s` to live at bar level so the bar module and the panel footer read the same number. `llmpanel.rs:71` notes the panel is a **separate process** from the bar, so in-process sharing is impossible as built. Either the panel pushes the rate to the bar over IPC, or the bar polls Ollama itself and the two can briefly disagree. Pick one deliberately.
3. **The local-AI badge's gold condition.** Spec golds the accel badge when the backend is **CPU-only**. The code golds it on **split offload**. Both are real warnings; the honest answer is probably both, with distinct tooltips. Confirm.

## 8. Bundle contents

```
START_HERE.md              this file
README.md                  the design spec — intent and structure
GAP_AUDIT.md               spec vs. code, per surface — your task list
styles.css                 design tokens (all themes) — loaded by the prototypes
_ds_bundle.js              prototype runtime — do not port, do not read for style
design_files/
  bar/       TezcaBar · BarStrip · BarSurfaces · LlmPanel · BarGallery · llm-data.js
  settings/  TezcaSettings · PageLook · PageDevices · PageDisplays · PageSystem
  shell/     TezcaShell · ShellDock · ShellLauncher · ShellNotify · ShellLock
  tokens/    styles.css — the token set as a readable reference
assets/
  obsidian-teal.jpg        repo wallpaper — Appearance preview, shell backgrounds
  smoke-light.jpg          repo wallpaper — the light theme
  reference-desktop.jpg    current dock, for comparison
  reference-launcher.jpg   current launcher, for comparison
github.md                  screen → repo file map
```

`support.js` / `ds-base.js` / `image-slot.js` inside each `design_files/` folder are prototype plumbing. They make the pages open in a browser. They are not part of the design and nothing in them should be ported.

Fonts are Inter and JetBrains Mono, both already used by Tezca. Icons in the prototypes are inline SVG, 1.7px stroke, 24px viewBox — port to whatever the target uses, but keep the stroke weight; GTK symbolic icons won't match exactly and that substitution is accepted (audit §1.3).
