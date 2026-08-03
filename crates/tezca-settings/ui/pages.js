/* The twelve pages, their sections and their controls.
 *
 * The inventory — which control lives where, what it is called, and what it
 * runs — is carried over from the GTK build's `pages.rs` rather than
 * re-derived, per START_HERE §5. The *commands* are the real ones the `tezca`
 * CLI actually accepts, which is not what the prototype's CMD map says: that
 * map names `tezca input layout`, `tezca power profile`, `tezca desktop set`
 * and `tezca service bar`, none of which exist. The echo footer is a teaching
 * device, so it has to print something you could paste into a shell.
 *
 * Every control applies on change and echoes; there is no Apply button. That is
 * the design's semantics and it is also what makes the footer worth reading.
 */

import {
    el, row, section, toggle, segmented, slider, field, themeCard, icon,
    asBool, asNum, kv, records, pairs, secsLabel, convertFileSrc,
} from './lib.js';
import { displaysPage } from './displays.js';

/* Sidebar icons — 24px viewBox, 1.7px stroke, per the design. */
const ICONS = {
    appearance: ['M12 3.2a8.8 8.8 0 1 0 0 17.6c1.3 0 2-.8 2-1.8 0-1.4-1.2-1.7-1.2-2.8 0-.8.7-1.4 1.6-1.4h1.9A4.9 4.9 0 0 0 21 9.8C21 6 17 3.2 12 3.2z', 'M7.5 12.2h.01M10 8.2h.01M14.5 7.7h.01M17.5 11h.01'],
    bar: ['M3.5 6.5h17M3.5 12h17M3.5 17.5h9'],
    dock: ['M4 14.5h3.5v3.5H4zM10.25 14.5h3.5v3.5h-3.5zM16.5 14.5H20v3.5h-3.5zM4 8h3.5v3.5H4zM10.25 8h3.5v3.5h-3.5zM16.5 8H20v3.5h-3.5z'],
    displays: ['M3 5.5h18v10.5H3z', 'M8.5 20h7M12 16v4'],
    sound: ['M11 5 6.5 9H3v6h3.5L11 19z', 'M15 9.8a3.2 3.2 0 0 1 0 4.4M17.6 7.4a6.8 6.8 0 0 1 0 9.2'],
    input: ['M3 6.5h18v11H3z', 'M7 10h.01M11 10h.01M15 10h.01M8 14h8'],
    network: ['M4.5 10.5a11 11 0 0 1 15 0M8 14a6 6 0 0 1 8 0M12 17.6h.01'],
    power: ['M2 7.5h15v9H2zM19.5 10.5v3', 'M4 9.5h7v5H4z'],
    startup: ['M12 3.5v9', 'M7.5 6.6a7.5 7.5 0 1 0 9 0'],
    keybinds: ['M3 6.5h18v11H3z', 'M6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M8 14h8'],
    gaming: ['M7.5 8h9a4.5 4.5 0 0 1 4.4 5.4l-.6 3A2.6 2.6 0 0 1 15.9 17l-1.4-2h-5l-1.4 2a2.6 2.6 0 0 1-4.4-.6l-.6-3A4.5 4.5 0 0 1 7.5 8z', 'M7 11.5h2.5M8.25 10.25v2.5M15 11h.01M17 12.5h.01'],
    system: ['M12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4z', 'M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-2.7 1.1v.3a2 2 0 1 1-4 0v-.2a1.6 1.6 0 0 0-2.8-1.1l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1A1.6 1.6 0 0 0 3.5 14h-.3a2 2 0 1 1 0-4h.2a1.6 1.6 0 0 0 1.1-2.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5v-.3a2 2 0 1 1 4 0v.2a1.6 1.6 0 0 0 2.7 1.1l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1h.3a2 2 0 1 1 0 4h-.2a1.6 1.6 0 0 0-1.4 1z'],
};

export const GROUPS = [
    ['Look & feel', ['appearance', 'bar', 'dock']],
    ['Devices', ['displays', 'sound', 'input', 'network', 'power']],
    ['System', ['startup', 'keybinds', 'gaming', 'system']],
];

export const LABELS = {
    appearance: 'Appearance', bar: 'Bar', dock: 'Dock', displays: 'Displays',
    sound: 'Sound', input: 'Input', network: 'Network', power: 'Power',
    startup: 'Startup', keybinds: 'Keybinds', gaming: 'Gaming', system: 'System',
};

