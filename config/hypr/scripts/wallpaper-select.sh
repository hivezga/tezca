#!/bin/sh
# Wallpaper picker (HyDE: SUPER+SHIFT+W). Lists images in the wallpaper
# directory in a Walker dmenu; the choice drives `tezca theme wallpaper`.
set -eu

dir="${TEZCA_WALLPAPER_DIR:-}"
if [ -z "$dir" ]; then
    for c in "$HOME/Pictures/wallpapers" "$HOME/Pictures"; do
        [ -d "$c" ] && dir="$c" && break
    done
fi
[ -n "$dir" ] || { notify-send "Tezca" "No wallpaper directory found"; exit 1; }

sel=$(find "$dir" -maxdepth 1 -type f \
    \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.webp' \) \
    | sort | walker -d -p "Wallpaper") || exit 0

[ -n "$sel" ] && exec "$HOME/.local/bin/tezca" theme wallpaper "$sel"
