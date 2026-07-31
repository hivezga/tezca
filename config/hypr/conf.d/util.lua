-- conf.d/util.lua — shared helpers for the Lua config (DESIGN.md §5).
--
-- WHY THIS EXISTS AT ALL
--
-- hyprlang tolerated a missing `source =` target: it logged an error and carried
-- on. Lua does not. An uncaught error anywhere in hyprland.lua aborts the whole
-- config, and Hyprland falls back to EMERGENCY MODE — literally one keybind
-- (SUPER+Q) and no rules, on a black desktop. Every optional file we touch
-- (theme colors, user rebinds, generated overrides) is therefore loaded through
-- `M.load`, which degrades to nil instead of taking the session down with it.

local M = {}

--- The absolute path of `rel` under $XDG_CONFIG_HOME (default ~/.config).
--- @param rel string  e.g. "tezca/overrides.lua"
--- @return string
function M.config(rel)
    local base = os.getenv("XDG_CONFIG_HOME")
    if base == nil or base == "" then
        base = (os.getenv("HOME") or "") .. "/.config"
    end
    return base .. "/" .. rel
end

--- `dofile` that never raises.
---
--- Returns the file's value, or nil when it is absent, unreadable, contains a
--- syntax error, or throws while running. A *missing* file is silent (optional
--- files are missing by design on a fresh install); a file that exists but is
--- broken warns on stderr, because that one is a bug worth seeing in the log.
--- @param path string
--- @return any|nil
function M.load(path)
    local chunk, err = loadfile(path)
    if chunk == nil then
        -- Distinguish "not there" from "there but broken" — only the latter is
        -- worth a log line. loadfile's message is "cannot open <path>" for the
        -- former, which we do not want spamming every reload.
        if err ~= nil and not err:match("cannot open") then
            io.stderr:write("tezca: skipping malformed " .. path .. ": " .. err .. "\n")
        end
        return nil
    end
    local ok, result = pcall(chunk)
    if not ok then
        io.stderr:write("tezca: error running " .. path .. ": " .. tostring(result) .. "\n")
        return nil
    end
    return result
end

--- Expand a flat option-path table into the nested shape `hl.config` wants.
---
---   { ["decoration:blur:size"] = 8 }  ->  { decoration = { blur = { size = 8 } } }
---
--- Both `:` and `.` separate segments, because Hyprland's own option paths mix
--- them: `hyprctl getoption general:col.active_border` is one key with one of
--- each. Accepting both is what lets ~/.config/tezca/overrides.lua store keys in
--- exactly the form `tezca hypr set`, `hyprctl getoption` and the Settings
--- sliders already speak — no translation on either side of the boundary, so
--- there is no round-trip to get wrong.
--- @param flat table<string, any>
--- @return table
function M.nest(flat)
    local out = {}
    for dotted, value in pairs(flat) do
        -- Walk one segment BEHIND the iterator: `pending` is the segment whose
        -- container we do not yet know is a leaf or a branch. It becomes a
        -- branch when another segment follows, and the leaf when none does.
        local node, pending = out, nil
        for segment in dotted:gmatch("[^.:]+") do
            if pending ~= nil then
                if type(node[pending]) ~= "table" then
                    node[pending] = {}
                end
                node = node[pending]
            end
            pending = segment
        end
        if pending ~= nil then
            node[pending] = value
        end
    end
    return out
end

--- Apply a flat dotted-key override table: config keys through `hl.config`,
--- and the `monitors` array (owned by `tezca display`) through `hl.monitor`.
---
--- Anything the compositor rejects is reported and skipped rather than allowed
--- to abort the config — a stale key left behind by a Hyprland upgrade must not
--- cost you your session. That is the whole point of routing generated state
--- through here instead of splicing it into the config as raw code.
--- @param overrides table|nil
function M.apply_overrides(overrides)
    if type(overrides) ~= "table" then
        return
    end

    for _, spec in ipairs(overrides.monitors or {}) do
        local ok, err = pcall(hl.monitor, spec)
        if not ok then
            io.stderr:write("tezca: bad monitor override for "
                .. tostring(spec.output) .. ": " .. tostring(err) .. "\n")
        end
    end

    local flat = {}
    local any = false
    for key, value in pairs(overrides) do
        if key ~= "monitors" then
            flat[key] = value
            any = true
        end
    end
    if not any then
        return
    end

    local ok, err = pcall(hl.config, M.nest(flat))
    if not ok then
        -- One bad key would otherwise discard every override in the table, so
        -- fall back to applying them one at a time and drop only the offender.
        io.stderr:write("tezca: override batch rejected (" .. tostring(err)
            .. "); retrying individually\n")
        for key, value in pairs(flat) do
            local okOne, errOne = pcall(hl.config, M.nest({ [key] = value }))
            if not okOne then
                io.stderr:write("tezca: dropping override " .. key
                    .. ": " .. tostring(errOne) .. "\n")
            end
        end
    end
end

return M
