#!/bin/sh
# Keybinding cheat-sheet (HyDE: SUPER+/). A read-only Walker list of
# "COMBO  description"; the selection is discarded, this is a hint viewer.
#
# The bind list comes from `tezca keybind list --machine`, not from re-parsing
# keybinds.conf with awk. That matters for correctness now that rebinds live in an
# override layer (~/.config/tezca/keybinds.conf): a pass over the shipped map alone
# would show the *shipped* combo for every key you have rebound. It also leaves
# exactly one implementation of the bind parser, which DESIGN.md §12 already
# claimed ("parsed live from the config").
#
# Machine format:  B \t line \t mods \t key \t desc \t action \t overridden(0|1)
tezca="$HOME/.local/bin/tezca"
[ -x "$tezca" ] || tezca=tezca

"$tezca" keybind list --machine 2>/dev/null | awk -F'\t' '
    $1 == "B" && $5 != "" {
        combo = $3
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", combo)
        gsub(/[[:space:]]+/, "+", combo)              # join modifiers with +
        if (combo == "")      combo = $4              # a bind with no modifier
        else if ($4 != "")    combo = combo "+" $4
        # Flag a bind you have rebound, so the sheet and reality agree visibly.
        printf "%-26s  %s%s\n", combo, $5, ($7 == "1" ? "  *" : "")
    }
' | walker -d -p "Keybindings" -N >/dev/null 2>&1 || true
