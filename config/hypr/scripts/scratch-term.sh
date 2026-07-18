#!/bin/sh
# Drop-down terminal (HyDE: SUPER+ALT+T). One kitty on the special `scratch`
# workspace, toggled in/out. A windowrule (windowrules.conf) floats + centers
# the `tezca-scratch` class and assigns it to special:scratch.
if hyprctl clients -j | grep -q 'tezca-scratch'; then
    hyprctl dispatch togglespecialworkspace scratch
    exit 0
fi

# First run: spawn it (the rule parks it on special:scratch, hidden), wait for
# it to map, then reveal the scratch workspace.
uwsm app -- kitty --class tezca-scratch
i=0
while [ "$i" -lt 20 ]; do
    hyprctl clients -j | grep -q 'tezca-scratch' && break
    i=$((i + 1))
    sleep 0.1
done
hyprctl dispatch togglespecialworkspace scratch
