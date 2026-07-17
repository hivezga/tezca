# Wallpaper credits

The Tezca source, configs, and themes are MIT-licensed (see `../LICENSE`). The
wallpapers bundled here are **not** — they carry their own terms, listed below.

| File | Theme | Source / author | Terms |
|---|---|---|---|
| `obsidian-teal.jpg` | `obsidian` (signature) | Wallhaven — [`wallhaven-3l85q3`](https://wallhaven.cc/w/3l85q3) | © original artist. Included for personal use / preview only — **not** re-licensed under Tezca's MIT. |
| `smoke-light.jpg` | `smoke` (light) | Generated for Tezca (ImageMagick gradient) | MIT, same as the project. Free to use/modify. |

## If you're redistributing or forking

`obsidian-teal.jpg` is a third-party image kept as the signature preview. If you
publish a fork, or you simply want a clean-room default, either:

- swap it for your own image and repoint `themes/obsidian/theme.meta`'s
  `wallpaper =` line, or
- run `tezca theme wallpaper <your-image>` for a dynamic palette from any picture, or
- use the bundled MIT `smoke-light.jpg` via `tezca theme set smoke`.

Nothing in the desktop hard-codes a wallpaper path — the active one lives in
`~/.config/tezca/current/wallpaper` and is set entirely through `tezca theme`.
