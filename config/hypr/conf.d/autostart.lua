-- conf.d/autostart.lua — session startup.
--
-- Only what a correct session needs, in dependency order: services first, then the
-- shell surfaces (bar, notifications, launcher), then the wallpaper daemon — and
-- finally whatever the user has added themselves.
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
--
-- USER CONTROL. Every shipped launch below carries an id and goes through
-- `launch(id, cmd)`, which skips it when that id appears in the `disabled` list
-- of ~/.config/tezca/startup.lua. That file also holds the user's own entries,
-- and is managed by `tezca startup` / Settings → Startup. It lives outside the
-- repo on purpose: this file is a symlink into the checkout, so adding an app by
-- editing it would dirty the working tree and break `git pull` downstream.
--
-- It is loaded through `util.load`, which returns nil for a missing OR malformed
-- file. That matters more than it looks: an uncaught Lua error anywhere in the
-- config aborts the whole thing and drops Hyprland into emergency mode — one
-- keybind, black screen. A broken startup.lua must cost you your extra apps,
-- never your session.

local util = require("util")

local startup = util.load(util.config("tezca/startup.lua"))
if type(startup) ~= "table" then
    startup = {}
end

-- Shipped-service ids the user has switched off, as a set for cheap lookup.
local off = {}
for _, id in ipairs(startup.disabled or {}) do
    if type(id) == "string" then
        off[id] = true
    end
end

--- Launch a shipped service unless the user disabled it.
--- @param id string   stable id, matching the SHIPPED table in cmd_startup.rs
--- @param cmd string
local function launch(id, cmd)
    if not off[id] then
        hl.exec_cmd(cmd)
    end
end

hl.on("hyprland.start", function()
    -- --- Phase 1: essential -------------------------------------------------
    -- Polkit agent — GUI auth prompts (sudo-in-GUI, mount, etc.).
    launch("polkit", "uwsm app -- systemctl --user start hyprpolkitagent 2>/dev/null || uwsm app -- /usr/lib/hyprpolkitagent/hyprpolkitagent")

    -- Clipboard history daemon (text + images) — cheap, useful from day one.
    launch("cliphist-text", "uwsm app -- wl-paste --type text --watch cliphist store")
    launch("cliphist-image", "uwsm app -- wl-paste --type image --watch cliphist store")

    -- --- Phase 2: aesthetic core --------------------------------------------
    -- Menubar + notification center.
    -- The bespoke gtk4-rs menubar (Phase 10) replaces Waybar. Started through
    -- its own systemd user unit rather than `uwsm app --`, because a bare app
    -- launch has no supervision: when the bar took a SIGSEGV overnight the
    -- desktop lost it until someone noticed by eye. The unit restarts it,
    -- captures its output in the journal (`tezca bar logs`), and makes
    -- `tezca bar status` authoritative. See config/systemd/user/tezca-bar.service.
    -- `start` is idempotent, so this is harmless if the unit is also enabled
    -- into graphical-session.target — and it is what makes the bar come up on a
    -- machine where it never got enabled.
    -- Deliberately NOT wrapped in `launch()`: the bar is the way back to the
    -- settings window that would switch it on again, so `tezca startup` refuses
    -- to disable it and there is nothing to check here.
    -- `reset-failed` first: the unit is also wanted by graphical-session.target,
    -- which can fire before uwsm has finished exporting WAYLAND_DISPLAY into the
    -- user manager. A bar that starts without a display exits, burns its restart
    -- budget and lands in `failed`, where a plain `start` is refused for
    -- "start request repeated too quickly". Clearing the counter first makes
    -- this line — which runs from inside a compositor that definitely exists —
    -- the authority on whether the bar comes up.
    hl.exec_cmd(
        "systemctl --user reset-failed tezca-bar.service; "
            .. "systemctl --user start --no-block tezca-bar.service"
    )
    launch("swaync", "uwsm app -- swaync")
    -- Walker launcher's provider backend (SUPER+SPACE).
    launch("elephant", "uwsm app -- elephant")
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
    launch("walker", "uwsm app -- walker --gapplication-service")
    -- Wallpaper daemon. NOTE: swww was renamed "awww" upstream (Provides/Replaces
    -- swww, but the binaries are awww / awww-daemon — there is no swww-daemon).
    launch("awww", "uwsm app -- awww-daemon")
    -- Idle/lock/dpms orchestration. Prefers the config generated by `tezca idle`
    -- (Settings → Power) and falls back to the shipped hypridle.conf when that
    -- file does not exist, which is the state on a fresh install.
    launch(
        "hypridle",
        'uwsm app -- sh -c \'cfg="${XDG_CONFIG_HOME:-$HOME/.config}/tezca/hypridle.conf"; '
            .. 'if [ -f "$cfg" ]; then exec hypridle -c "$cfg"; else exec hypridle; fi\''
    )
    -- Bespoke gtk4-rs magnifying macOS dock (Phase 5). Autohides at the bottom
    -- edge, magnifies on hover, glass + theme-driven colors. Reads
    -- ~/.config/tezca-dock/dock.toml. Launched through `uwsm app` for the correct
    -- systemd slice. NOTE: absolute path on purpose, same reason as tezca-bar.
    launch("tezca-dock", "uwsm app -- $HOME/.local/bin/tezca-dock")

    -- --- Wallpaper ----------------------------------------------------------
    -- `tezca wallpaper apply` paints the ACTIVE theme's wallpaper on every output
    -- AND re-applies any per-monitor overrides (set in tezca-settings → Displays).
    -- It reads ~/.config/tezca/current/wallpaper + monitor-wallpapers, so it
    -- follows theme switches automatically (no hard-coded image). Absolute path:
    -- ~/.local/bin isn't on the uwsm --user PATH at login.
    launch("wallpaper", 'sleep 1 && [ -x "$HOME/.local/bin/tezca" ] && "$HOME/.local/bin/tezca" wallpaper apply')

    -- --- Night light --------------------------------------------------------
    -- `tezca night apply` re-applies the saved colour temperature, and is a no-op
    -- when night light is switched off — so this costs one short-lived process at
    -- login and nothing else.
    launch("night", '[ -x "$HOME/.local/bin/tezca" ] && "$HOME/.local/bin/tezca" night apply')

    -- --- The user's own apps ------------------------------------------------
    -- Managed by `tezca startup` / Settings → Startup. `delay` exists because
    -- some apps want the bar and its tray up before they launch; it is applied
    -- with `sleep`, which costs nothing (hl.exec_cmd already goes through a shell
    -- and does not block).
    for _, e in ipairs(startup.entries or {}) do
        if type(e) == "table" and type(e.exec) == "string" and e.exec ~= "" and e.enabled ~= false then
            local delay = tonumber(e.delay) or 0
            if delay > 0 then
                hl.exec_cmd("sleep " .. math.floor(delay) .. " && " .. e.exec)
            else
                hl.exec_cmd(e.exec)
            end
        end
    end
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
