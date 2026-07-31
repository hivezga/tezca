-- conf.d/monitors.lua — dual 165 Hz layout.
--
--   Primary : 3440x1440@165  ultrawide  (left,  at 0x0)
--   Second  : 2560x1440@165             (right, immediately after the ultrawide)
--
-- Connector names verified against `hyprctl monitors` on this machine:
--   DP-1 = Xiaomi 3440x1440 ultrawide (primary).  Panel also does 180 Hz —
--          bump the rate below if you want it (GPU/cables permitting). Kept at
--          165 to match the design's "dual-165" target and pair with DP-3.
--   DP-3 = LG UltraGear 2560x1440 (max 164.96 Hz).
-- On different hardware, run `hyprctl monitors` and adjust (or use local.lua).

hl.monitor({ output = "DP-1", mode = "3440x1440@165", position = "0x0",    scale = 1 })
hl.monitor({ output = "DP-3", mode = "2560x1440@165", position = "3440x0", scale = 1 })

-- Fallback: any monitor not matched above comes up at its preferred mode so a
-- cable swap or a new display never leaves you with a black screen.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })

-- VRR: 2 = fullscreen-only. Safest for a mixed desktop/gaming setup on 165 Hz —
-- avoids flicker on the desktop while still giving games adaptive sync.
hl.config({
    misc = {
        vrr = 2,
    },
})

-- Per-monitor workspaces (semantic sets, DESIGN.md §11).
-- Odd workspaces live on the Xiaomi ultrawide (DP-1), even on the LG (DP-3).
-- Keep this in step with tezca-bar's `workspaces.DP-1 = odd` / `.DP-3 = even`
-- so each bar's pills switch to workspaces that actually live on that monitor.
--   ws3 hosts the Claude desktop app (conf.d/ai.lua) and ws5 is where games are
--   auto-moved fullscreen (conf.d/gaming.lua) — both odd, so both on DP-1.
hl.workspace_rule({ workspace = "1",  monitor = "DP-1", default = true })
hl.workspace_rule({ workspace = "3",  monitor = "DP-1" })
hl.workspace_rule({ workspace = "5",  monitor = "DP-1" })
hl.workspace_rule({ workspace = "7",  monitor = "DP-1" })
hl.workspace_rule({ workspace = "9",  monitor = "DP-1" })
hl.workspace_rule({ workspace = "2",  monitor = "DP-3", default = true })
hl.workspace_rule({ workspace = "4",  monitor = "DP-3" })
hl.workspace_rule({ workspace = "6",  monitor = "DP-3" })
hl.workspace_rule({ workspace = "8",  monitor = "DP-3" })
hl.workspace_rule({ workspace = "10", monitor = "DP-3" })
