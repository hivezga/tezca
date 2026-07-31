-- conf.d/input.lua — keyboard, pointer, gestures.

hl.config({
    input = {
        kb_layout          = "us",
        follow_mouse       = 1,
        sensitivity        = 0,      -- -1.0 … 1.0, 0 = raw libinput (flat)
        accel_profile      = "flat", -- predictable pointer for gaming/precision
        numlock_by_default = true,

        touchpad = {
            natural_scroll       = true,
            disable_while_typing = true,
            -- Was `tap-to-click` in hyprlang. The Lua schema spells every option
            -- with underscores, and a hyphenated key is silently ignored rather
            -- than rejected — so this one has to be renamed, not transcribed.
            tap_to_click         = true,
        },

        -- Faster key repeat for a snappier feel.
        repeat_rate  = 40,
        repeat_delay = 300,
    },
})

-- Touchpad gestures. Hyprland 0.55 replaced the old `gestures { workspace_swipe }`
-- block with the `gesture` keyword; in Lua that is `hl.gesture`.
hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })

-- Obsidian modifier — SUPER mirrors macOS ⌘ (DESIGN.md §12).
--
-- Was `$mod = SUPER`. Lua has no config-global variables, so the modifier is
-- exported from this module and pulled in by conf.d/keybinds.lua, which keeps
-- it declared where it always was.
return { mod = "SUPER" }