export const navIcon = (id) => icon(ICONS[id] || ICONS.system);

/* ── Appearance ──────────────────────────────────────────────────────────── */

async function appearance(ctx) {
    const out = [el('h1.pagetitle', 'Appearance')];
    const [wall, hypr] = await Promise.all([
        ctx.invoke('tz_wallpapers'),
        ctx.hypr([
            'general:gaps_in', 'general:gaps_out', 'general:border_size',
            'decoration:rounding', 'decoration:active_opacity', 'decoration:inactive_opacity',
            'decoration:blur:enabled', 'decoration:blur:size', 'decoration:blur:passes',
            'decoration:shadow:enabled', 'animations:enabled',
        ]),
    ]);

    out.push(section('Theme'));
    const grid = el('div.themegrid');
    for (const t of ctx.themes) {
        grid.append(themeCard(t, t.active, (name) => ctx.run(['theme', 'set', name], { reload: true })));
    }
    out.push(el('div.row.stack', el('div.row-control', grid)));
    out.push(
        row(
            'Derive the palette from the wallpaper',
            'Off pins the curated theme; on re-derives every colour when the picture changes.',
            toggle(ctx.derive, (v) => ctx.run(['theme', 'derive', v ? 'on' : 'off'])),
        ),
    );

    out.push(section('Wallpaper'));
    const preview = el('div.wallpreview');
    const fitCss = { fill: 'cover', fit: 'contain', stretch: '100% 100%', center: 'auto' };
    const paint = (path, fit) => {
        preview.style.backgroundImage = path ? `url("${convertFileSrc(path)}")` : 'none';
        preview.style.backgroundSize = fitCss[fit] || 'cover';
        preview.style.backgroundRepeat = fit === 'tile' ? 'repeat' : 'no-repeat';
    };
    let fit = wall.fit;
    paint(wall.current, fit);
    out.push(el('div.row.stack', el('div.row-control', preview)));
    out.push(
        row('Fit', 'How the picture is scaled onto each output.',
            segmented(
                [['fill'], ['fit'], ['stretch'], ['center']].map(([v]) => [v, v]),
                fit,
                (v) => {
                    fit = v;
                    paint(wall.current, v);
                    ctx.run(['wallpaper', 'fit', v]);
                },
            )),
    );
    if (wall.library.length) {
        const cards = el('div.wallgrid');
        for (const p of wall.library) {
            const c = el('div.wallcard', {
                style: `background-image:url("${convertFileSrc(p)}")`,
                title: p.split('/').pop(),
            });
            c.classList.toggle('on', p === wall.current);
            c.addEventListener('click', () => {
                for (const o of cards.children) o.classList.remove('on');
                c.classList.add('on');
                paint(p, fit);
                ctx.run(['theme', 'wallpaper', p], { reload: true });
            });
            cards.append(c);
        }
        out.push(el('div.row.stack', el('div.row-control', cards)));
    }

    out.push(section('Windows & motion'));
    const h = (opt, v) => ctx.run(['hypr', 'set', opt, String(v)]);
    const hv = (opt, d) => hypr.get(opt) ?? d;
    out.push(row('Inner gaps', 'Space between tiled windows.',
        slider({ min: 0, max: 40, value: asNum(hv('general:gaps_in'), 5), unit: 'px' }, (v) => h('general:gaps_in', v))));
    out.push(row('Outer gaps', 'Space between the tiling area and the screen edge.',
        slider({ min: 0, max: 60, value: asNum(hv('general:gaps_out'), 10), unit: 'px' }, (v) => h('general:gaps_out', v))));
    out.push(row('Border size', null,
        slider({ min: 0, max: 8, value: asNum(hv('general:border_size'), 2), unit: 'px' }, (v) => h('general:border_size', v))));
    out.push(row('Corner rounding', null,
        slider({ min: 0, max: 28, value: asNum(hv('decoration:rounding'), 12), unit: 'px' }, (v) => h('decoration:rounding', v))));
    out.push(row('Active opacity', null,
        slider({
            min: 50, max: 100, value: Math.round(asNum(hv('decoration:active_opacity'), 1) * 100),
            format: (v) => `${v} %`,
        }, (v) => h('decoration:active_opacity', (v / 100).toFixed(2)))));
    out.push(row('Inactive opacity', null,
        slider({
            min: 30, max: 100, value: Math.round(asNum(hv('decoration:inactive_opacity'), 0.92) * 100),
            format: (v) => `${v} %`,
        }, (v) => h('decoration:inactive_opacity', (v / 100).toFixed(2)))));
    out.push(row('Blur', 'Translucent surfaces sample what is behind them.',
        toggle(asBool(hv('decoration:blur:enabled'), true), (v) => h('decoration:blur:enabled', v))));
    out.push(row('Blur size', null,
        slider({ min: 1, max: 20, value: asNum(hv('decoration:blur:size'), 8) }, (v) => h('decoration:blur:size', v))));
    out.push(row('Blur passes', 'More passes cost GPU time for a softer result.',
        slider({ min: 1, max: 5, value: asNum(hv('decoration:blur:passes'), 2) }, (v) => h('decoration:blur:passes', v))));
    out.push(row('Shadows', null,
        toggle(asBool(hv('decoration:shadow:enabled'), true), (v) => h('decoration:shadow:enabled', v))));
    out.push(row('Animations', null,
        toggle(asBool(hv('animations:enabled'), true), (v) => h('animations:enabled', v))));
    return out;
}

