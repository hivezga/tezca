-- conf.d/windowrules.lua — floating dialogs, glass tweaks, scratchpad.
--
-- Lua rule shape (replaces the 0.55 `windowrule = <rule> <value>, match:<k> <v>`
-- grammar):
--
--   hl.window_rule({ match = { class = "^(foo)$" }, float = true })
--
-- Matchers moved from `match:class X` into the `match` table; booleans are real
-- Lua booleans instead of the `on`/`off` words hyprlang needed. Because a rule
-- table takes any number of effects, the several one-effect lines each block
-- used to need collapse into a single rule per matcher.
--
-- SIZES: hyprlang took percentages (`size 60% 55%`). The Lua schema documents
-- monitor-relative EXPRESSIONS instead, so each percentage is written out as the
-- multiplication it always meant — same geometry, documented syntax.
--
-- Rules are still evaluated top to bottom, so order matters.

-- Common transient windows should float and center.
hl.window_rule({ match = { class = "^(pavucontrol|nm-connection-editor|blueman-manager)$" }, float = true })
hl.window_rule({ match = { class = "^(org.kde.polkit-kde-authentication-agent-1)$" },        float = true })
hl.window_rule({ match = { title = "^(Open File|Save File|Save As|Choose Files)$" },         float = true })
hl.window_rule({ match = { class = "^(xdg-desktop-portal-gtk)$" },                           float = true })

-- Picture-in-picture always floats and pins.
hl.window_rule({
    match = { title = "^(Picture-in-Picture)$" },
    float = true,
    pin   = true,
})

-- Keep fully-opaque content crisp behind glass (no blur cost, no dim).
hl.window_rule({
    match   = { class = "^(mpv|vlc)$" },
    opaque  = true,
    no_blur = true,
})
hl.window_rule({ match = { fullscreen = true }, opaque = true })

-- Dialogs get the soft shadow but no rounding oddities.
hl.window_rule({ match = { float = true }, rounding = 8 })

-- Scratchpad drop-down terminal (SUPER+ALT+T via scripts/scratch-term.sh; also
-- SUPER+S toggles the special workspace). Special workspace.
hl.window_rule({
    match     = { class = "^(tezca-scratch)$" },
    float     = true,
    size      = { "monitor_w * 0.60", "monitor_h * 0.55" },
    center    = true,
    workspace = "special:scratch",
})

-- tezca-settings control center (SUPER+SHIFT+A) — float + center like a macOS
-- System Settings pane. GTK4 app_id = the application-id.
hl.window_rule({
    match  = { class = "^(dev\\.tezca\\.Settings)$" },
    float  = true,
    size   = { 960, 700 },
    center = true,
})

-- Suppress the "maximize" activate events some apps spam.
hl.window_rule({
    name           = "suppress-maximize-events",
    match          = { class = ".*" },
    suppress_event = "maximize",
})
