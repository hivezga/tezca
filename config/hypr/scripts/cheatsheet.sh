#!/bin/sh
# Keybinding cheat-sheet (HyDE: SUPER+/). Parses conf.d/keybinds.conf into a
# read-only Walker list of "COMBO — description" (every bind carries a trailing
# `# comment`). Selection is discarded; this is a hint viewer.
conf="$HOME/.config/hypr/conf.d/keybinds.conf"

awk -F'#' '
    /^[[:space:]]*binde?l?m?[[:space:]]*=/ {
        combo = $1
        sub(/^[^=]*=[[:space:]]*/, "", combo)      # drop "bind ... ="
        # keep the first two comma fields = "MODS, KEY"
        n = split(combo, a, ",")
        c = a[1] "+" a[2]
        gsub(/\$mod/, "SUPER", c)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", c)
        gsub(/[[:space:]]+/, "+", c)      # join modifiers with +
        gsub(/\++/, "+", c)               # collapse duplicates
        desc = $2
        gsub(/^[[:space:]]+/, "", desc)
        gsub(/[[:space:]]+$/, "", desc)
        if (desc != "")
            printf "%-26s  %s\n", c, desc
    }
' "$conf" | walker -d -p "Keybindings" -N >/dev/null 2>&1 || true
