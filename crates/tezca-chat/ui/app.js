/* The chat panel.
 *
 * Real tokens, not a simulation. The prototype fakes streaming with
 * `3 + random(4)` characters every 34ms; here the chunks come from the model
 * and 34ms is only the repaint cadence — Ollama emits per token, which is
 * bursty enough to thrash layout if every one caused a reflow.
 */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const win = window.__TAURI__.window.getCurrentWindow();

const $ = (id) => document.getElementById(id);
const report = (level, message) => invoke('ai_log', { level, message }).catch(() => {});

/** How often the transcript repaints while text is arriving. */
const PAINT_MS = 34;
/** Bars in the status sparkline. */
const SPARK_BARS = 12;
/** How long a copy button says `copied`. */
const COPIED_MS = 1400;

const state = {
    status: { up: false, resident: [], available: [], backend: '' },
    settings: { system: '', model: '', port: 0, backend: 'auto' },
    model: '',
    /** {role, content, foot?, stopped?} — `foot` is the per-message footer. */
    msgs: [],
    /** Text of the answer currently arriving. */
    live: '',
    turn: 0,
    streaming: false,
    attach: null,
    /** Recent tok/s samples for the status sparkline. */
    rates: [],
    /** Set when the user scrolls up, so streaming stops yanking them down. */
    pinned: false,
    menu: null,
    drawerOpen: false,
};

/* ── DOM helpers ─────────────────────────────────────────────────────────── */

function el(spec, attrs, ...kids) {
    const [tag, ...classes] = String(spec).split('.');
    const n = document.createElement(tag || 'div');
    if (classes.length) n.className = classes.join(' ');
    if (attrs && (attrs.nodeType || typeof attrs === 'string')) kids.unshift(attrs);
    else if (attrs) {
        for (const [k, v] of Object.entries(attrs)) {
            if (v === undefined || v === null || v === false) continue;
            if (k === 'on') for (const [e, f] of Object.entries(v)) n.addEventListener(e, f);
            else if (k in n) n[k] = v;
            else n.setAttribute(k, v);
        }
    }
    for (const k of kids.flat()) {
        if (k === null || k === undefined || k === false) continue;
        n.append(k.nodeType ? k : document.createTextNode(String(k)));
    }
    return n;
}

const clear = (n) => {
    while (n.firstChild) n.removeChild(n.firstChild);
    return n;
};

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
}

/* ── Markdown-ish ────────────────────────────────────────────────────────── */

/**
 * Split an answer into prose and fenced code.
 *
 * Deliberately only fences. A model's answer is mostly prose with the
 * occasional block, and the block is the part that needs a monospace surface
 * and a copy button; running a full markdown parser to reach it would be a
 * dependency and an injection surface for one feature.
 */
