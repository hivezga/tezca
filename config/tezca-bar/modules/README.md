# Custom bar modules

Drop a `<name>.toml` **manifest** in this folder to add your own module to
`tezca-bar` — a widget driven by a shell command, in the spirit of Waybar's
`custom/*`. One module per file, so a module is a single shareable file.

A module does nothing until you **place** it: add `custom:<name>` to a layout
region, either in `~/.config/tezca-bar/config.toml`:

```toml
layout_right = cpu, mem, sep, custom:weather, sep, clock, power
```

…or from **Settings ▸ Bar ▸ Modules** (discovered modules appear in the *Add*
menu). Then `tezca bar restart`.

## Manifest keys

| key              | required | meaning                                                   |
|------------------|----------|-----------------------------------------------------------|
| `exec`           | ✔        | shell command; its stdout drives the widget               |
| `interval`       |          | seconds between runs (default `10`, min `1`)              |
| `label`          |          | display name in Settings (default: prettified file name)  |
| `icon`           |          | static leading glyph/text                                 |
| `tooltip`        |          | static hover text (a script may override it)              |
| `on_click`       |          | shell command run on left-click                           |
| `on_right_click` |          | shell command run on right-click                          |

Values may be wrapped in matching quotes; interior quotes are preserved, so
`exec = echo "hi"` works as written.

## Output protocol

The `exec` command prints EITHER:

- **plain text** — the first non-empty line becomes the widget text; or
- **JSON** — `{"text": "…", "tooltip": "…", "class": "…"}`

`class` adds a CSS class to the widget so a theme (or the script itself) can
recolour it — e.g. printing `"class": "cold"` matches `.custom.cold` in the
bar's CSS. The built-in `warn` and `urgent`/`crit` classes give you the bar's
standard gold / red language for free. `class` may also be an array of strings.

Empty output hides the widget until it has something to show.

### Example — a weather module (JSON)

`weather.toml`:

```toml
label = Weather
icon = 
interval = 900
exec = ~/.config/tezca-bar/modules/weather.sh
on_click = xdg-open https://wttr.in
```

`weather.sh` (make it executable):

```sh
#!/bin/sh
temp=$(curl -sf "https://wttr.in/?format=%t" || echo "?")
printf '{"text":"%s","tooltip":"wttr.in","class":"cold"}\n' "$temp"
```

## Trust & privacy

A custom module runs a command **you** placed here, with your privileges —
exactly like a script referenced from a Waybar config. Tezca adds no sandbox and
makes no network request of its own for these; a module does whatever its `exec`
does. Only add modules you trust, same as any shell script.

## Debugging

```sh
tezca-bar --custom-dump
```

discovers every manifest, runs each `exec` once, and prints what it emits
(text / tooltip / class) — without opening a window or touching a running bar.
`example-clock.toml` in this folder is a working starting point.
