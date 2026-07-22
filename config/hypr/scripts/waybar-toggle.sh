#!/bin/sh
# Toggle the Tezca menubar (HyDE: ALT+Right-Control).
#
# tezca-bar hides/shows on SIGUSR1 while staying resident, so the toggle is
# instant (no relaunch flicker). If tezca-bar isn't running we fall back to the
# old Waybar kill/relaunch, so reverting to Waybar keeps this keybind working.
# Both comms are <15 chars, so `pkill -x` matches cleanly.
if pkill -0 -x tezca-bar 2>/dev/null; then
    exec pkill -USR1 -x tezca-bar
fi
if pkill -x waybar; then
    exit 0
fi
exec uwsm app -- "$HOME/.local/bin/tezca-bar"
