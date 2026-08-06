-- conf.d/keybinds.lua — Tezca keybindings, HyDE-parity layout.
--
-- SUPER is the Tezca modifier (⌘-like). This map mirrors HyDE's KEYBINDINGS.md
-- as closely as Tezca's stack allows, so HyDE muscle-memory transfers 1:1.
-- Differences by design:
--   • Menus use Walker + its elephant providers instead of HyDE's rofi.
--   • Tezca's own identity actions (AI, Claude, bespoke dock) live on SUPER+ALT+*
--     because their HyDE keys (A=app-finder, C=editor) are taken by parity.
--   • Game mode sits on SUPER+ALT+G — which is exactly where HyDE puts it too.
-- Cheat-sheet: SUPER+/  (Walker keybind hint, see scripts/cheatsheet.sh).
-- Control center (full GUI): SUPER+SHIFT+A  →  tezca-settings.
--
-- HYPRLANG → LUA BIND FLAGS. The bind variants became an options table:
--   bind   →  (no options)      binde  →  { repeating = true }
--   bindl  →  { locked = true } bindel →  { locked = true, repeating = true }
--   bindm  →  { mouse = true }  bindr  →  { release = true }
--
-- `$mod` comes from conf.d/input.lua, which is where it was always declared.

local mod = require("input").mod

-- =============================== Window management ===========================
hl.bind("CTRL + Q",             hl.dsp.window.close())                        -- close focused window (HyDE)
hl.bind("ALT + F4",             hl.dsp.window.close())                        -- close focused window (HyDE)
hl.bind(mod .. " + Q",          hl.dsp.window.close())                        -- Tezca bonus alias
hl.bind(mod .. " + Delete",     hl.dsp.exec_cmd("uwsm stop"))                 -- kill the Hyprland session (HyDE)
hl.bind(mod .. " + W",          hl.dsp.window.float({ action = "toggle" }))   -- toggle float (HyDE)
hl.bind(mod .. " + G",          hl.dsp.group.toggle())                        -- toggle group (HyDE)
hl.bind("SHIFT + F11",          hl.dsp.window.fullscreen({ mode = "fullscreen", action = "toggle" })) -- toggle fullscreen (HyDE)
hl.bind(mod .. " + F",          hl.dsp.window.fullscreen({ mode = "fullscreen", action = "toggle" })) -- Tezca bonus: fullscreen
hl.bind(mod .. " + SHIFT + F",  hl.dsp.window.pin({ action = "toggle" }))     -- toggle pin on focused window (HyDE)
hl.bind(mod .. " + J",          hl.dsp.layout("togglesplit"))                 -- toggle split — dwindle (HyDE)
hl.bind(mod .. " + L",          hl.dsp.exec_cmd("uwsm app -- hyprlock"))      -- lock screen (HyDE)
hl.bind("ALT + Control_R",      hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/bar-toggle.sh")) -- toggle the Tezca menubar (HyDE)

-- Group navigation.
-- `changegroupactive, b`/`f` split into the two named dispatchers they always were.
hl.bind(mod .. " + CTRL + H",   hl.dsp.group.prev())                          -- active group backwards (HyDE)
hl.bind(mod .. " + CTRL + L",   hl.dsp.group.next())                          -- active group forwards (HyDE)

-- --- Focus (arrows, HyDE-style) -------------------------------------------
hl.bind(mod .. " + left",  hl.dsp.focus({ direction = "l" }))
hl.bind(mod .. " + right", hl.dsp.focus({ direction = "r" }))
hl.bind(mod .. " + up",    hl.dsp.focus({ direction = "u" }))
hl.bind(mod .. " + down",  hl.dsp.focus({ direction = "d" }))

-- ALT+Tab was two `bind = ALT, Tab` lines that Hyprland ran in order. Two
-- hl.bind calls on one key do NOT stack that way in Lua — the documented form
-- for multiple actions per key is one bind holding a function.
hl.bind("ALT + Tab", function()                                               -- cycle focus (HyDE)
    hl.dispatch(hl.dsp.window.cycle_next())
    hl.dispatch(hl.dsp.window.bring_to_top())
end)

-- --- Resize the active window (SUPER+SHIFT+arrows, HyDE) -------------------
hl.bind(mod .. " + SHIFT + left",  hl.dsp.window.resize({ x = -40, y =   0, relative = true }), { repeating = true })
hl.bind(mod .. " + SHIFT + right", hl.dsp.window.resize({ x =  40, y =   0, relative = true }), { repeating = true })
hl.bind(mod .. " + SHIFT + up",    hl.dsp.window.resize({ x =   0, y = -40, relative = true }), { repeating = true })
hl.bind(mod .. " + SHIFT + down",  hl.dsp.window.resize({ x =   0, y =  40, relative = true }), { repeating = true })

-- --- Move the active window (SUPER+CTRL+SHIFT+arrows, HyDE) ----------------
hl.bind(mod .. " + CTRL + SHIFT + left",  hl.dsp.window.move({ direction = "l" }))
hl.bind(mod .. " + CTRL + SHIFT + right", hl.dsp.window.move({ direction = "r" }))
hl.bind(mod .. " + CTRL + SHIFT + up",    hl.dsp.window.move({ direction = "u" }))
hl.bind(mod .. " + CTRL + SHIFT + down",  hl.dsp.window.move({ direction = "d" }))

-- --- Move / resize with the pointer or a held key (HyDE) ------------------
-- The old `movewindow`/`resizewindow` dispatchers (the interactive, held-button
-- kind) are `window.drag()` / `window.resize()` with no args under Lua.
hl.bind(mod .. " + mouse:272", hl.dsp.window.drag(),   { mouse = true })
hl.bind(mod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })
hl.bind(mod .. " + Z",         hl.dsp.window.drag(),   { mouse = true })      -- hold to move   (HyDE)
hl.bind(mod .. " + X",         hl.dsp.window.resize(), { mouse = true })      -- hold to resize (HyDE)

