-- conf.d/animations.lua — smooth but restrained. 165 Hz means motion should
-- feel instant, not sluggish. Tuned, not maxed (DESIGN.md §4).
--
-- Phase 4 polish: shorter, more decisive window/workspace timings; explicit
-- fade curves; and a `layers` animation so the shell surfaces (swaync, Walker,
-- and the autohiding dock) glide in as glass instead of snapping.
--
-- hyprlang packed a curve into four bare numbers (`bezier = name, x1,y1,x2,y2`);
-- Lua takes them as the two control POINTS they always were.

hl.config({
    animations = {
        enabled = true,
    },
})

-- Custom beziers — a gentle overshoot for the "glass settling" feel.
hl.curve("smoke",    { type = "bezier", points = { { 0.25, 0.10 }, { 0.25, 1.00 } } }) -- ease-out, for fades
hl.curve("obsidian", { type = "bezier", points = { { 0.05, 0.90 }, { 0.10, 1.05 } } }) -- settle with a hair of overshoot
hl.curve("snappy",   { type = "bezier", points = { { 0.20, 1.00 }, { 0.30, 1.00 } } }) -- quick, no overshoot — workspaces
hl.curve("glass",    { type = "bezier", points = { { 0.16, 1.00 }, { 0.30, 1.00 } } }) -- layers/dock slide-and-settle

-- Windows — pop in from near their final size, out a touch faster.
hl.animation({ leaf = "windows",     enabled = true, speed = 3.2, bezier = "obsidian", style = "popin 6%" })
hl.animation({ leaf = "windowsIn",   enabled = true, speed = 3.2, bezier = "obsidian", style = "popin 6%" })
hl.animation({ leaf = "windowsOut",  enabled = true, speed = 2.6, bezier = "smoke",    style = "popin 6%" })
hl.animation({ leaf = "windowsMove", enabled = true, speed = 3.0, bezier = "snappy" })

-- Active-window accent border (breathing gradient sweep).
hl.animation({ leaf = "border",      enabled = true, speed = 6.0,  bezier = "default" })
hl.animation({ leaf = "borderangle", enabled = true, speed = 30,   bezier = "default", style = "loop" })

-- Fades.
hl.animation({ leaf = "fade",    enabled = true, speed = 3.0, bezier = "smoke" })
hl.animation({ leaf = "fadeIn",  enabled = true, speed = 3.0, bezier = "smoke" })
hl.animation({ leaf = "fadeOut", enabled = true, speed = 2.4, bezier = "smoke" })
hl.animation({ leaf = "fadeDim", enabled = true, speed = 3.0, bezier = "smoke" })

-- Shell layers — swaync / Walker / the dock glide in from their edge.
hl.animation({ leaf = "layers",    enabled = true, speed = 3.0, bezier = "glass", style = "slide" })
hl.animation({ leaf = "layersIn",  enabled = true, speed = 3.0, bezier = "glass", style = "slide" })
hl.animation({ leaf = "layersOut", enabled = true, speed = 2.4, bezier = "smoke", style = "slide" })

-- Workspaces.
hl.animation({ leaf = "workspaces",       enabled = true, speed = 3.6, bezier = "snappy",   style = "slide" })
hl.animation({ leaf = "specialWorkspace", enabled = true, speed = 4.0, bezier = "obsidian", style = "slidevert" })
