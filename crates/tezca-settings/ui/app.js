/* The shell: theme tokens, sidebar, routing, the CLI echo footer and ⌘K.
 *
 * State is deliberately thin — `page` and whether the palette is open. Every
 * control value lives in the system, not here: a page reads what things
 * actually are when you open it, and writes through the `tezca` CLI when you
 * change one. Nothing is cached, so the window cannot disagree with the machine.
 */

import { el, clear, invoke, listen, icon } from './lib.js';
import { GROUPS, LABELS, BUILDERS, navIcon } from './pages.js';

const $ = (id) => document.getElementById(id);
const win = window.__TAURI__.window.getCurrentWindow();

let boot = null;
let page = 'appearance';
let paletteEl = null;

/* ── Theme ───────────────────────────────────────────────────────────────── */

/** Hex → the bare `r, g, b` triple, so a rule can set its own alpha. */
function rgbTriple(hex) {
    const h = String(hex).replace('#', '');
    if (h.length < 6) return null;
    const n = parseInt(h.slice(0, 6), 16);
    return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
}

function applyTokens(tokens) {
    const root = document.documentElement;
    for (const [name, hex] of Object.entries(tokens.colors || {})) {
        root.style.setProperty(`--tz-${name}`, hex);
        const t = rgbTriple(hex);
        if (t) root.style.setProperty(`--tz-${name}-rgb`, t);
    }
    root.dataset.light = tokens.light ? 'true' : 'false';
}

/* ── The echo footer ─────────────────────────────────────────────────────── */

const STATE_TEXT = {
    sent: ['sent', ''],
    running: ['running…', ''],
    applied: ['✓ applied', 'ok'],
    failed: ['✗ failed', 'err'],
};

function showEcho(e) {
    $('echoCmd').textContent = e.line;
    const [text, cls] = STATE_TEXT[e.state] || ['', ''];
    const s = $('echoState');
    s.className = `echo-state ${cls}`;
    s.textContent = e.detail ? `${text} — ${e.detail}` : text;
    s.title = e.detail || '';
}

/* ── Page context ────────────────────────────────────────────────────────── */

const ctx = {
    invoke,
    get themes() {
        return boot?.themes ?? [];
    },
    get session() {
        return boot?.session ?? ['', ''];
    },
    get gameOn() {
        return boot?.game_on ?? false;
    },
    get derive() {
        return !!boot?.tokens?.name === false;
    },

    /** Run a `tezca` command. The echo footer updates itself over the event. */
    async run(args, opts = {}) {
        const r = await invoke('tz_run', { args: args.map(String) });
        if (opts.reload) await reload();
        return r;
    },
    read: (args) => invoke('tz_read', { args: args.map(String) }),
    announce: (args) => invoke('tz_announce', { args: args.map(String) }),
    hypr: async (opts) => new Map(await invoke('tz_hypr', { opts })),
    reload: () => render(),

    // A getter, not a field: `prefs` is declared further down, so reading it
    // while this object literal is being built hits the temporal dead zone.
    get prefs() {
        return prefs;
    },

    /** Session actions ask first — they end what you are doing. */
    confirmShell(label, shell) {
        const ok = window.confirm(`${label}?\n\nThis runs:  ${shell}`);
        if (ok) invoke('tz_script', { name: 'session.sh', args: [label.toLowerCase()] });
    },
};

/* ── Navigation ──────────────────────────────────────────────────────────── */

/* ── Presentation preferences ────────────────────────────────────────────── */

/**
 * `shell` and `density` are how this window looks, not how the machine is
 * configured, so they live in the webview's own store rather than in
 * ~/.config/tezca. Nothing outside this panel reads them, and giving them a
 * `tezca` key would imply the bar or the CLI cared.
 */
const prefs = {
    get shell() {
        return localStorage.getItem('tz.shell') === 'rail' ? 'rail' : 'grouped';
    },
    get density() {
        return localStorage.getItem('tz.density') === 'compact' ? 'compact' : 'comfortable';
    },
    set(key, value) {
        localStorage.setItem(`tz.${key}`, value);
        applyPrefs();
        renderNav();
    },
};

function applyPrefs() {
    document.documentElement.dataset.shell = prefs.shell;
    document.documentElement.dataset.density = prefs.density;
}

