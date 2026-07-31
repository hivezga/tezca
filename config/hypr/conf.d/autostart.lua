-- conf.d/autostart.lua — session startup.
--
-- Only what a correct session needs, in dependency order: services first, then the
-- shell surfaces (bar, notifications, launcher), then the wallpaper daemon.
--
-- Launch GUI services through `uwsm app` so they land in the right systemd slice
-- (correct cgroup accounting, clean shutdown).
--
-- `exec-once` has no Lua keyword: it is the `hyprland.start` event, which fires
-- once per session and — unlike a bare top-level call — does NOT re-fire on
-- `hyprctl reload`. That is what keeps a config reload from stacking a second
-- copy of every daemon. `hl.exec_cmd` runs through `sh -c` and spawns
-- asynchronously, so the `&&`, `||` and `$HOME` below still work as written and
-- nothing needs `& disown`.

hl.on("hyprland.start", function()
    -- --- Phase 1: essential -------------------------------------------------
    -- Polkit agent — GUI auth prompts (sudo-in-GUI, mount, etc.).
    hl.exec_cmd("uwsm app -- systemctl --user start hyprpolkitagent 2>/dev/null || uwsm app -- /usr/lib/hyprpolkitagent/hyprpolkitagent")

    -- Clipboard history daemon (text + images) — cheap, useful from day one.
    hl.exec_cmd("uwsm app -- wl-paste --type text --watch cliphist store")
    hl.exec_cmd("uwsm app -- wl-paste --type image --watch cliphist store")

    -- --- Phase 2: aesthetic core --------------------------------------------
    -- Menubar + notification center.
    -- The bespoke gtk4-rs menubar (Phase 10) replaces Waybar. Absolute path on
    -- purpose — the systemd --user PATH that uwsm apps inherit at login does NOT
    -- include ~/.local/bin (where install.sh puts the binary), so a bare
    -- `tezca-bar` would silently fail to launch, exactly like tezca-dock below.
    -- $HOME is expanded by the shell hl.exec_cmd runs the command through.
    hl.exec_cmd("uwsm app -- $HOME/.local/bin/tezca-bar")
    hl.exec_cmd("uwsm app -- swaync")
    -- Walker launcher's provider backend (SUPER+SPACE).
    hl.exec_cmd("uwsm app -- elephant")
    -- Walker itself, as a resident GApplication service. This is what makes the
    -- launcher feel instant: cold-starting walker per keypress cost ~1.2s (GTK4
    -- init + theme/CSS load + elephant handshake), long enough that a second
    -- SUPER+SPACE lands mid-startup and `close_when_open` cancels the launch
    -- outright — the "press it and nothing happens" bug. With the service
    -- resident, the keybind is a D-Bus activation of an already-warm process:
    -- ~90ms, and the toggle is race-free. The binds in keybinds.lua therefore
    -- call bare `walker`, NOT `uwsm app -- walker` (see the note there).
    -- Waits for elephant on its own ("waiting for elephant to start..."), so the
    -- ordering above is a preference, not a requirement.
    hl.exec_cmd("uwsm app -- walker --gapplication-service")
    -- Wallpaper daemon. NOTE: swww was renamed "awww" upstream (Provides/Replaces
    -- swww, but the binaries are awww / awww-daemon — there is no swww-daemon).
    hl.exec_cmd("uwsm app -- awww-daemon")
    -- Idle/lock/dpms orchestration (config: hypr/hypridle.conf).
    hl.exec_cmd("uwsm app -- hypridle")
    -- Bespoke gtk4-rs magnifying macOS dock (Phase 5). Autohides at the bottom
    -- edge, magnifies on hover, glass + theme-driven colors. Reads
    -- ~/.config/tezca-dock/dock.toml. Launched through `uwsm app` for the correct
    -- systemd slice. NOTE: absolute path on purpose, same reason as tezca-bar.
    hl.exec_cmd("uwsm app -- $HOME/.local/bin/tezca-dock")

    -- --- Wallpaper ----------------------------------------------------------
    -- `tezca wallpaper apply` paints the ACTIVE theme's wallpaper on every output
    -- AND re-applies any per-monitor overrides (set in tezca-settings → Displays).
    -- It reads ~/.config/tezca/current/wallpaper + monitor-wallpapers, so it
    -- follows theme switches automatically (no hard-coded image). Absolute path:
    -- ~/.local/bin isn't on the uwsm --user PATH at login.
    hl.exec_cmd('sleep 1 && [ -x "$HOME/.local/bin/tezca" ] && "$HOME/.local/bin/tezca" wallpaper apply')
end)

-- Flat obsidian base, so there's never a jarring default-grey flash before the
-- wallpaper daemon is up.
--
-- This was `exec-once = hyprctl keyword misc:background_color 0x0B0E0F`. That
-- shelling-out is not portable to Lua at all: `hyprctl keyword` is a legacy-only
-- entry point with no counterpart under the Lua config manager. It was never
-- anything but a config value set the long way round, so it is one now — which
-- also lands it at config-parse time rather than a process spawn after startup,
-- closing the window where the flash could happen.
hl.config({
    misc = {
        background_color = "0x0B0E0F",
    },
})