export function splitFences(text) {
    const parts = [];
    const re = /```([^\n`]*)\n?([\s\S]*?)(?:```|$)/g;
    let last = 0;
    let m;
    while ((m = re.exec(text)) !== null) {
        if (m.index > last) parts.push({ kind: 'text', text: text.slice(last, m.index) });
        parts.push({ kind: 'code', lang: m[1].trim(), text: m[2].replace(/\n$/, '') });
        last = re.lastIndex;
    }
    if (last < text.length) parts.push({ kind: 'text', text: text.slice(last) });
    return parts.filter((p) => p.kind === 'code' || p.text.trim() !== '');
}

function codeBlock(part) {
    const copy = el('button.lpcopy', { type: 'button' }, 'copy');
    copy.addEventListener('click', async (e) => {
        e.stopPropagation();
        try {
            await navigator.clipboard.writeText(part.text);
        } catch {
            // A webview without clipboard access still gets the selection path.
            const r = document.createRange();
            r.selectNodeContents(pre);
            getSelection().removeAllRanges();
            getSelection().addRange(r);
        }
        copy.textContent = 'copied';
        copy.classList.add('copied');
        setTimeout(() => {
            copy.textContent = 'copy';
            copy.classList.remove('copied');
        }, COPIED_MS);
    });
    const pre = el('div.lpcode', part.text, copy);
    return pre;
}

/* ── Transcript ──────────────────────────────────────────────────────────── */

function messageNode(m, i) {
    const who = m.role === 'user' ? 'you' : 'assistant';
    const node = el('div.lpmsg' + (m.role === 'user' ? '.lpuser' : ''));
    node.append(el('div.lpwho', el('span.lpwhoname', who)));

    if (m.role === 'user') {
        node.append(el('div.lpbody', m.content));
    } else {
        for (const part of splitFences(m.content)) {
            node.append(part.kind === 'code' ? codeBlock(part) : el('div.lpbody', part.text.trim()));
        }
    }

    if (m.foot || m.stopped) {
        const regen = el('button.lpbtn', { type: 'button', style: 'padding:3px 7px' }, 'regenerate');
        regen.addEventListener('click', () => regenerate(i));
        node.append(el('div.lprow', { style: 'gap:12px;margin-top:1px' },
            el('span.lpmeta', m.stopped ? `stopped · ${m.foot || ''}`.trim() : m.foot),
            el('span', { style: 'flex:1' }),
            m.role === 'assistant' ? regen : null));
    }
    return node;
}

function renderTranscript() {
    const scroll = $('scroll');
    clear(scroll);
    state.msgs.forEach((m, i) => scroll.append(messageNode(m, i)));

    if (state.streaming) {
        const node = el('div.lpmsg', el('div.lpwho', el('span.lpwhoname', 'assistant')));
        // The live text goes through the same splitter as a finished message, so
        // a fence becomes a code block the moment it opens rather than sitting
        // as literal backticks until the answer ends. `splitFences` closes an
        // unterminated fence at end-of-string for exactly this case.
        const parts = splitFences(state.live);
        if (!parts.length) parts.push({ kind: 'text', text: '' });
        parts.forEach((part, i) => {
            const last = i === parts.length - 1;
            if (part.kind === 'code') {
                const block = codeBlock(part);
                node.append(block);
                if (last) node.append(el('div.lpbody', el('span.lpcaret')));
            } else {
                const body = el('div.lpbody', part.text.replace(/^\n+/, ''));
                if (last) body.append(el('span.lpcaret'));
                node.append(body);
            }
        });
        scroll.append(node);
    }
    if (!state.msgs.length && !state.streaming) {
        scroll.append(el('div.lpmeta', { style: 'margin:auto;text-align:center;line-height:1.7' },
            state.status.up
                ? 'Ask the local model something.'
                : 'No local model server is listening. Start Ollama, or set the port in settings.'));
    }
    autoScroll();
}

function autoScroll() {
    const s = $('scroll');
    if (!state.pinned) s.scrollTop = s.scrollHeight;
}

/* ── Streaming ───────────────────────────────────────────────────────────── */

let pending = '';
let painter = null;

function startPainter() {
    if (painter) return;
    painter = setInterval(() => {
        if (!pending) return;
        state.live += pending;
        pending = '';
        renderTranscript();
    }, PAINT_MS);
}

function stopPainter() {
    clearInterval(painter);
    painter = null;
    if (pending) {
        state.live += pending;
        pending = '';
    }
}

async function send() {
    const ta = $('ta');
    const text = ta.value.trim();
    if (!text || state.streaming) return;

    const content = state.attach ? `${text}\n\n[${state.attach.label}]\n${state.attach.text}` : text;
    state.msgs.push({ role: 'user', content });
    state.attach = null;
    ta.value = '';
    autoGrow();
    renderAttach();
    await beginTurn();
}

/** Send everything so far and stream the answer. Ollama's chat is stateless. */
async function beginTurn() {
    state.streaming = true;
    state.live = '';
    state.rates = [];
    state.pinned = false;
    paintComposer();
    renderTranscript();
    startPainter();

    const history = state.msgs.map((m) => ({ role: m.role, content: m.content }));
    state.turn = await invoke('ai_send', { model: state.model, history });
}

function onChunk(c) {
    if (c.turn !== state.turn) return; // a stopped or superseded turn
    if (c.kind === 'token') {
        pending += c.text;
        pushRate();
        return;
    }
    if (c.kind === 'reasoning') return; // not shown; kept separate by design

    stopPainter();
    state.streaming = false;
    if (c.kind === 'error') {
        state.msgs.push({ role: 'assistant', content: state.live || `⚠ ${c.text}`, foot: c.text });
    } else {
        state.msgs.push({
            role: 'assistant',
            content: state.live,
            foot: `${c.tokens} tok · ${c.secs.toFixed(1)}s · ${Math.round(c.tps)} tok/s`,
        });
        setTps(0);
    }
    state.live = '';
    paintComposer();
    renderTranscript();
}

function stop() {
    if (!state.streaming) return;
    invoke('ai_stop');
    stopPainter();
    state.streaming = false;
    // Keep the partial: throwing away what already arrived is the one thing a
    // stop button must not do.
    if (state.live.trim()) {
        state.msgs.push({ role: 'assistant', content: state.live, stopped: true, foot: '' });
    }
    state.live = '';
    setTps(0);
    paintComposer();
    renderTranscript();
}

/** Truncate to just before message `i` and re-stream it. */
async function regenerate(i) {
    if (state.streaming) return;
    state.msgs = state.msgs.slice(0, i);
    renderTranscript();
    await beginTurn();
}

/* ── Status bar ──────────────────────────────────────────────────────────── */

function pushRate() {
    // Chars/4 is the usual token approximation; the backend's own measured rate
    // replaces it in the footer when the turn finishes.
    const now = performance.now();
    state.rates.push({ t: now, n: state.live.length + pending.length });
    while (state.rates.length > 2 && now - state.rates[0].t > 6000) state.rates.shift();
    const first = state.rates[0];
    const secs = (now - first.t) / 1000;
    if (secs > 0.3) setTps(((state.rates.at(-1).n - first.n) / 4) / secs);
}

let sparkSamples = [];

function setTps(v) {
    const label = $('tps');
    label.textContent = v > 0 ? `${Math.round(v)} tok/s` : 'idle';
    label.classList.toggle('busy', v > 0);

    sparkSamples.push(v);
    while (sparkSamples.length > SPARK_BARS) sparkSamples.shift();
    const peak = Math.max(1, ...sparkSamples);
    const spark = clear($('spark'));
    for (const s of sparkSamples) {
        spark.append(el('i', { style: `height:${Math.max(1, Math.round((s / peak) * 16))}px` }));
    }
}

function paintStatus() {
    const m = state.status.resident.find((x) => x.name === state.model) || state.status.resident[0];
    $('dot').classList.toggle('off', !state.status.up);
    $('modelName').textContent = m ? m.name : state.status.up ? 'no model loaded' : 'not running';

    const accel = $('accel');
    if (m && m.accel) {
        accel.hidden = false;
        accel.textContent = m.accel;
        accel.classList.toggle('warn', m.degraded);
    } else {
        accel.hidden = true;
    }
    $('modelMeta').textContent = m
        ? [m.quant, m.size, m.ctx].filter(Boolean).join(' · ')
        : state.status.backend || '';

    const pct = m && m.vram_pct !== null && m.vram_pct !== undefined ? m.vram_pct : null;
    $('vramBar').style.width = `${pct ?? 0}%`;
    $('vramLabel').textContent = pct === null ? 'VRAM —' : `VRAM ${pct}% of ${m.size || 'model'}`;

    const used = state.msgs.reduce((n, x) => n + x.content.length, 0) / 4;
    $('ctxUse').textContent = used > 0 ? `${Math.round(used)} tok in context` : '';
}

function paintComposer() {
    $('sendBtn').hidden = state.streaming;
    $('stopBtn').hidden = !state.streaming;
    $('sendBtn').disabled = state.streaming || !state.status.up;
}

/* ── Composer chips ──────────────────────────────────────────────────────── */

function renderChips() {
    const chips = clear($('chips'));
    const mk = (label, title, fn) => {
        const b = el('button.lpbtn', { type: 'button', title }, label);
        b.addEventListener('click', fn);
        return b;
    };
    chips.append(
        mk('selection', 'Paste the current clipboard selection', async () => {
            try {
                const t = await navigator.clipboard.readText();
                if (t) state.attach = { label: 'selection', text: t };
            } catch {
                state.attach = { label: 'selection', text: '' };
                report('warn', 'clipboard read refused');
            }
            renderAttach();
        }),
        mk('attach', 'Attach a file by path', () => {
            const p = window.prompt('Path to attach');
            if (p) state.attach = { label: p.split('/').pop(), text: `(file: ${p})` };
            renderAttach();
        }),
        mk('screenshot', 'Not wired yet — needs a capture path', () => {
            report('info', 'screenshot chip: no capture path is wired');
        }),
    );
}

function renderAttach() {
    const row = clear($('attachRow'));
    if (!state.attach) return;
    const drop = el('button.lpmeta', { type: 'button', style: 'background:none;border:none;cursor:pointer' }, '✕');
    drop.addEventListener('click', () => {
        state.attach = null;
        renderAttach();
    });
    row.append(el('div.lpattach', el('span', state.attach.label), drop));
}

/* ── Menus and drawer ────────────────────────────────────────────────────── */

function closeMenus() {
    state.menu = null;
    clear($('menus'));
}

function toggleMenu(which) {
    if (state.menu === which) return closeMenus();
    closeMenus();
    state.menu = which;
    const host = $('menus');
    host.append(which === 'models' ? modelsMenu() : sessionsMenu());
}

function modelRow(m, resident) {
    const b = el('button.lpsel', { type: 'button', style: 'width:100%' },
        el('span.lpdot' + (resident ? '' : '.hollow'), { style: resident ? '' : '' }),
        el('div', { style: 'flex:1;min-width:0' },
            el('div', { style: `font-size:12px;color:var(--tz-${resident ? 'text' : 'subtext'})` }, m.name),
            el('div.lprow', { style: 'gap:5px;margin-top:2px' },
                m.accel ? el('span.lpaccel' + (m.degraded ? '.warn' : ''), m.accel) : null,
                el('span.lpmeta', [m.quant, m.ctx].filter(Boolean).join(' · ')))),
        el('span.lpmeta', m.size));
    b.addEventListener('click', () => {
        state.model = m.name;
        invoke('ai_set', { key: 'model', value: m.name });
        closeMenus();
        paintStatus();
    });
    return b;
}

function modelsMenu() {
    const menu = el('div.lpmenu.models');
    menu.append(el('div.lpmeta.head', 'Resident'));
    if (state.status.resident.length) {
        for (const m of state.status.resident) menu.append(modelRow(m, true));
    } else {
        menu.append(el('div.lpmeta', { style: 'padding:2px 9px 7px' }, 'nothing loaded'));
    }
    menu.append(el('div.lprule'), el('div.lpmeta.head', 'Available locally'));
    const resident = new Set(state.status.resident.map((m) => m.name));
    const rest = state.status.available.filter((m) => !resident.has(m.name));
    if (rest.length) {
        for (const m of rest) menu.append(modelRow(m, false));
    } else {
        menu.append(el('div.lpmeta', { style: 'padding:2px 9px 7px' }, 'none on disk'));
    }
    return menu;
}

function sessionsMenu() {
    // Conversations are in-memory for now: there is no store behind them, and a
    // list that forgets everything on close would be a promise this cannot keep.
    const menu = el('div.lpmenu.sessions');
    menu.append(el('div.lpmeta.head', 'This conversation'));
    menu.append(el('div.lpmeta', { style: 'padding:2px 9px 8px;line-height:1.6' },
        `${state.msgs.length} messages · not saved`));
    menu.append(el('div.lprule'));
    const clearBtn = el('button.lpsel', { type: 'button', style: 'width:100%' },
        el('span', { style: 'font-size:12px;color:var(--tz-accent)' }, 'New conversation'));
    clearBtn.addEventListener('click', () => {
        state.msgs = [];
        closeMenus();
        renderTranscript();
        paintStatus();
    });
    menu.append(clearBtn);
    return menu;
}

function toggleDrawer() {
    state.drawerOpen = !state.drawerOpen;
    const host = clear($('drawer'));
    $('settingsBtn').classList.toggle('on', state.drawerOpen);
    if (!state.drawerOpen) return;

    const prompt = el('textarea.lpprompt', {
        value: state.settings.system,
        placeholder: 'System prompt — prepended to every conversation.',
    });
    prompt.addEventListener('change', () => {
        state.settings.system = prompt.value;
        invoke('ai_set', { key: 'system', value: prompt.value });
    });

    // Temperature and top-p are per-request sampler settings the backend takes,
    // and `tezca bar set` has no key for them — they live for this session only,
    // which is honest until the CLI grows somewhere to put them.
    const slider = (label, min, max, step, value, fmt) => {
        const out = el('span.lpmeta', { style: 'color:var(--tz-subtext)' }, fmt(value));
        const inp = el('input', { type: 'range', min, max, step, value });
        inp.addEventListener('input', () => (out.textContent = fmt(Number(inp.value))));
        return el('div',
            el('div.lprow', { style: 'justify-content:space-between;margin-bottom:5px' },
                el('span.lpmeta', label), out),
            inp);
    };

    const chips = el('div.lprow', { style: 'gap:6px' });
    for (const [label, key, val] of [
        ['backend', 'backend', state.settings.backend],
        ['port', 'port', String(state.settings.port || 'default')],
    ]) {
        chips.append(el('span.lpbtn', `${label} ${val}`));
        void key;
    }

    host.append(el('div.lpdrawer',
        el('div.lpmeta', { style: 'margin-bottom:6px' }, 'SYSTEM PROMPT'),
        prompt,
        el('div.lpgrid',
            slider('temperature', 0, 2, 0.05, 0.8, (v) => Number(v).toFixed(2)),
            slider('top-p', 0, 1, 0.05, 0.9, (v) => Number(v).toFixed(2))),
        el('div.lpmeta', { style: 'margin:12px 0 6px' }, 'BACKEND'),
        chips));
}

/* ── Composer behaviour ──────────────────────────────────────────────────── */

function autoGrow() {
    const ta = $('ta');
    ta.style.height = 'auto';
    ta.style.height = `${Math.min(120, ta.scrollHeight)}px`;
}

/* ── Boot ────────────────────────────────────────────────────────────────── */

async function refreshStatus() {
    state.status = await invoke('ai_status');
    if (!state.model) {
        state.model = state.settings.model || state.status.resident[0]?.name || '';
    }
    paintStatus();
    paintComposer();
}

async function main() {
    const boot = await invoke('ai_boot');
    applyTokens(boot.tokens);
    state.status = boot.status;
    state.settings = boot.settings;
    state.model = boot.settings.model || boot.status.resident[0]?.name || '';

    paintStatus();
    paintComposer();
    renderChips();
    renderTranscript();
    setTps(0);
    await win.show();

    listen('ai://chunk', (e) => onChunk(e.payload));

    $('modelBtn').addEventListener('click', () => toggleMenu('models'));
    $('sessionsBtn').addEventListener('click', () => toggleMenu('sessions'));
    $('settingsBtn').addEventListener('click', toggleDrawer);
    $('closeBtn').addEventListener('click', () => win.close());
    $('sendBtn').addEventListener('click', send);
    $('stopBtn').addEventListener('click', stop);

    const ta = $('ta');
    ta.addEventListener('input', autoGrow);
    ta.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            send();
        }
    });
    ta.focus();

    // Auto-scroll unless the user has scrolled up to read history.
    $('scroll').addEventListener('scroll', () => {
        const s = $('scroll');
        state.pinned = s.scrollHeight - s.scrollTop - s.clientHeight > 40;
    });

    window.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            if (state.menu) closeMenus();
            else if (state.streaming) stop();
            else win.close();
        }
    });
    document.addEventListener('click', (e) => {
        if (state.menu && !e.target.closest('.lpmenu, .lpsel, .lpbtn')) closeMenus();
    });

    setInterval(refreshStatus, 5000);
}

window.addEventListener('error', (e) => report('error', `${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener('unhandledrejection', (e) =>
    report('error', `unhandled rejection: ${e.reason?.stack || e.reason}`));

main().catch(async (err) => {
    await report('fatal', err?.stack || String(err));
    await win.show();
});
