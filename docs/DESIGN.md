# Project:Tezca — Design Document

> An elegant, performance-first Hyprland desktop environment.
> **Obsidian aesthetic · Rust core · CSS soul · NVIDIA-native.**

---

## 1. Context — why this exists

Existing Hyprland rices are gorgeous but assume AMD/Intel and general-purpose use.
This machine is an **NVIDIA gaming/AI/dev workstation**, and the two most popular
references ([HyDE](https://github.com/HyDE-Project/HyDE),
[end-4/dots-hyprland](https://github.com/end-4/dots-hyprland)) either lean heavy
(QML/Qt shell) or under-tune NVIDIA. [omarchy](https://github.com/basecamp/omarchy)
nails the *opinionated, install-once, theme-everything* philosophy but is AMD-first
and Waybar-locked.

**Tezca's thesis:** a curated, macOS-15-inspired Hyprland DE that is (a) correct and
buttery on NVIDIA + dual 165 Hz displays out of the box, (b) beautiful through a
single wallpaper-driven theme engine, and (c) built around a small Rust core so it's
maintainable and genuinely *ours*. Shareable on GitHub as a cohesive project, not a
pile of dotfiles.

---

## 2. Identity — the "Smoking Mirror"

**Tezca** ← *Tezcatlipoca*, the Aztec god of the obsidian **smoking mirror**. That's
the whole aesthetic brief in one image:

- **Obsidian**: deep near-black base, volcanic-glass surfaces.
- **Mirror**: translucency, blur, subtle reflective sheen (macOS Sequoia glass).
- **Smoke**: soft graded greys, gentle shadows, nothing hard-edged.
- **Signature accent**: turquoise/jade (the stone Tezcatlipoca's mirror was carved
  from) with an obsidian-gold secondary — used sparingly.

Design language: **elegant, simple, highly functional.** Rounded corners, generous
spacing, thin top menubar, a floating dock with magnification, muted palette that lets
content breathe. Every pixel earns its place.

---

## 3. Target machine profile (optimize for *this*)

| Component | Spec | Design consequence |
|---|---|---|
| CPU | Ryzen 7 5800X3D (8c/16t) | Compositor/shell will never bottleneck; can afford blur + effects |
| RAM | 32 GB | Room for AI workloads alongside the DE |
| GPU | **RTX 4070 Ti** (`nvidia-open`) | Explicit-sync path; NVIDIA env in uwsm; tearing/VRR for games |
| Displays | **3440×1440@165** (ultrawide, primary) + **2560×1440@165** | Per-monitor workspaces, VRR, high-refresh animations |
| OS | CachyOS (Arch), `paru`+`yay` | AUR available; performance kernel; explicit-sync-capable |
| Session | uwsm + Hyprland (both installed) | uwsm-managed env split (the correct modern setup) |

---

## 4. Design principles

1. **Performance is a feature.** 165 Hz means every dropped frame is visible. Effects
   are tuned, not maxed; games get a lean path (blur off, tearing on).
2. **One source of truth for color.** Wallpaper → palette → every app. No hand-syncing
   hex codes across 12 config files.
3. **Small Rust core, proven components at the edges.** Don't reinvent a battle-tested
   status bar; do build the things that make Tezca *Tezca*.
4. **Non-destructive & reversible.** Everything lives in the repo and is symlinked in;
   KDE stays as a fallback session at login.
5. **Modular config.** Hyprland split into `conf.d/` fragments so a theme or a tweak
   touches one small file.

---

## 5. Component stack (the decisions)

Chosen for long-term stability + performance + aesthetics. Everything marked `repo` is
in CachyOS repos; `AUR` installs via `paru`.

| Layer | Choice | Src | Why (vs alternative) |
|---|---|---|---|
| Compositor | **Hyprland** | repo | Given. Modern, animated, scriptable |
| Session/env | **uwsm** (`env` + `env-hyprland`) | repo | Correct env handling; where NVIDIA vars belong |
| Display mgr | **SDDM** (keep) + Tezca session | repo | Already there; add session, theme later. greetd is a fallback path |
| **Menubar** | **Waybar** (top) | repo | *See §6.* Most stable/performant bar, GTK-CSS = full macOS look |
| **Dock** | **nwg-dock-hyprland** → custom `tezca-dock` | repo | *See §6.* Real macOS dock (autohide, pins); Rust replacement later |
| Launcher | **Walker** (Spotlight/Raycast style) | repo | GTK4, plugins (apps/calc/clipboard/emoji/websearch/AI actions). `wofi`/`fuzzel` fallback |
| Notifications | **swaync** | repo | Notification **center** + quick-settings panel = macOS vibe; CSS-themable |
| Wallpaper | **swww** (animated) | AUR | GPU transitions on NVIDIA; `hyprpaper` (repo) = static fallback |
| Lockscreen | **hyprlock** | repo | Native, GPU, themable |
| Idle | **hypridle** | repo | Native idle/dpms/lock orchestration |
| Logout | **wlogout** | repo | Themable power menu |
| Theme engine | **matugen** | repo | **Rust** Material-You extraction + templating — the core (see §7) |
| Terminal | **kitty** (keep) | repo | GPU-accel, ligatures, themable via templates |
| Polkit agent | **hyprpolkitagent** | repo | Native GUI auth prompts |
| Clipboard | **cliphist** + wl-clipboard | repo | History, Walker-integrated |
| Screenshots | **hyprshot** (+ grim/slurp/swappy) | repo | Region/window/annotate |
| Portals | xdg-desktop-portal-hyprland + -gtk | repo | Screenshare/file pickers (installed) |
| Audio | PipeWire/WirePlumber | (CachyOS) | Bar module + Walker control |
| Fonts | **Inter** (UI) + **JetBrains Mono Nerd** | repo | SF-Pro-like UI + icon glyphs. Maple Mono optional (AUR) |
| **Control CLI** | **`tezca`** (custom Rust) | build | Themes, wallpaper, gaming mode, install (see §8) |

---

## 6. The bar/dock decision (you asked me to make the long-term call)

**Goal restated:** macOS-15 feel = a thin **top menubar** + a **floating dock**, with
long-term *stability + performance + beauty*, ideally Rust + CSS.

**Options weighed:**

| Approach | Stability | Perf | Beauty | Rust | Verdict |
|---|---|---|---|---|---|
| Waybar (both bar+dock) | ★★★★★ | ★★★★★ | ★★★★☆ | ✗ (C++, but GTK **CSS**) | Great bar, weak *dock* |
| Ironbar (replace Waybar) | ★★★☆☆ | ★★★★☆ | ★★★★☆ | ✓ | Less mature for a daily driver |
| Quickshell (QML shell) | ★★★★☆ | ★★★☆☆ | ★★★★★ | ✗ (QML, not CSS) | Heaviest; drifts from CSS goal |
| Custom Rust bar from scratch | ★★☆☆☆ (new) | ★★★★★ | ★★★★★ | ✓ | Huge surface to reinvent |
| **Waybar menubar + dedicated dock** | ★★★★★ | ★★★★★ | ★★★★★ | partial | **Chosen** |

**Decision — hybrid, phased:**

- **Top menubar = Waybar**, initially — the most battle-tested, lowest-overhead bar,
  with GTK CSS for the obsidian-glass look (clock, tray, indicators, per-monitor
  workspaces, system stats). ***Superseded in Phase 10*** by **`tezca-bar`** (see the
  update note below): once the dock proved a bespoke gtk4-layer-shell surface is a
  daily-driveable win, the same approach applied to the menubar unlocks things Waybar
  can't render from config — inline live sparklines, expandable glass popovers, an
  MPRIS now-playing widget, per-app menus, and animated state transitions. Waybar
  stays in the repo as a documented fallback.
- **Dock = nwg-dock-hyprland** now (mature, purpose-built: autohide, pinned launchers,
  running indicators), then **replaced by `tezca-dock`** — a bespoke **Rust + GTK4 +
  gtk4-layer-shell** dock — as the flagship v2 component with real macOS *magnification*
  and glass blur.

**Why this is the right long-term shape:** concentrate scarce custom-Rust effort on the
*one* surface that off-the-shelf tools genuinely can't nail (a magnifying macOS dock),
while keeping the always-critical menubar on software that will still be maintained in
five years. You get Rust + CSS where it matters, and stability everywhere else.
(Note: **yasb** is Windows-only — not viable on Hyprland; ruled out.)

**Update (Phase 10) — `tezca-bar` lands (Custom Rust core #4).** The "keep Waybar
forever" call above is retired. `crates/tezca-bar/` is a bespoke **Rust + GTK4 +
gtk4-layer-shell** top menubar replacing Waybar, built on the same pattern as
`tezca-dock`: obsidian-glass layer surface (namespace `tezca-bar`, blurred by a
`layerrule`), one window per monitor, palette read live from
`~/.config/tezca/current/colors.css` and repainted on **SIGUSR2** (so `tezca theme`
reskins it with no restart). Left cluster = Tezca "mirror" menu · per-app label ·
per-output workspaces; centre = MPRIS now-playing; right = live CPU/MEM sparklines ·
network/volume/brightness/battery · notification bell · clock · power, each clock/
audio/network module expanding into a glass popover. Per-monitor adaptive: the
ultrawide shows the full cluster, the 1440p secondary drops the per-app label and
tightens. It also introduces the **four Tezcatlipoca themes** (`obsidian` · `xipe` ·
`huitzilopochtli` · `quetzalcoatl`) — one obsidian base, one accent per direction.
Controlled by `tezca bar start|stop|restart|toggle|config|set`; SIGUSR1 toggles
visibility (the `ALT+Right-Ctrl` bind). Waybar's `config/waybar/` is kept as a
fallback. Data sources stay shell-out/`/proc` (hyprctl, wpctl, nmcli, playerctl,
swaync-client) so the crate carries no deps beyond the dock's.

---

## 7. Theming architecture — the heart of Tezca

A single wallpaper drives the entire desktop's color. This is the feature that makes it
feel like *one* designed system instead of a rice.

```
             wallpaper.png
                  │
             ┌────▼─────┐
             │ matugen  │  (Rust, Material-You extraction)
             └────┬─────┘
                  │ renders templates → colors for every app
   ┌──────┬───────┼───────┬────────┬────────┐
   ▼      ▼       ▼        ▼        ▼        ▼
 Hyprland Waybar swaync  kitty   Walker    GTK
 (borders)(CSS)  (CSS)  (theme) (CSS)   (gtk.css)
                  │
             `tezca theme` reloads each component live
```

**Two theming modes:**

1. **Dynamic** — `tezca theme wallpaper <img>`: matugen extracts a Material-You palette
   from the image and re-skins everything. Effortless variety.
2. **Curated** — named themes in `themes/` (e.g. `obsidian`, the signature dark;
   `smoke`, a soft light variant) that pin a hand-tuned palette + wallpaper + accent,
   overriding extraction when you want a specific look.

**Mechanism (omarchy-inspired, proven):**

- Each app config `@import`s a stable path: `~/.config/tezca/current/colors.css`
  (and per-app equivalents). Components never hardcode colors.
- Switching a theme = matugen re-renders templates → repoint the `current/` symlink →
  `tezca` sends each app its reload signal (Waybar SIGUSR2, swaync reload, hyprctl
  reload, kitty remote, Walker restart). No app restarts visible to the user.
- Templates live in `templates/`; generated output in `~/.config/tezca/current/`.

**Signature palette (`obsidian`, dark-first):** obsidian `#0B0E0F` base, smoke greys,
turquoise `#3FB8AF`-family accent, obsidian-gold secondary, glass surfaces at ~85%
opacity with blur. A `smoke` light variant ships alongside.

---

## 8. The `tezca` Rust CLI (custom core #1)

A single ergonomic binary that *is* the DE's control surface. Rust workspace crate.

```
tezca theme list | names | set <name> | wallpaper <img> | reload
tezca game on | off            # toggle gaming profile (blur off, tearing on, MangoHud)
tezca dock ...                 # talk to tezca-dock
tezca display list | set <name> … | reset | brightness <name>   # per-monitor mode/scale/pos + DDC brightness
tezca wallpaper set <img> --monitor <name> | clear | apply      # per-monitor wallpaper overrides
tezca hypr get | set <opt> <val>… | reset | list                # live Hyprland option tuning that persists
tezca keybind list | rebind --line N … | restore                # inspect + rebind keybindings safely
tezca settings [--page ...]    # open tezca-settings, the GUI control center
tezca doctor                   # verify NVIDIA env, explicit sync, monitors, deps
tezca install | link           # (bootstrap wraps this) symlink configs into place
```

**Persistence model:** `hypr`/`display` write live tweaks to a delimited *managed
block* in `conf.d/local.conf` (keyed per option, so re-setting replaces rather than
appends), applied instantly via `hyprctl keyword` and surviving reload/relogin;
`keybind` rewrites the matching line in `keybinds.conf` (with an `--expect` guard,
conflict detection, and a backup for `restore`). Every write is fully reversible
(`reset` / `restore`) — a bad mode never bricks the session.

Why a CLI (not just scripts): type-safe config, one dependency-free binary to ship,
testable, and it is the backend the GUI control-center calls. It orchestrates
matugen + symlinks + reload signals so theming is atomic and reversible.

**Custom Rust core #2:** `tezca-dock` — the magnifying macOS dock (gtk4-rs).
**Custom Rust core #3:** `tezca-settings` — the obsidian-glass GTK4 control center
(Appearance/Displays/Dock/Desktop/Keybinds/Gaming/System); shells out to `tezca` for
every action, so the GUI and keyboard bindings drive identical code paths.

---

## 9. NVIDIA + dual-monitor tuning (the correctness win)

**Env — `~/.config/uwsm/env`** (general/toolkit/NVIDIA; *not* in hyprland.conf):
`__GLX_VENDOR_LIBRARY_NAME=nvidia`, `LIBVA_DRIVER_NAME=nvidia`, `NVD_BACKEND=direct`,
`GBM_BACKEND=nvidia-drm` (validate against Electron/Firefox), cursor/toolkit vars.
**`~/.config/uwsm/env-hyprland`**: `HYPR*` / `AQ_*` (aquamarine) vars.
`nvidia-open` + recent driver = **explicit sync on**, so the old
`WLR_NO_HARDWARE_CURSORS`/stutter hacks are mostly unneeded — we verify, not cargo-cult.
`tezca doctor` checks `nvidia_drm.modeset=1` and explicit-sync availability.

**Monitors — `conf.d/monitors.conf`:**
- Ultrawide `3440x1440@165` primary, `2560x1440@165` secondary, arranged L/R.
- `misc:vrr = 2` (fullscreen-only VRR — safest for mixed desktop/gaming on 165 Hz).
- Per-monitor workspace binding so each screen keeps its own workspaces.

**Gaming path (`tezca game on`):** `general:allow_tearing = true` + per-game
`immediate` window rule for lowest latency; blur/animations off for game windows;
gamemode + MangoHud (both installed via goverlay) wired to the profile; gamescope
available for problem titles.

---

## 10. Repo structure (GitHub-ready)

> Local dir is `Project:Tezca`. The **`:` is fine on Linux but not on Windows/macOS
> and awkward for a git remote** — GitHub repo will be **`tezca`** (or `project-tezca`).
> Flagging now; doesn't affect local work.

```
Project:Tezca/
├── README.md                 # screenshots, one-command install
├── install.sh                # bootstrap: deps → build tezca → link → session
├── docs/DESIGN.md            # this document
├── config/                   # → symlinked into ~/.config
│   ├── hypr/
│   │   ├── hyprland.conf      # sources conf.d/* in order
│   │   ├── conf.d/            # env, monitors, input, decoration, animations,
│   │   │                      #   keybinds, windowrules, autostart, nvidia, gaming
│   │   ├── hyprlock.conf
│   │   └── hypridle.conf
│   ├── uwsm/{env,env-hyprland}
│   ├── waybar/{config.jsonc, style.css}
│   ├── swaync/{config.json, style.css}
│   ├── walker/
│   ├── kitty/
│   └── nwg-dock-hyprland/
├── themes/                   # obsidian/, smoke/, ... (palette + wallpaper + accent)
├── templates/                # matugen templates → current/colors.*
├── crates/
│   ├── tezca-cli/            # the `tezca` binary
│   └── tezca-dock/           # signature Rust dock (Phase 4+)
├── wallpapers/
└── assets/                   # fonts, icons, sddm theme
```

**Deployment:** `install.sh` installs packages (`paru`), builds `tezca`, and calls
`tezca link` to symlink `config/*` → `~/.config/*` (backing up any existing files).
Fully reversible.

---

## 11. Workflow features (gaming · AI · dev · hanging out)

- **Workspaces**: semantic per-monitor sets — e.g. ultrawide = `1 dev · 2 web · 3 chat`,
  secondary = `code/logs/monitoring`. Special "scratchpad" workspace for a drop-down
  terminal and an AI scratch window.
- **Gaming**: `tezca game on` profile; Steam/Proton/gamescope window rules; MangoHud
  overlay toggle; VRR + tearing; auto-move known games to a fullscreen workspace.
- **AI**: Walker actions + keybinds to launch Claude / chat / a local-LLM scratchpad;
  a dedicated AI workspace; quick-capture note window. (Ollama optional, later.)
- **Dev**: editor/terminal/browser workspace layout, project-launcher via Walker,
  clipboard history, screenshot-to-annotate flow.
- **Hanging out**: media keys, now-playing in the menubar, animated wallpaper, blur.

---

## 12. Keybinding philosophy

`SUPER` as the Tezca modifier (mirrors macOS `⌘`). The map follows a **HyDE-parity
layout** ([HyDE KEYBINDINGS.md](https://github.com/HyDE-Project/HyDE/blob/master/KEYBINDINGS.md))
so anyone coming from HyDE keeps their muscle memory — HyDE's rofi menus map onto Walker's
elephant providers (`walker -m windows|clipboard|unicode|symbols|files`). Tezca's own
signature actions (AI terminal, Claude, the bespoke dock) cluster on `SUPER+ALT` because
their HyDE keys (`A`=app-finder, `C`=editor) are taken by parity; game mode lands on
`SUPER+ALT+G`, exactly where HyDE puts it. `SUPER+SPACE` stays mapped to Walker (Spotlight
muscle memory) alongside the HyDE `SUPER+A`. Media/brightness on XF86 keys (plus HyDE's
`F10/F11/F12`). Discoverable + self-documenting: `SUPER+/` pops a Walker cheat-sheet parsed
live from the config (`scripts/cheatsheet.sh`), and the **Keybinds** tab of `tezca-settings`
renders the same. Full map in `conf.d/keybinds.conf`; helper scripts in `conf.d/../scripts/`.

---

## 13. Roadmap (phased, each phase independently usable)

| Phase | Deliverable | You can... |
|---|---|---|
| **0 · Repo scaffold** | Git repo, structure, `install.sh` + `tezca link` skeleton, README | Clone & link |
| **1 · Bootable session** | uwsm+Hyprland session, NVIDIA env, monitors, `tezca doctor` green | Log into a stable Tezca session |
| **2 · Aesthetic core** | Waybar menubar, swaync, Walker, kitty, swww, obsidian theme, keybinds | Daily-drive a beautiful desktop |
| **3 · Theme engine** | matugen templates, `tezca theme`, dynamic + curated modes | One-command re-skin from any wallpaper |
| **4 · Dock + polish** | nwg-dock styled, hyprlock/hypridle/wlogout, animations tuned | Full macOS-feel dock + lock/idle |
| **5 · Rust dock** | `tezca-dock` (gtk4-rs) with magnification, replaces nwg-dock | Signature bespoke dock |
| **6 · Gaming/AI profiles** | `tezca game`, AI workspace/launchers, gamescope rules | Optimized modes per activity |
| **7 · Share** | Screenshots, docs, curated themes, GitHub release | Publish `tezca` |

---

## 14. Verification strategy

- **Per phase, live**: after Phase 1 we switch you to the Tezca session and validate on
  the real hardware (both monitors, refresh rate, no NVIDIA flicker) via `tezca doctor`
  + visual check. KDE remains selectable at SDDM the entire time.
- **Theme engine**: switch wallpaper → confirm Waybar/kitty/swaync/Hyprland all recolor
  live with no restarts.
- **Gaming**: launch a title, confirm tearing/VRR active, blur off, MangoHud overlay,
  frame pacing on 165 Hz.
- **Reversibility**: `tezca link` backs up originals; uninstall path restores them.

---

## 15. Open questions / risks

- **swww vs hyprpaper**: swww (animated, AUR) is the aesthetic pick; hyprpaper (repo,
  static) is the zero-risk fallback. Default swww, keep hyprpaper as escape hatch.
- **GBM_BACKEND=nvidia-drm** occasionally breaks Electron/Firefox HW-accel — validate on
  your Brave/Antigravity/Claude apps; drop if it regresses.
- **`tezca-dock` scope**: magnification + blur in gtk4-rs is real work; nwg-dock covers
  us fully until it's ready, so it's never blocking.
- **Repo name**: `tezca` on GitHub (the local `:` path stays as-is).
