#!/usr/bin/env bash
# Project:Tezca — switch to a window by name, through walker.
#
# Walker has no window provider: that would need an elephant plugin, which is a
# Go binary rather than a config change. Its dmenu mode gets to the same place
# with what is already installed — the clipboard bind in keybinds.lua uses the
# identical pattern — so this pipes the client list in and focuses whatever
# comes back.
#
# On a tiling WM you switch far more often than you launch, which is why this is
# worth a top-level key at all.
set -euo pipefail

for bin in hyprctl jq walker; do
    command -v "$bin" >/dev/null || { echo "window-switcher: $bin not found" >&2; exit 1; }
done

# The address rides in a leading tab-separated field that is cut off before
# dispatch, so a window title containing quotes, tabs or Lua syntax is inert:
# only the field before the first tab is ever interpolated, and it is a
# hex address the compositor generated.
choice=$(
    hyprctl -j clients |
        jq -r '.[] | select(.mapped) | "\(.address)\t\(.class) — \(.title)"' |
        walker -d -p Windows
) || exit 0

[ -n "$choice" ] || exit 0
address=${choice%%$'\t'*}

# Belt and braces: refuse anything that is not the 0x-hex the compositor emits,
# rather than trusting that the cut above was enough.
case $address in
    0x[0-9a-fA-F]*) ;;
    *) exit 0 ;;
esac

hyprctl dispatch "hl.dsp.focus({ window = 'address:${address}' })"