/* ── Bar ─────────────────────────────────────────────────────────────────── */

const BAR_MODULES = [
    'mirror', 'appname', 'appmenu', 'workspaces', 'nowplaying', 'privacy', 'caffeine',
    'night', 'ai', 'llm', 'tray', 'cpu', 'mem', 'gpu', 'net', 'bluetooth', 'volume',
    'brightness', 'battery', 'weather', 'notifications', 'clock', 'power', 'gamemode',
    'recording', 'camera', 'mic', 'submap', 'sep',
];

async function bar(ctx) {
    const cfg = pairs(await ctx.invoke('tz_bar_config'));
    const custom = await ctx.invoke('tz_custom_modules');
    const set = (k, v) => ctx.run(['bar', 'set', k, String(v)]);
    const g = (k, d) => cfg.get(k) ?? d;
    const out = [el('h1.pagetitle', 'Bar')];

    out.push(section('Shape'));
    out.push(row('Shape', 'Floating leaves a margin; edge runs the full width with one hairline.',
        segmented([['floating', 'floating'], ['edge', 'edge']], g('shape', 'floating'), (v) => set('shape', v))));
    out.push(row('Height', null,
        slider({ min: 24, max: 64, value: asNum(g('height'), 40), unit: 'px' }, (v) => set('height', v))));
    out.push(row('Top margin', 'Floating only.',
        slider({ min: 0, max: 24, value: asNum(g('margin_top'), 6), unit: 'px' }, (v) => set('margin_top', v))));
    out.push(row('Side margin', 'Floating only.',
        slider({ min: 0, max: 40, value: asNum(g('margin_side'), 10), unit: 'px' }, (v) => set('margin_side', v))));

    out.push(section('Clock'));
    out.push(row('Format', 'strftime-style — %a %d %b   %H:%M gives “Wed 22 Jul  16:59”. See man strftime.',
        field(g('clock_format', '%a %d %b   %H:%M'), (v) => set('clock_format', v))));
    out.push(row('Extra time zones', 'Comma separated, Label=Area/City or just Area/City. Empty hides the section.',
        field(g('clock_zones', ''), (v) => set('clock_zones', v), 'Berlin=Europe/Berlin, UTC')));

    out.push(section('Workspaces'));
    out.push(row('Numerals', 'Mayan draws bar-and-dot geometry, so it needs no font.',
        segmented([['arabic', 'arabic'], ['mayan', 'mayan']], g('workspace_numerals', 'arabic'), (v) => set('workspace_numerals', v))));
    out.push(row('Show only used workspaces', 'Hides empty pills, keeping the focused one.',
        toggle(asBool(g('workspace_hide_empty')), (v) => set('workspace_hide_empty', v ? 'true' : 'false'))));
    out.push(row('Compact', 'Packs occupied workspaces down to the lowest slots.',
        toggle(asBool(g('workspace_compact')), (v) => set('workspace_compact', v ? 'true' : 'false'))));

    out.push(section('Indicators'));
    out.push(row('Volume OSD', null,
        toggle(asBool(g('osd_enabled'), true), (v) => set('osd_enabled', v ? 'true' : 'false'))));
    out.push(row('OSD timeout', null,
        slider({ min: 400, max: 10000, step: 100, value: asNum(g('osd_timeout_ms'), 2600), unit: 'ms' }, (v) => set('osd_timeout_ms', v))));

    out.push(section('Metrics'));
    for (const [k, label] of [['cpu_interval', 'CPU poll'], ['mem_interval', 'Memory poll'], ['gpu_interval', 'GPU poll'], ['net_interval', 'Network poll']]) {
        out.push(row(label, null, slider({ min: 1, max: 30, value: asNum(g(k), 3), unit: 's' }, (v) => set(k, v))));
    }
    out.push(row('Compact below', 'Monitors narrower than this drop to the compact layout.',
        slider({ min: 1000, max: 5000, step: 20, value: asNum(g('compact_width'), 3000), unit: 'px' }, (v) => set('compact_width', v))));
    out.push(row('Right-cluster strategy', 'How a crowded right side is thinned out.',
        segmented([['all', 'show all'], ['group', 'grouped'], ['hover', 'hover reveal'], ['tiers', 'priority']],
            g('clutter', 'all'), (v) => set('clutter', v))));

    // Not in the prototype's Bar page, and kept anyway: the bar has had a
    // weather module since `weather.rs` landed, the GTK panel exposed it, and
    // dropping the only way to configure it would be a regression dressed up as
    // fidelity. README ▸ Proposals still lists weather as not existing.
    out.push(section('Weather'));
    out.push(row('Weather module', 'Off unless coordinates are set.',
        toggle(asBool(g('weather_enabled')), (v) => set('weather_enabled', v ? 'true' : 'false'))));
    out.push(row('Place', g('weather_place') ? null : 'Search for a town, then pick it below.',
        field(g('weather_place', ''), (v) => ctx.run(['bar', 'weather', 'search', v]), 'e.g. Guadalajara')));
    out.push(row('Coordinates', 'Set by the search, or type them as “lat lon”.',
        field(
            g('weather_lat') ? `${g('weather_lat')} ${g('weather_lon')}` : '',
            (v) => {
                const [lat, lon] = v.split(/[ ,]+/).filter(Boolean);
                if (lat && lon) ctx.run(['bar', 'weather', 'set', lat, lon]);
            },
            '19.43 -99.13',
        )));
    out.push(row('Units', null,
        segmented([['c', '°C'], ['f', '°F']], g('weather_unit', 'c'), (v) => set('weather_unit', v))));

    out.push(section('Local AI'));
    out.push(row('Local AI module', 'Shows the resident model, where it is running, and the live rate.',
        toggle(asBool(g('llm_enabled')), (v) => set('llm_enabled', v ? 'true' : 'false'))));
    out.push(row('Backend', null,
        segmented([['auto', 'auto'], ['ollama', 'ollama'], ['llamacpp', 'llama.cpp']],
            g('llm_backend', 'auto'), (v) => set('llm_backend', v))));
    out.push(row('Port', null,
        slider({ min: 1024, max: 65535, step: 1, value: asNum(g('llm_port'), 11434) }, (v) => set('llm_port', v))));

    out.push(section('Modules'));
    const known = BAR_MODULES.concat(custom.map(([id]) => id));
    for (const [key, label] of [['layout_left', 'Left'], ['layout_center', 'Center'], ['layout_right', 'Right']]) {
        const ids = String(g(key, '')).split(',').map((s) => s.trim()).filter(Boolean);
        out.push(row(`${label} · ${ids.length}`, null, moduleList(ids, known, (next) => set(key, next.join(','))), { stack: true }));
    }
    out.push(el('div.pagehint', 'Custom modules: drop a <name>.toml manifest in ~/.config/tezca-bar/modules/ and it appears here.'));
    return out;
}

