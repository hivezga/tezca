# Gap audit — design spec vs. current implementation

Companion to `README.md` (the spec). Every row is **spec value → value found in the code → what to change**. Values were read out of the working tree; selectors are quoted verbatim so they can be grepped rather than guessed at.

Read this as the source of truth for *numbers*. `README.md` remains the source of truth for *intent*.

**Verdict column:** `PASS` = matches, leave alone · `DRIFT` = right shape, wrong number · `WRONG` = wrong pattern, needs restructuring · `ABSENT` = not implemented.

**Overall state**

| Surface | Assessment |
| --- | --- |
| Shell (dock, launcher, toasts, lock, session) | ~95% — essentially built to spec. Five small deltas. |
| `tezca-bar` strip, popovers, OSD | ~90% — structurally complete, ~20 numeric deviations. |
| `tezca-settings` | ~50% — every mechanism exists, but the **type scale and row pattern are wrong**, which is why it reads as a different design. |
| AI panel (`llmpanel.rs`) | ~15% — a stub. The largest gap in the project. |

---

## 0. Five systemic findings

Fix these first. Most of the individual rows below are downstream of them.

### 0.1 `tezca-settings` sizes everything in `pt`; the spec is in `px` — **WRONG**

`config/tezca-settings/style.css` expresses every font size in `pt`. GTK resolves `pt` against display DPI; at 96 dpi `1pt = 1.333px`. The spec's px values were therefore never actually applied — each was replaced by whatever the nearest half-point happened to be, and the errors do not go the same direction:

| Spec role | Spec | Code | Renders as | Error |
| --- | --- | --- | --- | --- |
| Page title | **23px** / 700 | `.tz-h2` 12.5pt | 16.7px | **−27%** |
| Section label | **10.5px** / 700 uppercase | *(no rule)* | — | absent |
| Control label | **13.5px** / 600 | `.tz-ctlrow > label` 10.5pt | 14px | +4% |
| Control hint | **12px** / 400 | `.tz-hint` 9.5pt | 12.7px | +6% |
| Switch-row label | 13.5px | `.tz-switchrow label` 11pt | 14.7px | +9% |
| Palette input | **14.5px** | `.tz-palette-entry` 12pt | 16px | +10% |
| Palette label | 13.5px | `.tz-palette-label` 10.5pt | 14px | +4% |
| Omnibox placeholder | **12.5px** | `.tz-omni label` 10pt | 13.3px | +6% |
| CLI command | **11.5px** | `.tz-echo-cmd` 9pt | 12px | +4% |

The net effect: **the title shrinks 27% while body text grows 4–10%**, so the 1.7× ratio between page title and control label collapses to 1.19×. Nothing in the page has a hierarchy. This single issue accounts for most of "looks nothing like the prototypes."

**Fix:** convert every `font-size` in `config/tezca-settings/style.css` to `px` and use the spec numbers. GTK4 accepts `px` in CSS. Do not scale by ratio — take the values from §1 below. `bar.css` already does this correctly (it is px throughout); settings is the outlier.

### 0.2 The settings row pattern is not the spec's row pattern — **WRONG**

Spec (README ▸ *Control patterns*): a row is `flex; align-items:center; gap:18px; padding:13px 0` with a **top hairline**, `:first-of-type` excepted. Rows sit directly on the page background. The label block is a 13.5px/600 label over a 12px/400 muted hint.

Code has **two** competing idioms and neither is that one:

- `.tz-ctlrow` (`pages.rs:4552`, the universal helper) — `padding: 6px 2px; min-height: 34px`, **no border at all**. Rows run together with no separation.
- `.tz-switchrow` (`pages.rs:3173`, `4266`) — `background alpha(@tz_surface,.55); border-radius:12px; padding:12px 16px; margin:4px 0`. A filled card per row. This is a different visual language from everything else in the panel and appears on Startup and Gaming only.

**Fix:** one row helper, matching spec. Add to `style.css`:

```css
.tz-ctlrow {
    border-top: 1px solid alpha(@tz_muted, 0.16);
    padding: 13px 0;
    min-height: 0;
}
.tz-ctlrow.tz-first { border-top: none; }
.tz-ctlrow.tz-compact { padding: 9px 0; }
.tz-ctlrow > label { font-size: 13.5px; font-weight: 600; color: @tz_text; }
.tz-ctlrow .tz-hint { font-size: 12px; font-weight: 400; color: @tz_muted; }
```

GTK has no `:first-of-type` that works across a `GtkBox`, so `pages.rs` must add `tz-first` to the first row it emits per section. Retire `.tz-switchrow` and route Startup/Gaming through `.tz-ctlrow`. The 18px label↔control gap becomes the row `GtkBox`'s `spacing` property.

### 0.3 The page's section rhythm is missing — **ABSENT**

The spec's page structure is: **23px page title**, then per section a **10.5px/700 uppercase `letter-spacing:.14em` faint label**, then hairline-separated rows. In the code `.tz-h2` (12.5pt) is doing duty as both page title *and* section heading (`pages.rs:690`, `2573`, `3798`, `4521`), and the uppercase section label does not exist.

