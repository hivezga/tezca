#!/bin/sh
# Toggle the Tezca menubar (HyDE: ALT+Right-Control).
#
# tezca-bar hides/shows on SIGUSR1 while staying resident, so the toggle is
# instant — no relaunch, no flicker. If it isn't running, start it: that makes
# this keybind a way back from a crashed or manually stopped bar, not just a
# toggle. Its comm is <15 chars, so `pkill -x` matches cleanly.
if pkill -0 -x tezca-bar 2>/dev/null; then
    exec pkill -USR1 -x tezca-bar
fi
exec uwsm app -- "$HOME/.local/bin/tezca-bar"
