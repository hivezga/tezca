#!/bin/sh
# Project:Tezca — nwg-dock launcher (single source of truth for the dock flags).
#
# Launched at login by conf.d/autostart.conf (`uwsm app -- .../dock.sh`, so it
# lands in the session's systemd slice) and re-launched by `tezca theme` when a
# theme switch needs the dock to reload its CSS (nwg-dock only reads style.css at
# start). Keeping the flags here means both paths stay identical.
#
# macOS feel: floating + autohiding at the bottom, running-app icons only, a
# small gap off the screen edge. NO exclusive zone (-x) so windows tile full
# height and the dock overlays on demand.
#
#   -d                 autohide: reveal on bottom hotspot, hide on leave/click
#   -i 44              icon size
#   -w 10              workspace count (matches keybinds.conf)
#   -p bottom          bottom edge
#   -l overlay         draw above windows when shown
#   -nolauncher        no launcher button (SUPER+SPACE / Walker covers that)
#   -mb 6              bottom margin (float above the edge)
#   -iw special:scratch   keep the drop-down scratch terminal out of the dock
exec nwg-dock-hyprland -d -i 44 -w 10 -p bottom -l overlay -nolauncher -mb 6 -iw "special:scratch"