/** Which group owns a page — the rail's selected state follows the page. */
const groupOf = (id) => GROUPS.findIndex(([, ids]) => ids.includes(id));

function renderNav() {
    const rail = $('rail');
    const isRail = prefs.shell === 'rail';
    rail.hidden = !isRail;
    clear(rail);
    if (isRail) {
        GROUPS.forEach(([title, ids], i) => {
            const b = el('button.railbtn', { type: 'button', title },
                navIcon(ids[0]), el('span', title.split(' ')[0]));
            b.classList.toggle('on', i === groupOf(page));
            // Jumping to a group means its first page — a rail button that
            // selected nothing would leave the sidebar showing another group's.
            b.addEventListener('click', () => go(ids[0]));
            rail.append(b);
        });
    }

    const nav = clear($('nav'));
    for (const [title, ids] of GROUPS) {
        // In rail mode the sidebar shows only the active group; the rail is what
        // switches between them.
        if (isRail && !ids.includes(page)) continue;
        nav.append(el('div.navgroup', title));
        for (const id of ids) {
            const b = el('button.navrow', { type: 'button' }, navIcon(id), el('span', LABELS[id]));
            b.classList.toggle('on', id === page);
            b.addEventListener('click', () => go(id));
            nav.append(b);
        }
    }
}

async function go(id) {
    page = id;
    closePalette();
    renderNav();
    // Selecting a page is itself a command you could have typed.
    showEcho({ line: `tezca settings --page ${id}`, state: 'sent', detail: '' });
    await renderPage();
}

async function renderPage() {
    const host = clear($('page'));
    host.append(el('div.pagehint', 'Loading…'));
    try {
        const nodes = await BUILDERS[page](ctx);
        clear(host).append(...nodes.filter(Boolean));
    } catch (err) {
        clear(host).append(
            el('h1.pagetitle', LABELS[page]),
            el('div.pagehint', `This page could not be built: ${err?.message || err}`),
        );
    }
}

/* ── Command palette (§1.4) ──────────────────────────────────────────────── */

/**
 * The index entries, each pointing at the page that owns it.
 *
 * The values are read live rather than sampled: a palette that says "165 Hz"
 * when the display is at 60 is worse than one that says nothing.
 */
function paletteIndex() {
    const t = boot?.tokens?.name || 'dynamic';
    return [
        ['appearance', 'Theme', t],
        ['appearance', 'Wallpaper', 'picture & fit'],
        ['appearance', 'Window gaps', 'inner & outer'],
        ['appearance', 'Blur', 'size & passes'],
        ['bar', 'Bar shape', 'floating · edge'],
        ['bar', 'Workspace numerals', 'arabic · mayan'],
        ['bar', 'Bar modules', 'left · centre · right'],
        ['bar', 'Clock format', 'strftime'],
        ['bar', 'Right-cluster strategy', 'clutter'],
        ['dock', 'Magnification', 'scale & radius'],
        ['dock', 'Pinned favourites', 'reorder'],
        ['displays', 'Arrangement', 'drag to arrange'],
        ['displays', 'Refresh rate', 'per monitor'],
        ['displays', 'Brightness', 'DDC/CI'],
        ['displays', 'Layout profiles', 'save & apply'],
        ['sound', 'Output device', 'and volume'],
        ['sound', 'Microphone', 'level & mute'],
        ['input', 'Keyboard layout', 'and variant'],
        ['input', 'Pointer sensitivity', 'and profile'],
        ['network', 'Wi-Fi', 'radio & scan'],
        ['network', 'Bluetooth', 'adapter'],
        ['power', 'Idle timeouts', 'dim · lock · suspend'],
        ['keybinds', 'Keybinds', 'the whole map'],
        ['keybinds', 'Rebind a shortcut', 'record a new combo'],
        ['keybinds', 'Reset a binding', 'back to the shipped default'],
    ];
}