-- ============================ Launcher — applications ========================
hl.bind(mod .. " + T",          hl.dsp.exec_cmd("uwsm app -- Alacritty.desktop"))               -- terminal (Alacritty)
hl.bind(mod .. " + Return",     hl.dsp.exec_cmd("uwsm app -- Alacritty.desktop"))               -- Tezca bonus alias
hl.bind(mod .. " + ALT + T",    hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/scratch-term.sh"))  -- dropdown terminal (HyDE)
hl.bind(mod .. " + E",          hl.dsp.exec_cmd("uwsm app -- dolphin"))                         -- file explorer (HyDE)
hl.bind(mod .. " + C",          hl.dsp.exec_cmd("uwsm app -- code"))                            -- text editor (HyDE)
hl.bind(mod .. " + B",          hl.dsp.exec_cmd("uwsm app -- brave-origin.desktop"))            -- Brave Origin
hl.bind("CTRL + SHIFT + Escape", hl.dsp.exec_cmd("uwsm app -- alacritty --class tezca-sysmon -e btop")) -- system monitor (HyDE)

-- =========================== Launcher — Walker menus =========================
-- Walker replaces HyDE's rofi; each menu is an elephant provider.
--
-- These deliberately call bare `walker`, not `uwsm app -- walker`. Walker runs as
-- a resident GApplication service (see autostart.lua), so each bind is a
-- ~90ms D-Bus activation of the warm instance rather than a ~1.2s cold GTK4
-- start. Wrapping that throwaway client in `uwsm app` would spin up a transient
-- systemd scope per keypress — ~180ms of pure overhead for a process that exits
-- immediately. Apps *launched from* walker are still correctly scoped: elephant
-- applies the uwsm prefix itself (auto_detect_launch_prefix, default true).
hl.bind(mod .. " + A",         hl.dsp.exec_cmd("walker -m desktopapplications")) -- application finder (HyDE)
hl.bind(mod .. " + SPACE",     hl.dsp.exec_cmd("walker"))                        -- Spotlight muscle-memory (Tezca, DESIGN §12)
-- Window switcher. HyDE binds this to `walker -m windows`, which is dead on
-- Hyprland: elephant DOES ship a `windows` provider, but it is niri-only —
-- /etc/xdg/elephant/providers/windows.so exports NiriWindow and
-- NiriWorkspaceHandler and nothing else, so the provider returns nothing here.
-- scripts/window-switcher.sh gets the same result through walker's dmenu mode,
-- the way the clipboard bind above does.
hl.bind(mod .. " + Tab",       hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/window-switcher.sh")) -- window switcher (was HyDE `walker -m windows`)
hl.bind(mod .. " + SHIFT + E", hl.dsp.exec_cmd("walker -m files"))               -- file finder (HyDE)
hl.bind(mod .. " + comma",     hl.dsp.exec_cmd("walker -m unicode"))             -- emoji picker (HyDE)
hl.bind(mod .. " + period",    hl.dsp.exec_cmd("walker -m symbols"))             -- glyph picker (HyDE)
-- dmenu mode over the service is fine (stdin and the selection both round-trip),
-- but note walker 2.17 segfaults its own service on `-d -X` (auto-select-single);
-- no bind uses -X, so keep it that way.
hl.bind(mod .. " + V",         hl.dsp.exec_cmd("cliphist list | walker -d -p Clipboard | cliphist decode | wl-copy")) -- clipboard (HyDE)
hl.bind(mod .. " + SHIFT + V", hl.dsp.exec_cmd([[uwsm app -- sh -c 'cliphist wipe && notify-send Tezca "Clipboard history cleared"']])) -- clipboard manager (HyDE)
hl.bind(mod .. " + slash",     hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/cheatsheet.sh"))       -- keybindings hint (HyDE)
hl.bind(mod .. " + SHIFT + A", hl.dsp.exec_cmd("$HOME/.local/bin/tezca settings"))                -- control center (repurposed from HyDE "select launcher")
-- Local-AI panel: a layer-shell column docked to the right edge, so the tiled
-- area reflows around it rather than being covered. The same command opens and
-- closes it — the panel is a single-instance GtkApplication, so pressing this
-- again reaches the running one and it closes itself.

-- ============================== Hardware — audio =============================
-- HyDE binds the bare F10-F12 keys as well. These grab those keys globally, so
-- they shadow in-app F10/F11/F12 — comment this block out if that bothers you.
hl.bind("F10", hl.dsp.exec_cmd("wpctl set-mute   @DEFAULT_AUDIO_SINK@ toggle"), { locked = true })                     -- mute output (HyDE)
hl.bind("F11", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),    { locked = true, repeating = true })   -- volume down (HyDE)
hl.bind("F12", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"),    { locked = true, repeating = true })   -- volume up (HyDE)
-- XF86 media keys (universal, always safe).
hl.bind("XF86AudioRaiseVolume", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"),   { locked = true, repeating = true })
hl.bind("XF86AudioLowerVolume", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),   { locked = true, repeating = true })
hl.bind("XF86AudioMute",        hl.dsp.exec_cmd("wpctl set-mute   @DEFAULT_AUDIO_SINK@ toggle"),   { locked = true })
hl.bind("XF86AudioMicMute",     hl.dsp.exec_cmd("wpctl set-mute   @DEFAULT_AUDIO_SOURCE@ toggle"), { locked = true })

-- ============================== Hardware — media =============================
hl.bind("XF86AudioPlay",  hl.dsp.exec_cmd("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioPause", hl.dsp.exec_cmd("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioNext",  hl.dsp.exec_cmd("playerctl next"),       { locked = true })
hl.bind("XF86AudioPrev",  hl.dsp.exec_cmd("playerctl previous"),   { locked = true })

-- ============================ Hardware — brightness ==========================
hl.bind("XF86MonBrightnessUp",   hl.dsp.exec_cmd("brightnessctl set 5%+"), { locked = true, repeating = true })
hl.bind("XF86MonBrightnessDown", hl.dsp.exec_cmd("brightnessctl set 5%-"), { locked = true, repeating = true })

-- ================================= Utilities =================================
hl.bind(mod .. " + K",       hl.dsp.exec_cmd("hyprctl switchxkblayout current next")) -- toggle keyboard layout (HyDE)
hl.bind(mod .. " + ALT + G", hl.dsp.exec_cmd("$HOME/.local/bin/tezca game toggle"))   -- game mode (HyDE + Tezca)
-- Screenshots — region/window/monitor land on the clipboard, then open swappy to
-- crop + annotate (Ctrl+S save, Ctrl+C copy). Print grabs everything to a file.
hl.bind(mod .. " + P",               hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/screenshot.sh region"))  -- snip a region → annotate (Tezca)
hl.bind(mod .. " + ALT + P",         hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/screenshot.sh window"))  -- snip a window → annotate (Tezca)
hl.bind(mod .. " + CTRL + P",        hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/screenshot.sh freeze"))  -- freeze + snip region → annotate (Tezca)
hl.bind(mod .. " + SHIFT + ALT + P", hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/screenshot.sh monitor")) -- snip focused monitor → annotate (Tezca)
hl.bind("Print",                     hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/screenshot.sh all"))     -- snip all monitors → save + copy (Tezca)
hl.bind(mod .. " + SHIFT + P",       hl.dsp.exec_cmd("uwsm app -- hyprpicker -a"))                        -- color picker → clipboard (HyDE)
hl.bind(mod .. " + CTRL + R",        hl.dsp.exec_cmd("hyprctl reload"))                                   -- reload Hyprland config (Tezca)
-- Screen recording — the moving-picture half of the screenshot binds above.
-- SUPER+SHIFT+R and SUPER+CTRL+R are both taken (theme selector, config reload),
-- so recording sits on ALT. Toggling: the same key stops and saves, which is a
-- clean SIGINT — the recorder needs it to finalise a playable file.
hl.bind(mod .. " + ALT + R",         hl.dsp.exec_cmd("$HOME/.local/bin/tezca record toggle"))              -- record all screens / stop (Tezca)
hl.bind(mod .. " + SHIFT + ALT + R", hl.dsp.exec_cmd("$HOME/.local/bin/tezca record toggle --region"))     -- record a region / stop (Tezca)
hl.bind(mod .. " + SHIFT + N",       hl.dsp.exec_cmd("$HOME/.local/bin/tezca night toggle"))               -- night light on/off (Tezca)

-- ============================ Theming and wallpaper ==========================
hl.bind(mod .. " + ALT + right", hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/wallpaper.sh next"))    -- next wallpaper (HyDE)
hl.bind(mod .. " + ALT + left",  hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/wallpaper.sh prev"))    -- previous wallpaper (HyDE)
hl.bind(mod .. " + SHIFT + W",   hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/wallpaper-select.sh"))  -- select wallpaper (HyDE)
hl.bind(mod .. " + SHIFT + T",   hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/theme-select.sh"))      -- select theme (HyDE)
hl.bind(mod .. " + SHIFT + R",   hl.dsp.exec_cmd("$HOME/.config/hypr/scripts/theme-select.sh"))      -- theme/mode selector (HyDE wallbash)
-- Tezca ships a single curated bar layout / animation set / lock layout, so
-- HyDE's SUPER+ALT+Up/Down, SUPER+SHIFT+Y and SUPER+SHIFT+U selectors are left
-- unbound — tune those in tezca-settings (SUPER+SHIFT+A) instead.

-- =========================== Workspaces — navigation =========================
hl.bind(mod .. " + 1", hl.dsp.focus({ workspace = 1 }))
hl.bind(mod .. " + 2", hl.dsp.focus({ workspace = 2 }))
hl.bind(mod .. " + 3", hl.dsp.focus({ workspace = 3 }))
hl.bind(mod .. " + 4", hl.dsp.focus({ workspace = 4 }))
hl.bind(mod .. " + 5", hl.dsp.focus({ workspace = 5 }))
hl.bind(mod .. " + 6", hl.dsp.focus({ workspace = 6 }))
hl.bind(mod .. " + 7", hl.dsp.focus({ workspace = 7 }))
hl.bind(mod .. " + 8", hl.dsp.focus({ workspace = 8 }))
hl.bind(mod .. " + 9", hl.dsp.focus({ workspace = 9 }))
hl.bind(mod .. " + 0", hl.dsp.focus({ workspace = 10 }))
hl.bind(mod .. " + CTRL + down",  hl.dsp.focus({ workspace = "empty" }))  -- nearest empty workspace (HyDE)
hl.bind(mod .. " + CTRL + right", hl.dsp.focus({ workspace = "r+1" }))    -- active workspace forwards (HyDE)
hl.bind(mod .. " + CTRL + left",  hl.dsp.focus({ workspace = "r-1" }))    -- active workspace backwards (HyDE)
hl.bind(mod .. " + mouse_down",   hl.dsp.focus({ workspace = "e+1" }))    -- next workspace (HyDE)
hl.bind(mod .. " + mouse_up",     hl.dsp.focus({ workspace = "e-1" }))    -- previous workspace (HyDE)

-- ============================ Workspaces — scratchpad ========================
-- `movetoworkspace` vs `movetoworkspacesilent` is now the `follow` flag: the
-- silent variant is the same move with follow = false.
hl.bind(mod .. " + S",         hl.dsp.workspace.toggle_special("scratch"))                              -- toggle scratchpad (HyDE)
hl.bind(mod .. " + SHIFT + S", hl.dsp.window.move({ workspace = "special:scratch", follow = true }))    -- move to scratchpad (HyDE)
hl.bind(mod .. " + ALT + S",   hl.dsp.window.move({ workspace = "special:scratch", follow = false }))   -- move to scratchpad, silent (HyDE)

-- ===================== Workspaces — move window to workspace =================
hl.bind(mod .. " + SHIFT + 1", hl.dsp.window.move({ workspace = 1,  follow = true }))
hl.bind(mod .. " + SHIFT + 2", hl.dsp.window.move({ workspace = 2,  follow = true }))
hl.bind(mod .. " + SHIFT + 3", hl.dsp.window.move({ workspace = 3,  follow = true }))
hl.bind(mod .. " + SHIFT + 4", hl.dsp.window.move({ workspace = 4,  follow = true }))
hl.bind(mod .. " + SHIFT + 5", hl.dsp.window.move({ workspace = 5,  follow = true }))
hl.bind(mod .. " + SHIFT + 6", hl.dsp.window.move({ workspace = 6,  follow = true }))
hl.bind(mod .. " + SHIFT + 7", hl.dsp.window.move({ workspace = 7,  follow = true }))
hl.bind(mod .. " + SHIFT + 8", hl.dsp.window.move({ workspace = 8,  follow = true }))
hl.bind(mod .. " + SHIFT + 9", hl.dsp.window.move({ workspace = 9,  follow = true }))
hl.bind(mod .. " + SHIFT + 0", hl.dsp.window.move({ workspace = 10, follow = true }))

-- Move window to workspace, silent (stay put) — HyDE uses SUPER+ALT+num.
hl.bind(mod .. " + ALT + 1", hl.dsp.window.move({ workspace = 1,  follow = false }))
hl.bind(mod .. " + ALT + 2", hl.dsp.window.move({ workspace = 2,  follow = false }))
hl.bind(mod .. " + ALT + 3", hl.dsp.window.move({ workspace = 3,  follow = false }))
hl.bind(mod .. " + ALT + 4", hl.dsp.window.move({ workspace = 4,  follow = false }))
hl.bind(mod .. " + ALT + 5", hl.dsp.window.move({ workspace = 5,  follow = false }))
hl.bind(mod .. " + ALT + 6", hl.dsp.window.move({ workspace = 6,  follow = false }))
hl.bind(mod .. " + ALT + 7", hl.dsp.window.move({ workspace = 7,  follow = false }))
hl.bind(mod .. " + ALT + 8", hl.dsp.window.move({ workspace = 8,  follow = false }))
hl.bind(mod .. " + ALT + 9", hl.dsp.window.move({ workspace = 9,  follow = false }))
hl.bind(mod .. " + ALT + 0", hl.dsp.window.move({ workspace = 10, follow = false }))

-- Move window to the next / previous relative workspace (HyDE).
hl.bind(mod .. " + ALT + CTRL + right", hl.dsp.window.move({ workspace = "r+1", follow = true }))
hl.bind(mod .. " + ALT + CTRL + left",  hl.dsp.window.move({ workspace = "r-1", follow = true }))

-- ================================ Logout menu ================================
hl.bind("ALT + CTRL + Delete", hl.dsp.exec_cmd("uwsm app -- wlogout -b 4"))   -- logout menu (HyDE)

-- ===================== Tezca identity (relocated to SUPER+ALT+*) =============
-- Their natural HyDE keys (A = app finder, C = editor, D free) are taken by
-- parity, so Tezca's signature actions cluster on the ALT layer. See ai.lua /
-- gaming.lua for the window rules and profile logic behind these.
hl.bind(mod .. " + ALT + A",         hl.dsp.workspace.toggle_special("ai"))                                          -- AI drop-down terminal (Claude Code)
hl.bind(mod .. " + ALT + SHIFT + A", hl.dsp.exec_cmd("uwsm app -- alacritty --class tezca-ai -e claude"))            -- (re)spawn the AI terminal
hl.bind(mod .. " + ALT + C",         hl.dsp.exec_cmd([[uwsm app -- sh -c 'command -v claude-desktop >/dev/null && claude-desktop || gtk-launch com.anthropic.Claude']])) -- Claude desktop app
hl.bind(mod .. " + ALT + D",         hl.dsp.exec_cmd("pkill -USR1 -x tezca-dock"))                                   -- pin/unpin the bespoke dock
hl.bind(mod .. " + D",               hl.dsp.exec_cmd("pkill -USR1 -x tezca-dock"))                                   -- Tezca bonus alias (HyDE leaves SUPER+D free)
hl.bind(mod .. " + N",               hl.dsp.exec_cmd([[uwsm app -- alacritty --class tezca-note -T "Quick Note" -e sh -c 'mkdir -p ~/Documents && ${EDITOR:-nano} ~/Documents/tezca-notes.md']])) -- quick note (Tezca)
