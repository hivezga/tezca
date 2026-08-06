repo: hivezga/tezca
branch: main
path: crates/tezca-settings, crates/tezca-bar, config/tezca-settings, config/tezca-bar, config/tezca-dock, themes, wallpapers

## Last sync
date: 2026-08-02T07:05:00Z

### Updated in this project
- Added a third template, `templates/tezca-shell`: dock, launcher, theme picker, notification toasts, lock screen and session dialog — each reconciled against its config in `config/`.
- Dock magnification reproduces `magnifier.rs` exactly — cos² falloff, influence 110px, max_scale 1.6, label above 1.15×, 6px hotspot.
- Dock item model follows `apps.rs`: pinned in dock.toml order, then running-unpinned behind a divider; labels are the `.desktop` Name= verbatim.
- Added the light `smoke` theme alongside obsidian and cyber — tokens verbatim from themes/smoke/colors.css.
- Read `crates/tezca-bar` and built a second template: the redesigned bar, its popovers, the OSD and a state gallery.
- Enriched every module with the data its popover already computes — CPU temp beside utilisation, GPU watts, battery time-to-empty, BT device battery, AI window reset time.
- Four strategies for the 21-slot right cluster (show all / grouped / hover-reveal / priority tiers), switchable in Tweaks.
- Mayan workspace numerals drawn as geometry instead of the Unicode Mayan block, so they need no font.
- Copied `wallpapers/obsidian-teal.jpg` and `smoke-light.jpg`; Settings' wallpaper picker now renders the real files.

## Screen map
| Screen | Repo files |
| --- | --- |
| Settings shell / nav / palette | crates/tezca-settings/src/main.rs, config/tezca-settings/style.css |
| Settings ▸ Displays | crates/tezca-settings/src/arrange.rs, crates/tezca-settings/src/pages.rs, crates/tezca-settings/src/backend.rs |
| Settings ▸ Appearance, Bar, Dock | crates/tezca-settings/src/pages.rs, config/tezca-bar/config.toml, config/tezca-dock/dock.toml, themes/*/colors.css |
| Settings ▸ Sound, Input, Network, Power | crates/tezca-settings/src/pages.rs, crates/tezca-settings/src/backend.rs |
| Settings ▸ Startup, Keybinds, Gaming, System | crates/tezca-settings/src/pages.rs, crates/tezca-settings/src/keybinds.rs |
| Bar strip + modules | crates/tezca-bar/src/bar.rs, crates/tezca-bar/src/bar.css, crates/tezca-bar/src/config.rs |
| Bar popovers | crates/tezca-bar/src/popovers.rs, crates/tezca-bar/src/sysinfo.rs, crates/tezca-bar/src/ai.rs, crates/tezca-bar/src/bluetooth.rs |
| Bar OSD | crates/tezca-bar/src/osd.rs |
| Dock | crates/tezca-dock/src/{magnifier,config,apps,hypr}.rs, config/tezca-dock/dock.toml, docs/screenshots/desktop.jpg |
| Launcher + theme picker | config/walker/{config.toml,themes/tezca/style.css,themes/tezca/layout.xml} + docs/screenshots/launcher.jpg (flat 600px card, F1–F4 chips, keybind-pair footer); theme swatches from themes/*/colors.css |
| Notification toasts | config/swaync/config.json (400px, 48px icon, 8/4/0 timeouts) + config/swaync/style.css (r14 chrome, summary/time/body scale, accent-tinted actions, close button) |
| Lock screen + session | config/hypr/hyprlock.conf (92px clock, Tezca wordmark, 340×56 input); config/wlogout/style.css (4 tiles, r20, gold shutdown); palette from themes/*/colors-hyprlock.conf |
| Privacy indicators | crates/tezca-bar/src/camera.rs, crates/tezca-bar/src/mic.rs, crates/tezca-bar/src/session.rs |

## Sync history
- 2026-08-02T04:22:31Z — picked up `arrange.rs`; rebuilt Displays as its own page with the drag-to-arrange canvas, layout profiles and confirm-or-revert.
- 2026-08-02T03:55:27Z — initial read of the GTK4 control center; built the redesigned settings prototype (3 nav groups, ⌘K palette, CLI echo, three visual directions).
</content>
</invoke>
<invoke name="check_design_system">
</invoke>
