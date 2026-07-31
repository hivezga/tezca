-- conf.d/ai.lua — the AI workspace + scratch surfaces (DESIGN.md §11).
--
-- Tezca is an AI workstation, so AI tools get first-class placement:
--   * the Claude desktop app is pinned to the semantic "ai/chat" workspace (3),
--   * a drop-down AI terminal (Claude Code) lives on the special `ai` workspace,
--   * a floating quick-capture note window for fleeting thoughts.
-- The launchers themselves are bound in conf.d/keybinds.lua (SUPER+A/C/N).
--
-- Lua rule shape — see conf.d/windowrules.lua. Percentage sizes are written as
-- monitor expressions there and here for the same reason.

-- --- Claude desktop app → AI workspace ------------------------------------
-- StartupWMClass is `com.anthropic.Claude` (from its .desktop). Pin it to ws3
-- on the ultrawide (silent = don't yank focus there when it opens elsewhere).
hl.window_rule({
    match     = { class = "^(com\\.anthropic\\.Claude)$" },
    workspace = "3 silent",
})

-- --- AI scratchpad (drop-down Claude Code terminal) -----------------------
-- SUPER+A toggles the special `ai` workspace; SUPER+SHIFT+A spawns an Alacritty
-- running `claude` into it (class tezca-ai). Floating + centered = Spotlight-y
-- drop-down over whatever you're doing.
hl.window_rule({
    match     = { class = "^(tezca-ai)$" },
    float     = true,
    size      = { "monitor_w * 0.62", "monitor_h * 0.72" },
    center    = true,
    workspace = "special:ai",
})

-- --- Quick-capture note window --------------------------------------------
-- SUPER+N drops a small floating editor on ~/Documents/tezca-notes.md. Stays on
-- the current workspace (it's a fleeting capture, not a place you live).
hl.window_rule({
    match  = { class = "^(tezca-note)$" },
    float  = true,
    size   = { "monitor_w * 0.46", "monitor_h * 0.52" },
    center = true,
    pin    = true,
})
