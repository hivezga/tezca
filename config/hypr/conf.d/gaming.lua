-- conf.d/gaming.lua — the lean, low-latency path (DESIGN.md §9, §11).
--
-- This file holds the STATIC, always-on gaming rules. The RUNTIME half of the
-- profile — stripping blur/shadows/animations for the lowest-latency present —
-- is toggled by `tezca game on|off` (SUPER+G), which layers `hyprctl eval`
-- tweaks on top of these rules and restores them with `hyprctl reload`.
--
-- `allow_tearing = true` is set globally in decoration.lua, but tearing only
-- actually happens for windows that opt in via the `immediate` rule below.
-- Desktop apps stay tear-free; games get the lowest-latency present.
--
-- Lua rule shape — see conf.d/windowrules.lua for the full note. Note that a
-- regex backslash has to be doubled here: `\d` is not a Lua string escape, so
-- it must be written `\\d` to reach the matcher intact.

-- --- Low-latency present (tearing) ----------------------------------------
-- Known game/launcher classes → allow tearing (immediate present). Steam Play
-- titles surface as `steam_app_<appid>`; native Wine/Proton windows as `*.exe`;
-- gamescope wraps problem titles in its own nested compositor.
--
-- The three effects the old config applied to steam_app_*/gamescope separately
-- (immediate, plus the decoration stripping below) collapse into one rule per
-- class, since a Lua rule table carries as many effects as you like.

-- --- Strip decoration on game windows -------------------------------------
-- No blur, fully opaque, no shadow — every frame counts. These are per-window so
-- they hold even when game mode is OFF (a game window never needs glass).
--
-- --- Auto-move games to the gaming workspace ------------------------------
-- Semantic workspace 5 (ultrawide / DP-1) is the games screen — see
-- monitors.lua. Full game windows land there so they never share a workspace
-- with the desktop, and go straight to fullscreen.
hl.window_rule({
    name       = "game-steam-app",
    match      = { class = "^(steam_app_\\d+)$" },
    immediate  = true,
    no_blur    = true,
    opaque     = true,
    no_shadow  = true,
    workspace  = "5",
    fullscreen = true,
})

hl.window_rule({
    name       = "game-gamescope",
    match      = { class = "^(gamescope)$" },
    immediate  = true,
    no_blur    = true,
    opaque     = true,
    no_shadow  = true,
    workspace  = "5",
    fullscreen = true,
})

-- Native Wine/Proton and the two big native titles get tearing only — they are
-- not auto-moved, because they are as likely to be windowed as fullscreen.
hl.window_rule({
    name      = "game-native-immediate",
    match     = { class = "^(cs2|dota2|.*\\.exe)$" },
    immediate = true,
})

-- --- Launcher / storefront chrome -----------------------------------------
-- Steam's own client windows behave better floating for dialogs.
hl.window_rule({
    match = { class = "^(steam)$", title = "^(Friends List|Steam Settings)$" },
    float = true,
})
-- Other launchers float their transient config dialogs sanely.
hl.window_rule({
    match = { class = "^(lutris)$", title = "^(.*[Cc]onfigure.*)$" },
    float = true,
})