/** A reorderable region list of drag chips (§1.6). */
function moduleList(ids, known, onChange) {
    const wrap = el('div.chips');
    let order = ids.slice();

    const render = () => {
        wrap.textContent = '';
        order.forEach((id, i) => {
            const isSep = id === 'sep';
            const chip = el(
                'div.chip' + (isSep ? '.sep' : '') + (id === 'llm' || id === 'ai' ? '.ai' : ''),
                { draggable: true, title: isSep ? 'separator' : id },
            );
            if (!isSep) chip.append(el('span.grip', '⠿'), el('span', id));
            chip.addEventListener('dragstart', (e) => {
                e.dataTransfer.setData('text/plain', String(i));
                chip.classList.add('dragging');
            });
            chip.addEventListener('dragend', () => chip.classList.remove('dragging'));
            chip.addEventListener('dragover', (e) => {
                e.preventDefault();
                chip.classList.add('over');
            });
            chip.addEventListener('dragleave', () => chip.classList.remove('over'));
            chip.addEventListener('drop', (e) => {
                e.preventDefault();
                chip.classList.remove('over');
                const from = Number(e.dataTransfer.getData('text/plain'));
                if (Number.isNaN(from) || from === i) return;
                const [moved] = order.splice(from, 1);
                order.splice(i, 0, moved);
                render();
                onChange(order);
            });
            chip.addEventListener('dblclick', () => {
                order.splice(i, 1);
                render();
                onChange(order);
            });
            wrap.append(chip);
        });
        const add = el('button.adddrop', { type: 'button' }, '+ add');
        add.addEventListener('click', () => {
            const pick = known.find((k) => !order.includes(k)) || 'sep';
            order.push(pick);
            render();
            onChange(order);
        });
        wrap.append(add);
    };
    render();
    return wrap;
}

