#!/bin/sh
# Theme picker (HyDE: SUPER+SHIFT+T, and reused for SUPER+SHIFT+R). Lists the
# curated themes (via `tezca theme names`) in a Walker dmenu; the choice drives
# `tezca theme set`.
set -eu

sel=$("$HOME/.local/bin/tezca" theme names | walker -d -p "Theme") || exit 0
[ -n "$sel" ] && exec "$HOME/.local/bin/tezca" theme set "$sel"