**Fix:** split into two classes.

```css
.tz-pagetitle { font-size: 23px; font-weight: 700; color: @tz_text; margin-bottom: 2px; }
.tz-seclabel {
    font-size: 10.5px; font-weight: 700; letter-spacing: 1.47px;
    text-transform: uppercase; color: @tz_faint;
    margin-top: 22px; margin-bottom: 7px;
}
```

Then in `pages.rs`, the `tz-h2` helper at 4521 becomes `tz-seclabel`, and each page's first heading becomes `tz-pagetitle`.

### 0.4 The AI panel is a stub — **ABSENT**

`crates/tezca-bar/src/llmpanel.rs` builds: header (status dot, model label, close button), a message list, a composer with a send button, a status line. That is all of it. `append_bubble()` (line 355) emits a `GtkLabel` per turn.

Everything else the spec calls for is missing. Inventory in §3.

### 0.5 One popover class is carrying two different sizes — **DRIFT**

`.pop-big` in `bar.css` is fixed at `30px`. The spec asks for **30px** on Battery (charge figure) and **34px** on Weather (temperature).

**Fix:** add `.pop-big.tz-xl { font-size: 34px; }` and apply it in the weather popover.

---

## 1. `tezca-settings`

### 1.1 Window & title bar

| Item | Spec | Code | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Window size | 1180 × 788 | `main.rs:123–124` — 1180 × 788 | PASS | — |
| Window radius | `--tz-radius-lg` 14px | *(not set)* | ABSENT | `window.tezca-settings { border-radius: 14px }` |
| Window border | 1px `--tz-line-strong` (.24) | *(not set)* | ABSENT | `border: 1px solid alpha(@tz_muted, .24)` |
| Window fill | `--tz-base` | `alpha(@tz_base, .86)` | DRIFT | Intentional glass — keep if the compositor blurs it, else `@tz_base`. |
| Title bar height | **52px** | `headerbar.tz-header` `min-height: 44px` | DRIFT | → `52px` |
| Title bar fill | `rgba(20,25,27,.62)` | `alpha(@tz_surface, .55)` | DRIFT | → `.62` |
| Title bar hairline | `--tz-line` (.16) | `alpha(@tz_muted, .18)` | DRIFT | → `.16` |
| Brand `TEZCA` | 14px / 800 / `.14em` (1.96px) | `.tz-brand` 11pt (14.7px), ls 1.8px | DRIFT | → `14px`, ls `1.96px` |
| Sub `SETTINGS` | **11.5px** / 600 uppercase muted | `.tz-subtitle` 9pt (12px), ls 1.2px | DRIFT | → `11.5px`, ls `1.61px` |
| Logo mark | 20px concentric circles, accent | *(not found)* | ABSENT | Cairo `DrawingArea`, same approach as the bar's mirror glyph in `draw.rs`. |
| Window dots | three 12px dots, close `--tz-urgent` | *(not found)* | ABSENT | Optional if using the real titlebar; drop from spec if so. |

### 1.2 Search omnibox

| Item | Spec | Code (`button.tz-omni`) | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Max width | **520px** | *(unconstrained)* | ABSENT | `.set_size_request` / `set_max_width_chars` |
| Height | **32px** | `min-height: 30px` | DRIFT | → `32px` |
| Radius | 10px | `10px` | PASS | — |
| Fill | `rgba(11,14,15,.6)` | `alpha(@tz_base, .55)` | DRIFT | → `.6` |
| Border | 1px `--tz-line` (.16) | `alpha(@tz_muted, .18)` | DRIFT | → `.16` |
| Placeholder | 12.5px muted | `10pt` = 13.3px | DRIFT | → `12.5px` |
| `⌘K` cap | 10.5px mono, r5, hairline | `8pt` = 10.7px, r5, hairline | DRIFT | → `10.5px` |

### 1.3 Sidebar

