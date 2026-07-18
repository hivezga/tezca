#!/bin/sh
# Toggle Waybar (HyDE: ALT+Right-Control). If it's running, kill it; otherwise
# relaunch it the same way autostart does. `waybar` comm is <15 chars so `-x`
# matches cleanly.
if pkill -x waybar; then
    exit 0
fi
exec uwsm app -- waybar
