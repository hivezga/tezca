-- conf.d/decoration.lua — the "smoking mirror" look: obsidian glass, soft smoke.
--
-- Border + shadow colors come from the theme engine. The palette lives in
-- ~/.config/tezca/current/colors-hypr.lua, which the `tezca` CLI repoints per
-- theme (obsidian default, or wallpaper-extracted via matugen). Loaded FIRST so
-- the values exist before the general block below uses them. `hyprctl reload`
-- (sent by `tezca theme`) recolors the borders live. Edit templates/ or themes/.

local util = require("util")

-- The obsidian defaults, inlined as the fallback. hyprlang would merely log an
-- error for a missing `source` and carry on with unset $tz_* variables; a failed
-- load in Lua would abort the whole config, so the palette has to have a floor.
-- Kept in step with themes/obsidian/colors-hypr.lua.
local theme = util.load(util.config("tezca/current/colors-hypr.lua")) or {
    accent     = "rgba(3FB8AFff)",
    accent_dim = "rgba(2A8C86ff)",
    inactive   = "rgba(1A1E1Faa)",
    shadow     = "rgba(00000055)",
    shadow_dim = "rgba(00000033)",
}

hl.config({
    general = {
        gaps_in     = 5,
        gaps_out    = 12,
        border_size = 2,

        col = {
            -- Turquoise/jade accent (active) → smoke grey (inactive). Sparingly.
            -- `$tz_accent $tz_accent_dim 45deg` becomes an explicit gradient.
            active_border   = { colors = { theme.accent, theme.accent_dim }, angle = 45 },
            inactive_border = theme.inactive,
        },

        layout           = "dwindle",
        resize_on_border = true,
        allow_tearing    = true, -- required for per-window tearing; opt-in via rules
    },

    decoration = {
        rounding = 12,

        -- Glass: translucent surfaces so the wallpaper/blur reads through.
        active_opacity   = 0.98,
        inactive_opacity = 0.92,

        blur = {
            enabled           = true,
            size              = 8,
            passes            = 3,
            new_optimizations = true,
            ignore_opacity    = true,
            xray              = false,
            noise             = 0.015,
            contrast          = 1.05,
            brightness        = 0.85, -- darken behind glass → obsidian, not milky
            vibrancy          = 0.15,
        },

        -- Soft smoke shadow, nothing hard-edged.
        shadow = {
            enabled        = true,
            range          = 24,
            render_power   = 3,
            color          = theme.shadow,
            color_inactive = theme.shadow_dim,
        },
    },

    -- Dwindle layout tuning.
    -- `pseudotile` was removed from the dwindle block in Hyprland 0.55 (pseudo is
    -- now driven only by the `pseudo` dispatcher / windowrule), so it's gone.
    dwindle = {
        preserve_split = true,
        smart_split    = false,
    },

    misc = {
        disable_hyprland_logo    = true,
        disable_splash_rendering = true,
        force_default_wallpaper  = 0,
        -- `misc.vfr` was removed in Hyprland 0.55 — VFR is now always-on
        -- internally, so there's no knob to set (idle frames are throttled
        -- automatically).
        focus_on_activate        = true,
    },
})

-- Shell-layer glass — blur the translucent bar / swaync / Walker surfaces so
-- they read as obsidian glass over the wallpaper. `ignore_alpha` keeps fully
-- transparent regions (padding around floating modules) from being blurred:
-- pixels below the threshold aren't blurred, which keeps the transparent margins
-- around floating modules crisp.
--
-- In Lua each layer gets ONE rule table carrying both effects, rather than the
-- two `layerrule =` lines per namespace that hyprlang needed. The namespace
-- patterns stay UNANCHORED, exactly as they were — `match:namespace tezca-bar`
-- was a substring match, and anchoring them here would quietly stop matching any
-- surface whose namespace carries a suffix.

-- tezca-bar — the obsidian-glass top strip. It draws a translucent background
-- and lets the compositor frost the wallpaper behind it.
hl.layer_rule({ name = "tezca-glass-bar",    match = { namespace = "tezca-bar" },
                blur = true, ignore_alpha = 0.2 })
hl.layer_rule({ name = "tezca-glass-swaync", match = { namespace = "swaync-control-center" },
                blur = true, ignore_alpha = 0.2 })
hl.layer_rule({ name = "tezca-glass-notify", match = { namespace = "swaync-notification-window" },
                blur = true, ignore_alpha = 0.2 })
hl.layer_rule({ name = "tezca-glass-walker", match = { namespace = "walker" },
                blur = true, ignore_alpha = 0.2 })
-- The autohiding dock and the wlogout power veil are obsidian glass too.
-- tezca-dock draws a translucent obsidian pill and relies on the compositor to
-- frost the wallpaper behind it.
hl.layer_rule({ name = "tezca-glass-dock",   match = { namespace = "tezca-dock" },
                blur = true, ignore_alpha = 0.2 })
hl.layer_rule({ name = "tezca-glass-logout", match = { namespace = "wlogout" },
                blur = true, ignore_alpha = 0.2 })
