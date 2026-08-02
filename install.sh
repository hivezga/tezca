#!/usr/bin/env bash
# ┌─────────────────────────────────────────────────────────────────────┐
# │  Project:Tezca — bootstrap                                            │
# │  deps (paru) → build `tezca` → `tezca link` → next steps             │
# │                                                                       │
# │  Non-destructive: `tezca link` backs up any existing config first.    │
# │  Idempotent: safe to re-run.                                          │
# └─────────────────────────────────────────────────────────────────────┘
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOLD=$'\e[1m'; DIM=$'\e[2m'; CYAN=$'\e[36m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; RED=$'\e[31m'; RST=$'\e[0m'

say()  { printf '%s◆%s %s%s%s\n' "$CYAN" "$RST" "$BOLD" "$1" "$RST"; }
info() { printf '  %s\n' "$1"; }
warn() { printf '  %s!%s %s\n' "$YELLOW" "$RST" "$1"; }
die()  { printf '  %s✗%s %s\n' "$RED" "$RST" "$1" >&2; exit 1; }

confirm() {
    local prompt="${1:-Proceed?}"
    read -rp "  ${prompt} [y/N] " ans
    [[ "$ans" == [yY] || "$ans" == [yY][eE][sS] ]]
}

# --- 0. sanity ------------------------------------------------------------
say "Project:Tezca installer"
info "repo: ${DIM}${REPO_DIR}${RST}"
echo

[[ "$(uname -s)" == "Linux" ]] || die "Tezca targets Linux (Hyprland)."
command -v paru >/dev/null || die "paru not found. Tezca targets Arch/CachyOS with paru."
command -v cargo >/dev/null || die "cargo not found. Install rustup and a stable toolchain."

# The local repo path may contain a ':' (Project:Tezca), which breaks cargo's
# LD_LIBRARY_PATH. If so, build into a colon-free cache dir. A clean GitHub
# clone (named 'tezca') has no colon and this is a no-op.
TARGET_DIR="${REPO_DIR}/target"
if [[ "$REPO_DIR" == *:* ]]; then
    TARGET_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/tezca/target"
    warn "repo path contains ':' — building into ${DIM}${TARGET_DIR}${RST}"
fi

# --- 1. packages ----------------------------------------------------------
# Phase 1 = a bootable session; Phase 2 = the aesthetic stack. We install both
# so the desktop is ready as phases are enabled. Anything already present is
# skipped by --needed.
PKGS_CORE=(hyprland uwsm hyprpolkitagent
           xdg-desktop-portal-hyprland xdg-desktop-portal-gtk
           qt5-wayland qt6-wayland alacritty
           cliphist wl-clipboard
           pipewire wireplumber
           networkmanager bluez bluez-utils
           polkit brightnessctl playerctl)

PKGS_AESTHETIC=(swaync
                hyprlock hypridle wlogout hyprsunset
                hyprshot grim slurp swappy hyprpicker wf-recorder
                gtk4 gtk4-layer-shell ddcutil
                inter-font ttf-jetbrains-mono-nerd)

# Phase 6 — gaming/AI workflow: the `tezca game run` wrapper (gamemode +
# MangoHud) and gamescope for problem titles. notify-send for the toggle toast.
PKGS_WORKFLOW=(gamemode mangohud gamescope libnotify)

# AUR / possibly-AUR (paru resolves either way).
# elephant-all-bin is walker's provider backend — without it the launcher opens
# but returns zero results, so it is not optional despite living here.
# awww is swww's renamed successor: the binaries are awww / awww-daemon, and
# there is no swww-daemon, so install it under its real name rather than relying
# on the Provides alias.
PKGS_AUR=(walker-bin elephant-all-bin awww matugen-bin)

say "Packages"
info "core:      ${DIM}${PKGS_CORE[*]}${RST}"
info "aesthetic: ${DIM}${PKGS_AESTHETIC[*]}${RST}"
info "workflow:  ${DIM}${PKGS_WORKFLOW[*]}${RST}"
info "aur:       ${DIM}${PKGS_AUR[*]}${RST}"
echo
MISSING_AUR=()
if confirm "Install/verify these packages with paru?"; then
    paru -S --needed "${PKGS_CORE[@]}" "${PKGS_AESTHETIC[@]}" "${PKGS_WORKFLOW[@]}"
    # AUR names occasionally differ across time; don't let one bad name abort.
    # Collect the misses instead of only warning in passing — a warning 200 lines
    # of pacman output ago is a warning nobody sees, and a silently absent matugen
    # means `tezca theme wallpaper` fails much later with no obvious cause.
    for p in "${PKGS_AUR[@]}"; do
        paru -S --needed "$p" || MISSING_AUR+=("$p")
    done
else
    warn "skipping package install"
fi
echo

# --- 2. build tezca + tezca-dock + tezca-bar + tezca-settings -------------
# The workspace build compiles the std-only `tezca` CLI plus the three gtk4-rs
# binaries: `tezca-dock` (magnifying dock), `tezca-bar` (top menubar), and
# `tezca-settings` (control center). The GTK crates (already in the package set
# below) are slow on first build.
say "Building the tezca workspace (CLI + dock + bar + settings)"
( cd "$REPO_DIR" && CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release )

# Run the test suite before putting anything on PATH. These binaries rewrite the
# Hyprland config of the session you are about to log into, so "it compiled" is
# not the bar worth clearing: the tests cover the config rewriting, the keybind
# override layer, and the symlink/seed logic.
say "Running the test suite"
if ( cd "$REPO_DIR" && CARGO_TARGET_DIR="$TARGET_DIR" cargo test --workspace --quiet ); then
    info "${GREEN}✓${RST} tests pass"
else
    warn "tests FAILED — see the output above"
    confirm "Install anyway?" || die "aborted (nothing was installed or linked)"
fi

BIN_DIR="${HOME}/.local/bin"
mkdir -p "$BIN_DIR"
for b in tezca tezca-dock tezca-bar tezca-settings; do
    SRC="${TARGET_DIR}/release/${b}"
    [[ -x "$SRC" ]] || die "build succeeded but $SRC is missing"
    install -m755 "$SRC" "${BIN_DIR}/${b}"
    info "${GREEN}✓${RST} installed → ${DIM}${BIN_DIR}/${b}${RST}"
done
BIN="${BIN_DIR}/tezca"
case ":$PATH:" in
    *":${HOME}/.local/bin:"*) : ;;
    *) warn "~/.local/bin is not on PATH — add it to use \`tezca\` directly" ;;