/* ── Dock ────────────────────────────────────────────────────────────────── */

async function dock(ctx) {
    const cfg = pairs(await ctx.invoke('tz_dock_config'));
    const set = (k, v) => ctx.run(['dock', 'set', k, String(v)]);
    const g = (k, d) => cfg.get(k) ?? d;
    const out = [el('h1.pagetitle', 'Dock')];

    out.push(section('Size & physics'));
    out.push(row('Icon size', null, slider({ min: 32, max: 80, value: asNum(g('icon_size'), 48), unit: 'px' }, (v) => set('icon_size', v))));
    out.push(row('Magnification', 'How far the icon under the pointer grows.',
        slider({
            min: 100, max: 250, value: Math.round(asNum(g('max_scale'), 1.6) * 100),
            format: (v) => `${(v / 100).toFixed(2)}×`,
        }, (v) => set('max_scale', (v / 100).toFixed(2)))));
    out.push(row('Magnify radius', 'How far either side of the pointer the effect reaches.',
        slider({ min: 40, max: 220, value: asNum(g('influence'), 110), unit: 'px' }, (v) => set('influence', v))));
    out.push(row('Icon gap', null, slider({ min: 0, max: 32, value: asNum(g('gap'), 10), unit: 'px' }, (v) => set('gap', v))));
    out.push(row('Bottom margin', null, slider({ min: 0, max: 40, value: asNum(g('margin_bottom'), 8), unit: 'px' }, (v) => set('margin_bottom', v))));
    out.push(row('Hotspot height', 'The band at the screen edge that wakes the dock.',
        slider({ min: 1, max: 24, value: asNum(g('hotspot_height'), 6), unit: 'px' }, (v) => set('hotspot_height', v))));
    out.push(row('Autohide delay', null, slider({ min: 0, max: 2000, step: 50, value: asNum(g('hide_delay_ms'), 350), unit: 'ms' }, (v) => set('hide_delay_ms', v))));

    out.push(section('Pinned favourites'));
    const pinned = String(g('pinned', '')).split(',').map((s) => s.trim()).filter(Boolean);
    out.push(row(`${pinned.length} pinned`, 'Drag to reorder; double-click to unpin.',
        moduleList(pinned, [], (next) => set('pinned', next.join(','))), { stack: true }));
    return out;
}

/* ── Sound ───────────────────────────────────────────────────────────────── */

async function sound(ctx) {
    const st = kv(await ctx.read(['audio', 'status', '--machine']));
    const outs = records(await ctx.read(['audio', 'outputs', '--machine']));
    const ins = records(await ctx.read(['audio', 'inputs', '--machine']));
    const out = [el('h1.pagetitle', 'Sound')];

    out.push(section('Output'));
    if (outs.length) {
        const cur = st.get('output_name');
        out.push(row('Device', st.get('output') || null,
            segmented(outs.map((r) => [r.get('name') || r.get('@'), (r.get('description') || r.get('@') || '').slice(0, 22)]),
                cur, (v) => ctx.run(['audio', 'set-output', v]))));
    }
    out.push(row('Volume', null,
        slider({ min: 0, max: 100, value: asNum(st.get('output_volume'), 50), unit: '%' }, (v) => ctx.run(['audio', 'volume', String(v)]))));
    out.push(row('Muted', null,
        toggle(asBool(st.get('output_muted')), (v) => ctx.run(['audio', 'mute', v ? 'on' : 'off']))));

    out.push(section('Input'));
    if (ins.length) {
        out.push(row('Device', st.get('input') || null,
            segmented(ins.map((r) => [r.get('name') || r.get('@'), (r.get('description') || r.get('@') || '').slice(0, 22)]),
                st.get('input_name'), (v) => ctx.run(['audio', 'set-input', v]))));
    }
    out.push(row('Mic level', null,
        slider({ min: 0, max: 100, value: asNum(st.get('input_volume'), 50), unit: '%' }, (v) => ctx.run(['audio', 'volume', String(v), '--input']))));
    out.push(row('Mic muted', null,
        toggle(asBool(st.get('input_muted')), (v) => ctx.run(['audio', 'mute', v ? 'on' : 'off', '--input']))));
    return out;
}

