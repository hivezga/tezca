-- Project:Tezca — Hyprland border/shadow palette (matugen template).
--
-- Rendered by matugen into ~/.config/tezca/current/colors-hypr.lua and loaded at
-- the top of conf.d/decoration.lua, which feeds these fields into the general.col
-- border and decoration.shadow blocks. Swapping the wallpaper recolors the window
-- borders live on `hyprctl reload`.
--
-- This is a Lua DATA file: it must stay a bare `return { ... }` table with string
-- values and no logic. decoration.lua loads it through util.load, so a matugen
-- run that emits a syntax error degrades to the built-in obsidian fallback
-- instead of erroring the config and dropping the session into emergency mode.

return {
    accent         = "rgba({{colors.primary.default.hex_stripped}}ff)",
    accent_dim     = "rgba({{colors.primary_container.default.hex_stripped}}ff)",
    inactive       = "rgba({{colors.surface_container_high.default.hex_stripped}}aa)",
    shadow         = "rgba(00000055)",
    shadow_dim     = "rgba(00000033)",
}
