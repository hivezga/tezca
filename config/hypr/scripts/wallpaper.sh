#!/bin/sh
# Cycle the wallpaper (HyDE: SUPER+ALT+Left/Right). Walks the images in the
# wallpaper directory and feeds the next/previous one to `tezca theme wallpaper`
# (dynamic matugen re-skin). Directory: $TEZCA_WALLPAPER_DIR, else the first of
# ~/Pictures/wallpapers or ~/Pictures.
#
# Usage: wallpaper.sh next|prev
set -eu

dir="${TEZCA_WALLPAPER_DIR:-}"
if [ -z "$dir" ]; then
    for c in "$HOME/Pictures/wallpapers" "$HOME/Pictures"; do
        [ -d "$c" ] && dir="$c" && break
    done
fi
[ -n "$dir" ] || { notify-send "Tezca" "No wallpaper directory found"; exit 1; }

list=$(find "$dir" -maxdepth 1 -type f \
    \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.webp' \) | sort)
[ -n "$list" ] || { notify-send "Tezca" "No images in $dir"; exit 1; }

count=$(printf '%s\n' "$list" | wc -l)
current=$(cat "$HOME/.config/tezca/current/wallpaper" 2>/dev/null || true)

# 1-based index of the current wallpaper (0 if unknown → wraps sensibly).
idx=$(printf '%s\n' "$list" | grep -nxF "$current" 2>/dev/null | head -n1 | cut -d: -f1 || true)
[ -n "${idx:-}" ] || idx=0

case "${1:-next}" in
    prev) new=$((idx - 1)); [ "$new" -lt 1 ] && new=$count ;;
    *)    new=$((idx + 1)); [ "$new" -gt "$count" ] && new=1 ;;
esac

img=$(printf '%s\n' "$list" | sed -n "${new}p")
exec "$HOME/.local/bin/tezca" theme wallpaper "$img"