/* ── Input ───────────────────────────────────────────────────────────────── */

async function input(ctx) {
    const o = await ctx.hypr([
        'input:kb_layout', 'input:kb_variant', 'input:kb_options', 'input:repeat_rate',
        'input:repeat_delay', 'input:numlock_by_default', 'input:sensitivity',
        'input:accel_profile', 'input:force_no_accel', 'input:follow_mouse',
        'input:touchpad:natural_scroll', 'input:touchpad:tap_to_click',
        'input:touchpad:disable_while_typing', 'input:touchpad:scroll_factor',
        'cursor:inactive_timeout', 'cursor:hide_on_key_press', 'cursor:no_hardware_cursors',
    ]);
    const h = (opt, v) => ctx.run(['hypr', 'set', opt, String(v)]);
    const g = (k, d) => o.get(k) ?? d;
    const out = [el('h1.pagetitle', 'Input')];

    out.push(section('Keyboard'));
    out.push(row('Layout', 'A layout you cannot type in is hard to undo from the GUI — the footer shows the command that reverts it.',
        field(g('input:kb_layout', 'us'), (v) => h('input:kb_layout', v))));
    out.push(row('Variant', null, field(g('input:kb_variant', ''), (v) => h('input:kb_variant', v), 'e.g. dvorak')));
    out.push(row('Options', null, field(g('input:kb_options', ''), (v) => h('input:kb_options', v), 'e.g. caps:escape')));
    out.push(row('Repeat rate', null, slider({ min: 1, max: 100, value: asNum(g('input:repeat_rate'), 35), unit: '/s' }, (v) => h('input:repeat_rate', v))));
    out.push(row('Repeat delay', null, slider({ min: 100, max: 2000, step: 25, value: asNum(g('input:repeat_delay'), 250), unit: 'ms' }, (v) => h('input:repeat_delay', v))));
    out.push(row('Num Lock at login', null, toggle(asBool(g('input:numlock_by_default')), (v) => h('input:numlock_by_default', v))));

    out.push(section('Pointer'));
    out.push(row('Sensitivity', '−1 is slowest, 0 is untouched, 1 is fastest.',
        slider({
            min: -100, max: 100, value: Math.round(asNum(g('input:sensitivity'), 0) * 100),
            format: (v) => (v / 100).toFixed(2),
        }, (v) => h('input:sensitivity', (v / 100).toFixed(2)))));
    out.push(row('Acceleration profile', null,
        segmented([['flat', 'flat'], ['adaptive', 'adaptive']], g('input:accel_profile', 'flat'), (v) => h('input:accel_profile', v))));
    out.push(row('Force no acceleration', null, toggle(asBool(g('input:force_no_accel')), (v) => h('input:force_no_accel', v))));
    out.push(row('Focus follows mouse', null, toggle(asNum(g('input:follow_mouse'), 1) !== 0, (v) => h('input:follow_mouse', v ? 1 : 0))));

    out.push(section('Touchpad'));
    out.push(row('Natural scroll', null, toggle(asBool(g('input:touchpad:natural_scroll')), (v) => h('input:touchpad:natural_scroll', v))));
    out.push(row('Tap to click', null, toggle(asBool(g('input:touchpad:tap_to_click'), true), (v) => h('input:touchpad:tap_to_click', v))));
    out.push(row('Disable while typing', null, toggle(asBool(g('input:touchpad:disable_while_typing'), true), (v) => h('input:touchpad:disable_while_typing', v))));
    out.push(row('Scroll speed', null,
        slider({
            min: 20, max: 300, value: Math.round(asNum(g('input:touchpad:scroll_factor'), 1) * 100),
            format: (v) => `${v} %`,
        }, (v) => h('input:touchpad:scroll_factor', (v / 100).toFixed(2)))));

    out.push(section('Cursor'));
    out.push(row('Hide after', '0 keeps the cursor visible.',
        slider({ min: 0, max: 600, step: 5, value: asNum(g('cursor:inactive_timeout'), 0), unit: 's' }, (v) => h('cursor:inactive_timeout', v))));
    out.push(row('Hide while typing', null, toggle(asBool(g('cursor:hide_on_key_press')), (v) => h('cursor:hide_on_key_press', v))));
    out.push(row('Software cursors', 'Slower, but fixes a cursor that disappears on some drivers.',
        toggle(asBool(g('cursor:no_hardware_cursors')), (v) => h('cursor:no_hardware_cursors', v))));
    return out;
}