function openPalette() {
    if (paletteEl) return;
    const items = paletteIndex();
    const input = el('input', { type: 'text', placeholder: 'Jump to a setting, theme, or keybind…' });
    const list = el('div.palette-list');
    let rows = [];
    let sel = 0;

    const draw = () => {
        const q = input.value.trim().toLowerCase();
        const hits = items
            .filter(([p, label, val]) => !q || `${LABELS[p]} ${label} ${val}`.toLowerCase().includes(q))
            .slice(0, 7); // the design's cap
        sel = 0;
        clear(list);
        rows = hits.map(([p, label, val], i) => {
            const r = el('button.palette-row', { type: 'button' },
                el('span.palette-page', LABELS[p]),
                el('span.palette-label', label),
                el('span.palette-value', val));
            if (i === 0) r.classList.add('on');
            r.addEventListener('click', () => {
                closePalette();
                go(p);
            });
            r.addEventListener('mouseenter', () => {
                rows.forEach((x) => x.classList.remove('on'));
                r.classList.add('on');
                sel = i;
            });
            list.append(r);
            return r;
        });
        if (!hits.length) list.append(el('div.palette-empty', `Nothing matches “${input.value}”.`));
    };

    const move = (d) => {
        if (!rows.length) return;
        rows[sel]?.classList.remove('on');
        sel = (sel + d + rows.length) % rows.length;
        rows[sel].classList.add('on');
        rows[sel].scrollIntoView({ block: 'nearest' });
    };

    input.addEventListener('input', draw);
    input.addEventListener('keydown', (e) => {
        if (e.key === 'ArrowDown') { e.preventDefault(); move(1); }
        else if (e.key === 'ArrowUp') { e.preventDefault(); move(-1); }
        else if (e.key === 'Enter') { e.preventDefault(); rows[sel]?.click(); }
    });

    const panel = el('div.palette',
        el('div.palette-head',
            icon(['M11 11m-6.5 0a6.5 6.5 0 1 0 13 0a6.5 6.5 0 1 0 -13 0', 'm16 16 4.5 4.5']),
            input),
        list,
        el('div.palette-foot',
            el('span', '↑↓ navigate'), el('span', '↵ open'), el('span', 'esc close')));

    paletteEl = el('div.scrim', panel);
    paletteEl.addEventListener('click', (e) => {
        if (e.target === paletteEl) closePalette();
    });
    document.body.append(paletteEl);
    draw();
    input.focus();
}

function closePalette() {
    paletteEl?.remove();
    paletteEl = null;
}

/* ── Boot ────────────────────────────────────────────────────────────────── */

async function reload() {
    boot = await invoke('tz_boot');
    applyTokens(boot.tokens);
    paintFooter();
}

function paintFooter() {
    $('footTheme').textContent = boot?.tokens?.name || 'dynamic palette';
    const [host, meta] = boot?.session ?? ['', ''];
    $('footMeta').textContent = meta || host || '—';
}

async function render() {
    await reload();
    renderNav();
    await renderPage();
}

/* A scripting error must not leave an invisible process behind: report it where
 * whoever typed `tezca settings` will see it, and show the window regardless so
 * there is something to close. */
const report = (level, message) => invoke('tz_log', { level, message }).catch(() => {});
window.addEventListener('error', (e) => report('error', `${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener('unhandledrejection', (e) =>
    report('error', `unhandled rejection: ${e.reason?.stack || e.reason}`));

async function main() {
    // The window is created hidden so it never flashes an unstyled frame: the
    // palette arrives over IPC, which is a round trip after the webview is
    // technically ready to paint.
    boot = await invoke('tz_boot');
    applyTokens(boot.tokens);
    applyPrefs();
    if (boot.page) page = boot.page;
    paintFooter();
    renderNav();
    await renderPage();
    await win.show();

    listen('tezca://echo', (e) => showEcho(e.payload));

    $('omni').addEventListener('click', openPalette);
    $('close').addEventListener('click', () => win.close());
    $('min').addEventListener('click', () => win.minimize());
    $('max').addEventListener('click', async () => {
        await win.toggleMaximize();
        $('win').classList.toggle('flush', await win.isMaximized());
    });

    window.addEventListener('keydown', (e) => {
        if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
            e.preventDefault();
            paletteEl ? closePalette() : openPalette();
        } else if (e.key === 'Escape') {
            closePalette();
        }
    });
}

main().catch(async (err) => {
    await report('fatal', err?.stack || String(err));
    clear($('page')).append(
        el('h1.pagetitle', 'Something went wrong'),
        el('div.pagehint', err?.message || String(err)),
    );
    await win.show();
});
