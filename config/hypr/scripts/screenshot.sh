#!/usr/bin/env bash
# Tezca screenshot helper. Capture a region / window / monitor, put it on the
# clipboard right away, then open swappy to optionally crop / annotate / blur.
# In swappy: Ctrl+S saves to ~/Pictures/Screenshots (see config/swappy/config),
# Ctrl+C copies the annotated version. Cancel the selection and nothing happens.
# `all` skips the editor: it just grabs every monitor straight to a file + copy.
#
# Usage: screenshot.sh region|freeze|window|monitor|all
set -euo pipefail

dir="$HOME/Pictures/Screenshots"
mkdir -p "$dir"

annotate=1
case "${1:-region}" in
    region)  cap=(hyprshot -m region -s -r) ;;              # drag a box
    freeze)  cap=(hyprshot -m region -z -s -r) ;;           # freeze first, then drag
    window)  cap=(hyprshot -m window -s -r) ;;              # click one window
    monitor) cap=(hyprshot -m active -m output -s -r) ;;    # the focused monitor
    all)     cap=(grim -); annotate=0 ;;                    # every monitor, no editor
    *)       notify-send "Tezca" "Unknown screenshot mode: ${1}"; exit 1 ;;
esac

tmp="$(mktemp --suffix=.png)"
trap 'rm -f "$tmp"' EXIT

# Capture. A cancelled selection (Esc) yields no image — bail quietly.
if ! "${cap[@]}" > "$tmp" 2>/dev/null || [ ! -s "$tmp" ]; then
    exit 0
fi

wl-copy --type image/png < "$tmp"   # on the clipboard immediately

if [ "$annotate" -eq 1 ]; then
    swappy -f "$tmp"                 # crop / annotate; Ctrl+S save, Ctrl+C copy
else
    out="$dir/tezca_$(date +%Y%m%d_%H%M%S).png"
    cp "$tmp" "$out"
    notify-send "Tezca" "Screenshot saved → ${out##*/} · copied to clipboard"
fi