/* ── Network ─────────────────────────────────────────────────────────────── */

async function network(ctx) {
    const [net, bt, vpn] = await Promise.all([
        ctx.read(['net', 'status', '--machine']),
        ctx.read(['bt', 'status', '--machine']),
        ctx.read(['net', 'vpn', 'list', '--machine']),
    ]);
    const n = kv(net);
    const b = kv(bt);
    const out = [el('h1.pagetitle', 'Network')];

    out.push(section('Wi-Fi'));
    out.push(row(n.get('ssid') || 'Not connected', n.get('ipv4') ? `IPv4 ${n.get('ipv4')}` : null,
        toggle(asBool(n.get('radio'), true), (v) => ctx.run(['net', 'radio', v ? 'on' : 'off']))));
    const scan = el('button.btn', { type: 'button' }, 'Scan for networks');
    scan.addEventListener('click', async () => {
        scan.disabled = true;
        scan.textContent = 'Scanning…';
        ctx.announce(['net', 'list', '--rescan', '--machine']);
        const aps = records(await ctx.read(['net', 'list', '--rescan', '--machine']));
        scan.disabled = false;
        scan.textContent = `Scan for networks (${aps.length} found)`;
    });
    out.push(row('Available networks', 'A rescan takes a few seconds; the footer says when it finishes.', scan));

    out.push(section('Bluetooth'));
    out.push(row('Adapter', b.get('adapter') || null,
        toggle(asBool(b.get('powered')), (v) => ctx.run(['bt', 'power', v ? 'on' : 'off']))));

    out.push(section('Airplane mode'));
    out.push(row('Airplane mode', 'Every radio, Bluetooth included.',
        toggle(asBool(n.get('airplane')), () => ctx.run(['net', 'airplane', 'toggle']))));

    const vpns = records(vpn);
    if (vpns.length) {
        out.push(section('VPN'));
        for (const v of vpns) {
            const name = v.get('name') || v.get('@');
            out.push(row(name, null,
                toggle(asBool(v.get('active')), (on) => ctx.run(['net', 'vpn', on ? 'up' : 'down', name]))));
        }
    }
    return out;
}

/* ── Power ───────────────────────────────────────────────────────────────── */

async function power(ctx) {
    const st = kv(await ctx.read(['idle', 'status', '--machine']));
    const out = [el('h1.pagetitle', 'Power')];
    const mins = (v) => Math.max(0, Math.round(asNum(v, 0) / 60));

    out.push(section('Idle'));
    out.push(row('Dim & blank displays after', secsLabel(st.get('dpms')),
        slider({ min: 0, max: 60, value: mins(st.get('dpms')), format: (v) => (v ? `${v} min` : 'off') },
            (v) => ctx.run(['idle', 'set', '--dpms', v ? String(v * 60) : 'off']))));
    out.push(row('Lock after', secsLabel(st.get('lock')),
        slider({ min: 0, max: 120, value: mins(st.get('lock')), format: (v) => (v ? `${v} min` : 'off') },
            (v) => ctx.run(['idle', 'set', '--lock', v ? String(v * 60) : 'off']))));
    out.push(row('Suspend after', secsLabel(st.get('suspend')),
        slider({ min: 0, max: 240, value: mins(st.get('suspend')), format: (v) => (v ? `${v} min` : 'off') },
            (v) => ctx.run(['idle', 'set', '--suspend', v ? String(v * 60) : 'off']))));

    out.push(section('Keep awake'));
    out.push(row('Caffeine', 'Holds the session awake until you turn it off.',
        toggle(asBool(st.get('inhibited')), (v) => ctx.run(['idle', 'inhibit', v ? 'on' : 'off']))));
    return out;
}

