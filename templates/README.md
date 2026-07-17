# templates/ — the Tezca theme engine

A single wallpaper drives the whole desktop's color. These are the **matugen
templates**: matugen extracts a Material-You palette from an image, renders each
template, and writes the result into `~/.config/tezca/current/`, where every
component picks it up.

```
        wallpaper.jpg
             │  matugen (Material-You extraction)
             ▼
  ~/.config/tezca/current/
    ├── colors.css            → @imported by Waybar, swaync, Walker (GTK CSS)
    ├── colors-kitty.conf     → included by kitty.conf
    ├── colors-hypr.conf      → sourced by hypr/conf.d/decoration.conf (borders)
    ├── colors-hyprlock.conf  → sourced by hypr/hyprlock.conf (+ wallpaper path)
    ├── wallpaper             → one line: the active wallpaper's absolute path
    └── theme.state           → the active theme name / source
```

## The two modes

- **Dynamic** — `tezca theme wallpaper <img>`: matugen renders these templates
  from the image. Effortless variety from any picture.
- **Curated** — `tezca theme set <name>`: copies a hand-tuned palette from
  `themes/<name>/` verbatim (matugen not involved), pinning an exact look. The
  signature is `themes/obsidian/`.

Either way the `tezca` CLI then points every component's stable import at the
new files and sends each one its live-reload signal (Waybar SIGUSR2, swaync
`--reload-css`, `hyprctl reload`, kitty SIGUSR1, awww wallpaper) — no restarts.

## Color tokens

Components reference only these tokens (and `alpha()`/opacity of them), never
raw hexes. Keep the names identical across `templates/` and every `themes/*/`.

| token           | dynamic → Material-You role  | obsidian literal |
|-----------------|------------------------------|------------------|
| `tz_base`       | `surface_container_lowest`   | `#0B0E0F`        |
| `tz_surface`    | `surface_container`          | `#14191B`        |
| `tz_text`       | `on_surface`                 | `#E8EAED`        |
| `tz_subtext`    | `on_surface_variant`         | `#C3C8CC`        |
| `tz_muted`      | `outline`                    | `#8B9398`        |
| `tz_faint`      | `surface_bright`             | `#5A6166`        |
| `tz_accent`     | `primary`                    | `#3FB8AF`        |
| `tz_accent_dim` | `primary_container`          | `#2A8C86`        |
| `tz_on_accent`  | `on_primary`                 | `#0B0E0F`        |
| `tz_gold`       | fixed brand constant         | `#C9A24B`        |
| `tz_urgent`     | `error`                      | `#E06C75`        |

`tz_gold` stays a fixed brand constant in both modes — it's the obsidian-gold
signature, used sparingly (power glyph, caps-lock), and shouldn't chase the
wallpaper. matugen is invoked with `--prefer saturation -m dark` so it runs
non-interactively and pulls the most vivid accent from dark obsidian imagery.

## Template syntax (matugen 4.x)

`{{colors.<role>.default.hex}}` → `#84d2e6`, `.hex_stripped` → `84d2e6` (for
Hyprland's `rgba(RRGGBBAA)`), `.rgba` → `rgba(r, g, b, a)`. Anything not in
`{{ }}` is literal, which is how the fixed ANSI/gold values pass through.