| Item | Spec | Code | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Width | 224px | `main.rs:193` — 224 | PASS | — |
| Fill | `rgba(20,25,27,.34)` | `.tz-sidebar` `alpha(@tz_surface,.30)` | DRIFT | → `.34` |
| Right hairline | 1px `--tz-line` | *(not set)* | ABSENT | `border-right: 1px solid alpha(@tz_muted,.16)` |
| Row padding | **8px 11px** | `10px 12px` | DRIFT | → `8px 11px` |
| Row font | **13px** / 600 | *(inherited)* / 600 | DRIFT | add `font-size: 13px` |
| Row radius | 10px | `10px` | PASS | — |
| Icon | 17px stroked, 1.7 width | `.tz-navicon` `-gtk-icon-size: 18px` | DRIFT | → `17px`. Symbolic icons won't match the prototype's 1.7px stroke; acceptable substitution. |
| Icon↔label gap | 11px | *(box spacing, unverified)* | — | set row box `spacing` to 11 |
| Hover fill | `rgba(139,147,152,.12)` | `alpha(@tz_muted,.14)` | DRIFT | → `.12` |
| Selected fill | `rgba(210,228,226,.14)` | `alpha(@tz_accent,.18)` | DRIFT | → `.14` |
| Selected marker | **`inset 2px`** accent | `inset 3px` | DRIFT | → `2px`. Note the launcher's marker *is* 3px per spec — they differ deliberately. |
| Group header | 10px/600 uppercase `.13em` (1.3px) faint, `14px 12px 7px` | `.tz-navgroup` 8pt (10.7px)/**700**, ls 1.4px, `14px 12px 6px` | DRIFT | → `10px`, `font-weight: 600`, ls `1.3px`, padding-bottom `7px` |
| Groups & membership | Look & feel / Devices / System | `main.rs:56–68` — exact match | PASS | — |
| Footer | **card**: radius, `@tz_surface` fill, hairline border | `.tz-session` — `border-top` only, no card | WRONG | Wrap in a box with `background:@tz_surface; border:1px solid alpha(@tz_muted,.16); border-radius:10px; margin:8px; padding:10px 11px`. |
| Footer dot | 6px accent + `0 0 8px` glow | 6px accent, no glow | DRIFT | add `box-shadow: 0 0 8px alpha(@tz_accent,.7)` |
| Footer host | 11px / 600 | `9.5pt` = 12.7px | DRIFT | → `11px` |
| Footer meta | 10.5px mono faint | `8.5pt` = 11.3px | DRIFT | → `10.5px` |
| Rail shell (`shell: rail`) | 64px icon rail + 196px sidebar | *(not implemented)* | ABSENT | Second presentation; ship after the primary is correct. |

### 1.4 Command palette

Structurally the largest divergence after the row pattern. The code builds a **bordered entry above a list**; the spec is a **header row** (accent magnifier + borderless input) above the list.

| Item | Spec | Code (`palette.rs` / `.tz-palette*`) | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Panel width | 620px | `palette.rs:144` — 620 | PASS | — |
| Top offset | 118px from top | *(centred)* | DRIFT | `Align::Start` + `margin_top: 118` |
| Max height | 440px | *(unconstrained)* | ABSENT | `set_max_content_height(440)` on the scroller |
| Radius | 14px | `14px` | PASS | — |
| Fill | `rgba(17,22,24,.96)` | `alpha(@tz_surface,.98)` | DRIFT | → `.96` |
| Border | 1px `--tz-line-strong` (.24) | `alpha(@tz_muted,.26)` | DRIFT | → `.24` |
| Shadow | `0 30px 90px -12px rgba(0,0,0,.8)` | *(none)* | ABSENT | add |
| Scrim | `rgba(3,6,7,.62)` + `blur(3px)` | `alpha(@tz_base,.62)` | PASS | blur is compositor-side; fine |
| Header | `15px 17px` padding, accent magnifier glyph, **borderless** 14.5px input | `.tz-palette-entry` — `border-bottom` hairline, 12pt, `10px 8px`, no icon | WRONG | Rebuild as an hbox: 15px 17px padding, accent icon, `border: none` entry at 14.5px. Drop the `border-bottom`. |
| Row padding | **9px 11px** | `8px 10px` | DRIFT | → `9px 11px` |
| Row radius | *(unspecified)* | `9px` | PASS | — |
| Page column | 11px mono faint, **78px fixed** | `8.5pt` = 11.3px, `set_width_chars(11)` | DRIFT | → `11px`; `set_size_request(78,-1)` is more faithful than char count |
| Label column | 13.5px text | `10.5pt` = 14px | DRIFT | → `13.5px` |
| Value column | 11px mono muted | `8.5pt` = 11.3px muted | DRIFT | → `11px` |
| Selected/hover fill | `rgba(210,228,226,.14)` | `alpha(@tz_accent,.16)` | DRIFT | → `.14` |
| First result pre-selected | yes | *(verify)* | — | must be selected on open with an empty query |
| Footer | 10.5px mono faint on **`@tz_surface` fill** | `.tz-palette-foot` — hairline only, no fill; tip 8pt | DRIFT | add `background-color: @tz_surface`; tip → `10.5px` |
| Result cap | 7 | `palette.rs:21` — documented cap | PASS | confirm the constant is 7 |
| Entry animation | `tzpop .16s` (fade + 8px rise + `scale(.985)`) | *(none)* | ABSENT | GTK4 can do the fade+rise with a `GtkRevealer`; `scale()` is not animatable — drop it. |

### 1.5 CLI echo footer

| Item | Spec | Code (`.tz-echo*`) | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Height | **34px** | `min-height: 28px` | DRIFT | → `34px` |
| Fill | `rgba(20,25,27,.62)` | `alpha(@tz_surface,.55)` | DRIFT | → `.62` |
| Top hairline | `--tz-line` (.16) | `.16` | PASS | — |
| Padding | *(implied)* | `6px 16px` | PASS | — |
| `CLI` tag | 9.5px / 700 / `.12em` (1.14px) faint | `7.5pt` = 10px, ls 1.2px | DRIFT | → `9.5px`, ls `1.14px` |
| Command | 11.5px mono accent, ellipsised | `9pt` = 12px mono accent | DRIFT | → `11.5px`; confirm `set_ellipsize(End)` |
| State | 11px mono faint | `8.5pt` = 11.3px | DRIFT | → `11px` |
| Semantics | every control writes on change | `backend.rs:40` + `.ok/.err/.busy/.sent` states | PASS | richer than spec — keep |

### 1.6 Control patterns

Beyond §0.2, per-pattern:

| Pattern | Spec | Code | Verdict | Fix |
| --- | --- | --- | --- | --- |
| **Switch** | 40 × 23px, r12, 2.5px pad, track `rgba(139,147,152,.22)` + hairline, knob 17px | `switch { min-width: 46px }`, `:checked` accent | DRIFT | → `min-width: 40px; min-height: 23px; border-radius: 12px; padding: 2.5px; background: alpha(@tz_muted,.22); border: 1px solid alpha(@tz_muted,.16)`; `slider { min-width:17px; min-height:17px }`; add `transition: 160ms` |
| **Segmented** | 3px pad, r10, segment `5px 12px` r7 12px/600 | `.tz-seg` 3px/r10 ✓; segment `4px 12px`, 9.5pt (12.7px) | DRIFT | segment → `padding: 5px 12px; font-size: 12px` |
| **Slider** | 18px tall, 250px group, **62px right-aligned mono readout** | `scale { min-width: 200px }`, `scale value` 9pt | DRIFT | group → 250px; replace `scale value` with a real 62px mono label (`font-size:12px; font-family:"JetBrains Mono"; xalign:1`) — GTK's built-in value is not mono and not tabular |
| Slider trough | *(spec: native, accent-color)* | 6px, r6, `alpha(@tz_muted,.20)` | PASS | — |
| **Text field** | `7px 11px`, r10, `rgba(11,14,15,.6)`, hairline, **12px mono** | `entry.tz-search` `8px 12px`, `alpha(@tz_surface,.80)`, sans | DRIFT | → `padding: 7px 11px; background: alpha(@tz_base,.6); font-family:"JetBrains Mono"; font-size:12px` |
| **Theme card** | column, 9px gap, 11px pad, r10, surface, hairline; 28px swatch strip (flex 2/1/1/1, r4); selected accent border + `0 0 0 3px` @22% | `button.tz-theme` — 11px pad, r10, `@tz_surface`, `.16` hairline, active border + `0 0 0 3px` `.22` | PASS | verify the cairo swatch strip is 28px tall with 2/1/1/1 weights and r4 |
| **Drag chip** | `6px 9px`, r7, surface, hairline, 11.5px mono, `⠿` handle, `cursor:grab` | `.tz-modrow` `2px 2px`, 10pt, `.tz-modgrip` present | WRONG | The chip has no chrome at all — it is a bare label row. Give `.tz-modrow` `background:@tz_surface; border:1px solid alpha(@tz_muted,.16); border-radius:7px; padding:6px 9px; font-family:"JetBrains Mono"; font-size:11.5px` |
| Region separators | rendered as hairlines | *(verify)* | — | separators in a region list must draw as a 1px rule, not the word "separator" |
| AI chip highlight | accent-highlighted | *(not found)* | ABSENT | add `.tz-modrow.tz-ai { color: @tz_accent; border-color: alpha(@tz_accent,.35) }` |
| `+ add` affordance | dashed | `.tz-moddrop` dashed r8 | PASS | — |

### 1.7 Pages

Page inventory, control labels and CLI commands were verified against `pages.rs` and match the spec — this part was done well. Remaining page-level items:

| Item | Spec | Code | Verdict |
| --- | --- | --- | --- |
| Wallpaper preview | 250px, fit-mode segmented drives the preview | `pages.rs:90`, `1385` + `.tz-wallpreview` | PASS — confirm the fit mode actually re-renders |
| Wallpaper cards | 74px, 2-up | `button.tz-wallcard` `min-height: 74px` | PASS |
| Preview radius | *(card 14px)* | `12px` | DRIFT → `14px` |
| Displays canvas | drag + edge snap, profiles, confirm-or-revert | `arrange.rs` — snapping at `:458`, `:489`; `Palette::of` repaints on theme change | PASS |
| Arrange well radius | *(card 14px)* | `.tz-arrange` `10px` | DRIFT → `14px` |
| Keybinds table | from `keybinds.rs` | `.tz-keylist` + `keybinds.rs` | PASS |
| Compact density | rows drop to 9px padding | *(not implemented)* | ABSENT — add `.tz-compact` per §0.2 and a toggle |

---

## 2. `tezca-bar`

`bar.css` is px-based, token-only, and structurally faithful. What follows is almost entirely small numbers. The file's inline comments document several deliberate departures — **those are correct and should not be "fixed"**; they are listed under §2.6 so nobody re-opens them.

### 2.1 The strip

| Item | Spec | Code (`.bar`) | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Height | 40 (compact 32 / roomy 48) | `bar.rs:867` from `cfg.height` | PASS | — |
| Fill | `rgba(11,14,15,.70)` | `alpha(@tz_base,.70)` | PASS | — |
| Border | 1px `rgba(63,184,175,.18)` | `alpha(@tz_accent,.18)` | PASS | — |
| Radius | 15px | `15px` | PASS | — |
| Margins | 6px top / 10px side | `bar.rs:1370–1372` from config | PASS | confirm `config.toml` defaults are 6 / 10 |
| Exclusive zone | height + top margin | `bar.rs:1373` | PASS | — |
| **Edge** fill | up to `.86` | `.bar.edge` `alpha(@tz_base,.82)` | DRIFT | → `.86` |
| Edge border | bottom hairline only, r0 | matches | PASS | — |
| Submap inset ring | `rgba(201,162,75,.14)` | `alpha(@tz_gold,.12)` | DRIFT | → `.14` |
| Submap hint | `resize` + `hjkl · esc` | `.submap-label` + `.submap-hint` | PASS | — |

### 2.2 Modules

| Item | Spec | Code | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Padding | `0 9px` | `0 9px` | PASS | — |
| Height | bar − 12 (= 28) | `min-height: 28px` | PASS | — |
| Radius | 9px | `9px` | PASS | — |
| Internal gap | **6px** | *(box spacing — not found in `bar.rs`)* | UNVERIFIED | every module's `GtkBox` needs `spacing: 6` |
| Hover | `rgba(63,184,175,.09)` | `alpha(@tz_accent,.09)` | PASS | — |
| State colours | accent / gold / urgent | `.on` `.warn` `.crit` | PASS | — |
| Separator | 1px × 16px, `rgba(139,147,152,.16)`, 5px margins | `.sep` exact | PASS | — |
| Value type | 12px/500 mono `tnum` | `.metric-val` etc. | PASS | — |
| Sub-value | 9.5px/500 mono faint | `.metric-sub` 9.5px faint | PASS | — |
| Enriched readouts | CPU temp, GPU watts, batt time, BT battery, AI reset, SSID over throughput | `.metric-sub`, `.control-name`, `.control-sub` all present | PASS | — |
| Sparklines | 26 × 13px, 1.3px accent stroke | `draw.rs:75–137`, three instances (`bar.rs:1129–1142`) | PASS | verify the size/stroke constants are 26 / 13 / 1.3; CPU=accent, MEM=**gold**, GPU=accent-dim — spec implies accent for all three, the code's differentiation is better, keep it |
| Local-AI accel badge | 9.5px mono uppercase `.06em`; **gold when CPU-only** | `.llm.warn` → gold | DRIFT | the code golds on *split offload*, spec golds on *CPU-only*. Both are defensible; make it gold for either and document it. No dedicated badge class — add `.llm-accel { font-size:9.5px; letter-spacing:.57px; text-transform:uppercase }` |

### 2.3 Workspaces

| Item | Spec | Code (`button.ws`) | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Min width | 26px | `26px` | PASS | — |
| Radius | 9px | `9px` | PASS | — |
| Margin | 2px | `0 2px` | PASS | — |
| Transition | `.18s cubic-bezier(.25,.1,.25,1)` | exact | PASS | — |
| Empty / occupied | muted / subtext | `@tz_muted` / *(via `.occupied`?)* | UNVERIFIED | confirm an occupied-but-inactive pill gets `@tz_subtext` |
| Active | accent fill, on-accent, 600, glow | exact | PASS | — |
| **Urgent** | urgent fill + **white** | `color: @tz_on_accent` (dark) | DRIFT | → `#FFFFFF` (or `@tz_on_urgent`, which the settings fallback tokens define) |
| Mayan numerals | drawn geometry: 3px dots, 13 × 2.5px bars, 2px row gaps | `draw.rs:192–229`, `BAR_W: 13.0`, `MAYAN_MAX: 19` | PASS | verify dot radius 3 and bar height 2.5 |

### 2.4 Now-playing pill

| Item | Spec | Code | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Padding / radius / fill | `3px 13px 3px 5px`, r12, `rgba(232,234,237,.055)` | exact | PASS | — |
| Cover art | **26px** | `.np-art` `min-width/height: 24px` | DRIFT | → `26px` |
| Art radius | *(unspecified)* | 7px | PASS | — |
| Title | 11px / 600 | exact | PASS | — |
| Artist + elapsed | **9.5px** mono | `.np-artist` `10px` | DRIFT | → `9.5px` |
| Equaliser | 4 bars, 2px wide, accent, `1.1s` staggered 0/.18/.36/.54 | `draw.rs:152` frame-clock animated | PASS | verify bar width 2px, period 1.1s, stagger values |
| Real `artUrl` | MPRIS art with gradient fallback | gradient fallback present | UNVERIFIED | confirm `artUrl` is actually loaded, not only the fallback |

### 2.5 Popovers & OSD

| Item | Spec | Code | Verdict | Fix |
| --- | --- | --- | --- | --- |
| Fill / border / radius / padding | `rgba(11,14,15,.96)`, accent-tinted, 14px, 16px | `popover.tz-popover > contents` exact | PASS | — |
| Shadow | `0 20px 50px rgba(0,0,0,.6)` | `alpha(black,.55)` | DRIFT | → `.6` |
| Positioning | measured per trigger, clamped 8px | `GtkPopover` native anchoring | PASS | native is better than measuring — keep |
| Title | 13px / 600 | `.pop-title` exact | PASS | — |
| Metadata | 11px mono muted | `.pop-sub` / `.pop-mono` exact | PASS | — |
| Meter | 5px tall, r3, `rgba(139,147,152,.14)` track, accent fill | `levelbar.mix` — 5px, r3, track `alpha(@tz_text,.08)` | DRIFT | track → `alpha(@tz_muted,.14)` |
| Chip | 10px mono accent on `rgba(63,184,175,.12)`, r5 | `.chip-connected` exact | PASS | — |
| Dividers | `rgba(139,147,152,.14)` | `.pop-sep` `alpha(@tz_text,.10)` | DRIFT | → `alpha(@tz_muted,.14)` |
| Big figure | 30px battery / **34px weather** | `.pop-big` 30px only | DRIFT | see §0.5 |
| Per-core heat grid | 16 cells | `.core-grid` + `levelbar.core` | PASS | spec suggested a `DrawingArea`; 16 levelbars is fine at this count |
| Bar charts | net 24 · BT 20 · volume 28 · battery 24 · spend 30 | `levelbar.core` reused, `net-up` variant | UNVERIFIED | confirm each chart's bar count; the **spend chart's red cap rule** is not in the CSS — add `.spend-cap { background: @tz_urgent; min-height: 1px }` |
| Popover coverage | 15 popovers | `popovers.rs` | PASS | inventory reads complete |
| OSD fill | `rgba(11,14,15,.86)` | `.osd` `alpha(@tz_base,.82)` | DRIFT | → `.86` |
| OSD geometry | 38px from bottom, min 240px, `14px 20px`, r18 | padding/min-width/radius exact | PASS | confirm the 38px bottom margin in `osd.rs` |
| OSD glyph / value | 22px accent / 13px mono, 44px right-aligned | exact | PASS | — |
| OSD track | 8px, `accent-dim → accent` gradient, `0 0 10px` glow | exact | PASS | — |
| OSD entry | `.2s` | `220ms` | PASS | — |
| OSD dismiss | **2.6s** | *(in `osd.rs`)* | UNVERIFIED | confirm 2600ms |

### 2.6 Deliberate departures — do not "fix"

`bar.css` documents these in comments. They are correct calls:

1. **No `box-shadow` on `.bar`.** A layer-shell surface is sized to content, so an outer shadow is clipped; growing the surface either reserves dead screen or lays a click-eating strip across the display. The compositor's blur is the depth cue.
2. **MEM sparkline is gold, not accent.** Three accent sparklines in a row are unreadable.
3. **Native `GtkPopover` anchoring** instead of the spec's manual measurement — same outcome, less code.
4. **No swaync countdown hairline / toast stacking.** Documented in `swaync/style.css`: upstream renders neither and exposes no hook, so both need a fork. The spec already tags the countdown as `PROPOSED`.

---

## 3. AI panel — implementation inventory

`crates/tezca-bar/src/llmpanel.rs` (~400 lines). Present: `WIDTH = 400` (`:29`), header with `llm-dot` / `llm-model` / one `llm-icon` close button, `llm-messages` scroller, `llm-composer` + `llm-send`, `llm-status`, `submit()` (`:276`), `append_bubble()` (`:355`), `refresh_header()` (`:241`), `scroll_to_end()` (`:375`).

| Spec feature | State | Notes |
| --- | --- | --- |
| 400px, right-anchored, full height, left hairline | PASS | — |
| **Real Hyprland window, not a layer surface** | WRONG | `:146` calls `set_exclusive_zone(WIDTH)` — it is a layer-shell surface. Spec: a normal window so the tiled area reflows. Decide and align; if it stays layer-shell, update the spec. |
| `tzslide .22s` entry from `translateX(100%)` | ABSENT | GTK4 cannot transition a layer-surface position. Achievable as a real window, or drop. |
| `SUPER I` binding | UNVERIFIED | check `config/hypr/conf.d/` for the bind |
| Model selector (chevron, metadata line `Q4_K_M · resident · 128k ctx`) | ABSENT | header shows a bare name (`short_name`, `:384`) |
| Model menu — Resident / Available / Pulling sections, VRAM each, pull progress meter | ABSENT | data exists in `llm.rs`; no UI |
| Conversations button | ABSENT | only the close button exists |
| Settings drawer — system prompt block, temp + top-p sliders, keep-alive / num_ctx / gpu-layers chips | ABSENT | — |
| Transcript 16px gap | DRIFT | `.llm-messages` has `padding: 16px 14px`; the message box needs `spacing: 16` |
| User bubble | PASS | `alpha(@tz_muted,.10)` vs spec `.09`, r10, `9px 11px` — DRIFT on the alpha |
| Assistant plain 12.5px/1.62 | DRIFT | `.llm-body` 12.5px ✓; no `line-height` — GTK uses Pango line spacing, set it on the label |
| Role label 10px mono uppercase faint | DRIFT | `.llm-who` 10px mono faint; **not uppercase** — add `text-transform: uppercase` |
| **Code blocks** — 11px mono, `rgba(4,8,9,.55)`, hairline, r9 | ABSENT | `append_bubble` emits one flat `GtkLabel`; needs a markdown-ish splitter emitting code children |
| **Copy button** + `copied` for 1.4s | ABSENT | — |
| **Per-message footer** `312 tok · 7.4s · 42 tok/s` | ABSENT | — |
| **Regenerate** (truncate + re-stream) | ABSENT | — |
| **Streaming** 3–7 chars / 34ms | UNVERIFIED | check whether `submit()` streams or awaits the whole response |
| **Blinking caret** 7 × 13px accent, `tzcaret 1s step-end` | ABSENT | — |
| **Stop button** replacing send; partial kept with `stopped` footer | ABSENT | — |
| Auto-scroll unless user scrolled up | DRIFT | `scroll_to_end()` is unconditional — it will fight a user reading history |
| Composer chips — `selection` / `attach` / `screenshot` | ABSENT | — |
| Attachment row (removable) | ABSENT | — |
| Textarea auto-grow to 120px | UNVERIFIED | — |
| `⏎` send / `⇧⏎` newline | UNVERIFIED | — |
| Send button 28px accent square | DRIFT | `.llm-send` `min-width/height: 26px` → `28px` |
| Status bar — live tok/s + **12-bar sparkline**, context use, VRAM meter, `SUPER I` reminder | ABSENT | `.llm-status` is a single mono label |
| **Generation rate lifted to bar level** | UNVERIFIED | `:71` notes the panel is a *separate process* from the bar, so `llmTps` cannot be shared in-process. Either IPC the rate to the bar or accept that the bar module shows its own poll. **This contradicts the spec's premise — resolve explicitly.** |

The panel is where the bulk of remaining work sits. Everything else in this audit is edits; this is construction.

---

## 4. Shell surfaces

Built to spec. Five deltas only.

| Surface | Item | Spec | Code | Verdict |
| --- | --- | --- | --- | --- |
| **Dock** | pinned, 48px, gap 10, margin 8, hotspot 6, max_scale 1.6, influence 110 | `dock.toml` — all exact | PASS |
| | cos² falloff | `magnifier.rs:334–335` — `(FRAC_PI_2 * d/influence).cos()` squared | PASS |
| | label above 1.15× | `magnifier.rs:515` | PASS |
| | pill fill `rgba(11,14,15,.70)` | `:421` — `0.72` | DRIFT → `0.70` |
| | not CSS-transitioned | per-frame `snapshot()` | PASS |
| | pinned-then-running with divider | `apps.rs` + divider at `:436` | PASS |
| **Launcher** | 600px card, r20, 16px pad, `alpha(base,.85)`, two-layer shadow | `style.css` — all exact | PASS |
| | filled pill input inside card, r14, 19px, accent caret | exact | PASS |
| | flat list, no group headers | `layout.xml` single `GtkGridView` | PASS |
| | row r12, `10px 12px`, 2px margin, 32px icon, 15px / 12px subtext | exact | PASS |
| | selected `inset 3px` + `alpha(accent,.14)` | exact | PASS |
| | list caps 400px | `layout.xml:93` | PASS |
| | F1–F4 chips `alpha(accent,.20)` r6 | exact | PASS |
| | footer keybind pairs, accent rule, no count | exact | PASS |
| **Toasts** | 400px, r14, 12px pad, `alpha(base,.82)`, accent border, `0 6px 24px .45` | exact | PASS |
| | critical = border-only `alpha(urgent,.55)` | exact | PASS |
| | timeouts 8 / 4 / 0 | `config.json:24–26` | PASS |
| | 48px icon, r10 | r10 ✓; size from config | UNVERIFIED — confirm 48 |
| | `.summary` 14px/600, `app · time` muted 12px, body 13px/1.45 | sizes exact; **no `line-height` on body** | DRIFT — set Pango line spacing 1.45 |
| | actions all identical, **13px**, r12, `alpha(accent,.06)` on `.18` | r12 ✓, colours ✓, **font-size inherits 14px** | DRIFT → add `font-size: 13px` |
| | close button, muted → urgent fill | exact | PASS |
| **Lock** | 92px clock, 340 × 56 input | `hyprlock.conf:33`, `:71` | PASS |
| | Tezca wordmark | `:48` 22px | PASS |
| **Session** | four tiles, r20 | `wlogout/style.css:35` | PASS |
| | **shutdown is gold** | gold on `:hover` only (`:57–61`) | DRIFT — spec reads as a resting state; make the shutdown tile gold at rest, or amend the spec |
| **Theme picker** | 5 cards, live swatch strip, active accent-bordered | `pages.rs:43` `tz-theme-grid` | PASS |

---

## 5. Interaction gaps

| Interaction | State |
| --- | --- |
| Sidebar click → page + `tezca settings --page <id>` echo | PASS |
| `⌘K` / `Ctrl+K` open, `Esc` close, type-to-filter, click-to-navigate | PASS (`palette.rs`) |
| Palette: first result pre-selected, `↑↓` navigate, `↵` open | UNVERIFIED — confirm arrow keys move selection while focus is in the entry |
| Every control echoes its command | PASS |
| Displays: drag + snap; apply → revert countdown | PASS (`arrange.rs`) |
| Bar: click module → popover; scrim closes; swap directly | PASS |
| Bar: click grouped pill → expand in place; collapse button regroups | PASS (`.cluster-chip`, `button.cluster-collapse`) |
| Grouped expand **animation** (`GtkRevealer` per spec) | ABSENT — expansion is instant |
| Hover-reveal strategy | PASS (`.bar.hover-reveal .ambient`) |
| Priority tiers below `compact_width` | UNVERIFIED — `pages.rs:1691` lists all four strategies; confirm tier 3 membership is tray / brightness / GPU / caffeine / night light |
| Volume change → OSD 2.6s | UNVERIFIED |
| AI panel: send / stop / regenerate / copy / streaming | ABSENT (§3) |
| Dock: 6px hotspot magnifies per frame, labels past 1.15×, reset on leave | PASS |

---

## 6. On moving everything to Tauri

You asked: if settings goes to Tauri, why not move it all?

**Because the two surfaces have opposite cost profiles.**

`tezca-settings` is a form-heavy window you open occasionally, dismiss, and forget. Cold-start cost is amortised over a session; idle cost is zero because the process is not running. Its layout problem — 80 controls, a palette, a drag canvas — is exactly what CSS is good at. Tauri is a clear win there, and this audit is itself the argument: the settings gaps are almost all *CSS expressiveness* gaps (no hairline row idiom, no proper type scale, no `gap`, no entry animation).

`tezca-bar` is the opposite. It runs forever, on every output, from login to shutdown. A webview per monitor means a full browser engine resident permanently to draw a 40px strip — tens of MB of RSS each, a compositor surface with its own render loop, and GPU wakeups on every clock tick. And the thing GTK gives you here is not layout, it is **`wlr-layer-shell`**: exclusive-zone reservation, per-output anchoring, keyboard-interactivity modes. In a webview all of that is either reimplemented or fought. This audit's own evidence supports it — the bar is at ~90% fidelity *in GTK*, and its few misses are one-line numbers. The bar is not what failed.

The same reasoning applies to the other shell pieces, more strongly: the dock, launcher, toasts and lock screen are not yours to rewrite. `walker`, `swaync`, `hyprlock` and `wlogout` are upstream projects you are *theming*, and §4 shows the theming got you to ~95%. Moving those to Tauri means forking or replacing four working daemons to gain nothing this audit identified as a problem.

**So: Tauri for `tezca-settings`. GTK4 for the bar. Nothing for the shell but the five deltas in §4.**

Two consequences worth deciding up front if you take the Tauri route:

- The AI panel (§3) is the one genuinely ambiguous case. It is a 400px chat column with code blocks, markdown, streaming text and a copy button — every one of which is easier in a webview, and it is a real window rather than a layer surface. It is also nearly unbuilt, so there is no sunk cost. If Tauri is coming for settings anyway, **build the AI panel there too** and let `tezca-bar` keep only the module that toggles it. That resolves the "generation rate lifted to bar level" contradiction as an IPC question rather than a same-process one.
- Doing the Tauri migration makes most of §1 moot — do not spend a day converting `pt` to `px` in a stylesheet you are about to delete. §1's value in that case is as the **spec for the Tauri build**: the numbers are what the CSS should say.

---

## 7. Suggested order

1. **§0.1–0.3 settings type scale, row pattern, section rhythm.** Highest visual return per edit; this is what makes it read as the prototype. *(Skip if migrating to Tauri — carry the numbers over instead.)*
2. **§2 bar numeric deltas.** About 20 one-line changes; an hour, and the bar is done.
3. **§4 shell deltas.** Five one-line changes.
4. **§3 the AI panel.** Real construction. Decide GTK vs. Tauri, and resolve the layer-surface and shared-rate questions, before starting.
5. **§1.4 palette header restructure**, then the remaining §1 drift.
6. **Absent extras last:** rail shell, compact density, grouped-expand `GtkRevealer`, AI-chip highlight.

Items marked UNVERIFIED are ones I could not settle from a read — check them, don't rewrite them.