/* ── Startup ─────────────────────────────────────────────────────────────── */

async function startup(ctx) {
    const items = records(await ctx.read(['startup', 'list', '--machine']));
    const out = [el('h1.pagetitle', 'Startup')];

    out.push(section('Tezca services'));
    for (const [id, label, hint] of [
        ['bar', 'Menubar', 'tezca-bar — the top strip'],
        ['dock', 'Dock', 'tezca-dock — the magnifying dock'],
    ]) {
        const running = await ctx.read([id, 'status']);
        out.push(row(label, hint,
            toggle(/running|active/i.test(running || ''), (v) => ctx.run([id, v ? 'start' : 'stop']))));
    }

    if (items.length) {
        out.push(section('Autostart entries'));
        for (const it of items) {
            const name = it.get('name') || it.get('@');
            const id = it.get('id') || name;
            const rm = el('button.btn.danger', { type: 'button' }, 'Remove');
            rm.addEventListener('click', () => {
                ctx.run(['startup', 'remove', id]);
                ctx.reload();
            });
            out.push(row(name, it.get('exec') || null, rm));
        }
    }
    return out;
}

/* ── Keybinds ────────────────────────────────────────────────────────────── */

async function keybinds(ctx) {
    const sections = await ctx.invoke('tz_keybinds');
    const out = [el('h1.pagetitle', 'Keybinds')];
    if (!sections.length) {
        out.push(el('div.pagehint', 'No keybinds were reported — check `tezca keybind list`.'));
        return out;
    }
    for (const s of sections) {
        out.push(section(s.title));
        const t = el('table.keylist');
        for (const b of s.binds) {
            t.append(el('tr',
                el('td.combo', b.combo),
                el('td.desc', b.desc || '—'),
                el('td.action', { title: b.action }, b.action)));
        }
        out.push(t);
    }
    return out;
}

/* ── Gaming ──────────────────────────────────────────────────────────────── */

async function gaming(ctx) {
    const out = [el('h1.pagetitle', 'Gaming')];
    out.push(section('Game mode'));
    out.push(row('Game mode', 'Silences notifications and drops animations while you play.',
        toggle(ctx.gameOn, (v) => ctx.run(['game', v ? 'on' : 'off']))));
    out.push(el('div.pagehint', 'Per-game options live in the profile itself — see `tezca game run`.'));
    return out;
}

/* ── System ──────────────────────────────────────────────────────────────── */

async function system(ctx) {
    const version = (await ctx.read(['--version'])) || '';
    const out = [el('h1.pagetitle', 'System')];

    out.push(section('About'));
    out.push(row('Tezca', version || 'unknown', null));
    out.push(row('Session', ctx.session[1] || '—', null));

    out.push(section('Session'));
    const actions = [
        ['Lock', ['idle', 'inhibit', 'off'], 'loginctl lock-session'],
        ['Log out', null, 'uwsm stop'],
        ['Suspend', null, 'systemctl suspend'],
        ['Reboot', null, 'systemctl reboot'],
        ['Power off', null, 'systemctl poweroff'],
    ];
    const bar = el('div.rowline');
    for (const [label, , shell] of actions) {
        const b = el('button.btn' + (label === 'Power off' ? '.danger' : ''), { type: 'button' }, label);
        b.addEventListener('click', () => ctx.confirmShell(label, shell));
        bar.append(b);
    }
    out.push(row('Actions', 'Each asks first — these end the session.', bar, { stack: true }));

    out.push(section('Maintenance'));
    const reload = el('button.btn', { type: 'button' }, 'Reload theme');
    reload.addEventListener('click', () => ctx.run(['theme', 'reload'], { reload: true }));
    out.push(row('Re-apply the active theme', 'Re-copies the palette and signals every surface.', reload));
    const reset = el('button.btn.danger', { type: 'button' }, 'Reset Hyprland options');
    reset.addEventListener('click', () => ctx.run(['hypr', 'reset']));
    out.push(row('Reset tuned options', 'Drops every `tezca hypr set` override back to the shipped config.', reset));
    return out;
}

export const BUILDERS = {
    appearance, bar, dock, displays: displaysPage, sound, input,
    network, power, startup, keybinds, gaming, system,
};
