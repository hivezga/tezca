<h1 align="center">Tezca</h1>

<p align="center">
  <em>An elegant, performance-first Hyprland desktop environment.</em><br>
  <strong>Obsidian aesthetic · Rust core · CSS soul · NVIDIA-native.</strong>
</p>

---

**Tezca** (← *Tezcatlipoca*, the Aztec god of the obsidian **smoking mirror**) is a
curated, macOS-15-inspired Hyprland desktop tuned to be correct and buttery on
**NVIDIA + dual-165 Hz** out of the box, beautiful through a single
wallpaper-driven theme engine, and built around a small **Rust core** so it stays
maintainable.

It's opinionated on purpose — not a pile of dotfiles, but a cohesive DE for
**gaming · AI · dev · hanging out**.

> Full rationale, decisions, and roadmap live in [`docs/DESIGN.md`](docs/DESIGN.md).

## Aesthetic — the "Smoking Mirror"

- **Obsidian** — deep near-black base, volcanic-glass surfaces.
- **Mirror** — translucency, blur, subtle sheen (macOS Sequoia glass).
- **Smoke** — soft graded greys, gentle shadows, nothing hard-edged.
- **Accent** — turquoise/jade `#3FB8AF`, used sparingly, with obsidian-gold secondary.

## Install

Targets **Arch / CachyOS** with `paru` and a Rust toolchain.

```sh
git clone https://github.com/hivezga/tezca ~/tezca
cd ~/tezca
./install.sh
```

`install.sh` installs packages via `paru`, builds the `tezca` binary, and runs
`tezca link` to symlink `config/*` into `~/.config` — **backing up anything that's
already there**. It's non-destructive and re-runnable.

Then:

```sh
tezca doctor      # verify NVIDIA env, modeset, monitors, deps
```

Log out and pick the **Hyprland (uwsm-managed)** session at SDDM. Your previous
desktop (e.g. KDE Plasma) stays selectable as a fallback the whole time.

## The `tezca` CLI

The DE's control surface — a single dependency-free Rust binary.

| Command | Does | Phase |
|---|---|---|
| `tezca link` | symlink `config/*` → `~/.config` (backs up existing) | ✅ 0/1 |
| `tezca doctor` | verify NVIDIA env, modeset, monitors, dependencies | ✅ 1 |
| `tezca theme …` | wallpaper-driven theming (list/set/wallpaper/reload) | ⏳ 3 |
| `tezca game on\|off` | toggle the gaming profile (tearing, blur off, MangoHud) | ⏳ 6 |

`tezca link --dry-run` previews every action without touching a file.

## Layout

```
config/       → symlinked into ~/.config (hypr, uwsm, kitty, …)
themes/       curated palettes (obsidian, smoke, …)
templates/    matugen templates → current/colors.*   (Phase 3)
crates/       the Rust core (tezca-cli, later tezca-dock)
wallpapers/   assets/       docs/DESIGN.md
```

## Status

Phase 0 (scaffold) + Phase 1 (bootable NVIDIA-tuned session) are in place. See the
[roadmap](docs/DESIGN.md#13-roadmap-phased-each-phase-independently-usable) for
what's next.

## License

MIT