esac

# tezca-settings desktop entry — so it shows up in Walker (SUPER+A) and the dock.
APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$APPS_DIR"
install -m644 "${REPO_DIR}/config/tezca-settings/tezca-settings.desktop" \
    "${APPS_DIR}/tezca-settings.desktop"
info "${GREEN}✓${RST} installed → ${DIM}${APPS_DIR}/tezca-settings.desktop${RST}"
echo

# --- 3. link config -------------------------------------------------------
say "Linking config into ~/.config"
info "${DIM}(existing files are backed up to *.bak.<epoch>. --force skips the backup${RST}"
info "${DIM} only for symlinks, which hold no data — real files are always saved.)${RST}"
echo
if confirm "Run \`tezca link\` now?"; then
    TEZCA_REPO="$REPO_DIR" "$BIN" link
else
    warn "skipped — run \`tezca link\` yourself when ready"
fi
echo

# --- 4. next steps --------------------------------------------------------
if (( ${#MISSING_AUR[@]} )); then
    say "Packages that did NOT install"
    for p in "${MISSING_AUR[@]}"; do
        case "$p" in
            matugen-bin) warn "$p — \`tezca theme wallpaper\` (dynamic theming) will not work without it" ;;
            awww)        warn "$p — no animated wallpaper; \`tezca theme\` will report it as skipped" ;;
            walker-bin|elephant-all-bin)
                         warn "$p — the launcher (SUPER+A / SUPER+SPACE) will not open" ;;
            *)           warn "$p — install it manually when you can" ;;
        esac
    done
    info "${DIM}retry with: paru -S ${MISSING_AUR[*]}${RST}"
    echo
fi

say "Done"
cat <<EOF
  ${GREEN}Next:${RST}
    1. ${BOLD}tezca doctor${RST}   — verify NVIDIA env, modeset, monitors, deps
    2. Log out, and at SDDM pick the ${BOLD}Hyprland (uwsm-managed)${RST} session.
       (KDE Plasma stays selectable as a fallback.)
    3. After first login, run ${BOLD}hyprctl monitors${RST} and fix connector names
       in ${DIM}config/hypr/conf.d/monitors.lua${RST} if they differ from DP-1/DP-2.
    4. Theme it: ${BOLD}tezca theme wallpaper ~/Pictures/some.jpg${RST} re-skins the
       whole desktop from any image, or ${BOLD}tezca theme set obsidian${RST} for the
       signature look. (${DIM}tezca link${RST} already seeded obsidian as the default.)

  ${DIM}Everything is reversible: your originals are the *.bak.* files next to the
  new symlinks in ~/.config.

  Your settings live outside this repo, so \`git pull\` never conflicts with them:
    ~/.config/tezca/overrides.lua   display + Hyprland tweaks
    ~/.config/tezca/keybinds.lua    your rebound keys
    ~/.config/tezca-bar/            bar config + your custom modules
    ~/.config/tezca-dock/           dock config${RST}
EOF
