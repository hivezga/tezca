/* Small DOM helpers and the six control patterns from GAP_AUDIT §1.6.
 *
 * No framework: this window is a form, and a form is what the platform is
 * already good at. Every factory returns a real element and takes an `onChange`
 * that runs the CLI — there is no virtual tree and no re-render, so a slider
 * cannot lose its drag to a reconciliation pass.
 */

const { invoke } = window.__TAURI__.core;
export const listen = window.__TAURI__.event.listen;
export const convertFileSrc = window.__TAURI__.core.convertFileSrc;

export { invoke };

/* ── DOM ─────────────────────────────────────────────────────────────────── */

/** el('div.row', {title:'x'}, child, 'text') */
export function el(spec, attrs, ...kids) {
    const [tag, ...classes] = String(spec).split('.');
    const n = document.createElement(tag || 'div');
    if (classes.length) n.className = classes.join(' ');
    if (attrs && (attrs.nodeType || typeof attrs === 'string')) {
        kids.unshift(attrs);
    } else if (attrs) {
        for (const [k, v] of Object.entries(attrs)) {
            if (v === undefined || v === null || v === false) continue;
            if (k === 'on') for (const [ev, fn] of Object.entries(v)) n.addEventListener(ev, fn);
            else if (k === 'html') n.innerHTML = v;
            else if (k in n && k !== 'list') n[k] = v;
            else n.setAttribute(k, v);
        }
    }
    for (const k of kids.flat()) {
        if (k === null || k === undefined || k === false) continue;
        n.append(k.nodeType ? k : document.createTextNode(String(k)));
    }
    return n;
}

export const clear = (n) => {
    while (n.firstChild) n.removeChild(n.firstChild);
    return n;
};

/** Inline SVG at the design's 1.7px stroke on a 24px viewBox. */
export function icon(paths, size = 17) {
    const ns = 'http://www.w3.org/2000/svg';
    const s = document.createElementNS(ns, 'svg');
    s.setAttribute('viewBox', '0 0 24 24');
    s.setAttribute('width', size);
    s.setAttribute('height', size);
    s.setAttribute('fill', 'none');
    s.setAttribute('stroke', 'currentColor');
    s.setAttribute('stroke-width', '1.7');
    s.setAttribute('stroke-linecap', 'round');
    s.setAttribute('stroke-linejoin', 'round');
    for (const d of [].concat(paths)) {
        const p = document.createElementNS(ns, 'path');
        p.setAttribute('d', d);
        s.append(p);
    }
    return s;
}

/* ── Rows (§0.2) ─────────────────────────────────────────────────────────── */

/** A label block over an optional hint, with the control right-aligned. */
export function row(label, hint, control, opts = {}) {
    const lab = el('div.row-label', el('span.label', label));
    if (hint) lab.append(el('span.hint', hint));
    const r = el('div.row' + (opts.stack ? '.stack' : ''), lab);
    if (control) r.append(el('div.row-control', control));
    return r;
}

export const section = (title) => el('div.seclabel', title);

/* ── Control patterns (§1.6) ─────────────────────────────────────────────── */

/** 40 × 23px, 2.5px pad, 17px knob, 160ms. */
export function toggle(on, onChange) {
    const knob = el('span.knob');
    const s = el('button.switch', { type: 'button' }, knob);
    const paint = (v) => s.classList.toggle('on', !!v);
    paint(on);
    s.addEventListener('click', () => {
        const v = !s.classList.contains('on');
        paint(v);
        onChange(v);
    });
    s.setValue = paint;
    return s;
}

/** Segments at `5px 12px`, 7px radius, 12px/600. */
export function segmented(options, value, onChange) {
    const g = el('div.seg');
    const buttons = options.map(([val, text]) => {
        const b = el('button', { type: 'button' }, text ?? val);
        b.classList.toggle('on', val === value);
        b.addEventListener('click', () => {
            for (const o of g.children) o.classList.remove('on');
            b.classList.add('on');
            onChange(val);
        });
        g.append(b);
        return [val, b];
    });
    g.setValue = (v) => {
        for (const [val, b] of buttons) b.classList.toggle('on', val === v);
    };
    return g;
}

/**
 * A 250px group with a 62px right-aligned mono readout.
 *
 * `commit` fires on release rather than on every pixel of the drag: each one
 * shells out and writes a config file, and a drag from 24 to 64 would otherwise
 * be forty rewrites and forty lines in the echo footer.
 */
export function slider({ min, max, step = 1, value, unit = '', format }, onChange) {
    const out = el('span.readout');
    const inp = el('input', { type: 'range', min, max, step, value });
    const show = (v) => {
        out.textContent = format ? format(v) : unit ? `${v} ${unit}` : String(v);
    };
    show(value);
    inp.addEventListener('input', () => show(Number(inp.value)));
    inp.addEventListener('change', () => onChange(Number(inp.value)));
    const g = el('div.slider', inp, out);
    g.setValue = (v) => {
        inp.value = v;
        show(Number(v));
    };
    return g;
}

/** `7px 11px`, r10, 12px mono. Commits on blur or Enter, not per keystroke. */
export function field(value, onChange, placeholder = '') {
    const i = el('input.field', { type: 'text', value: value ?? '', placeholder });
    let last = i.value;
    const commit = () => {
        if (i.value === last) return;
        last = i.value;
        onChange(i.value);
    };
    i.addEventListener('blur', commit);
    i.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            commit();
        }
    });
    i.setValue = (v) => {
        i.value = v ?? '';
        last = i.value;
    };
    return i;
}

/** A theme card: swatch strip over name and description. */
export function themeCard(t, active, onPick) {
    const strip = el(
        'div.swatches',
        el('i', { style: `background:${t.base}` }),
        el('i', { style: `background:${t.accent}` }),
        el('i', { style: `background:${t.gold}` }),
        el('i', { style: `background:${t.accent}; opacity:.55` }),
    );
    const c = el(
        'button.themecard',
        { type: 'button' },
        strip,
        el('div.themename', t.label || t.name),
        t.description ? el('div.themedesc', t.description) : null,
    );
    c.classList.toggle('on', active);
    c.addEventListener('click', () => onPick(t.name));
    return c;
}

/* ── Value parsing ───────────────────────────────────────────────────────── */

/** Hyprland and the TOML configs both spell booleans several ways. */
export function asBool(v, fallback = false) {
    if (v === undefined || v === null || v === '') return fallback;
    const s = String(v).trim().toLowerCase();
    if (['1', 'true', 'on', 'yes', 'enabled'].includes(s)) return true;
    if (['0', 'false', 'off', 'no', 'disabled'].includes(s)) return false;
    return fallback;
}

export function asNum(v, fallback = 0) {
    const n = Number(String(v ?? '').trim());
    return Number.isFinite(n) ? n : fallback;
}

/** `key=value` lines, as every `--machine` reader emits them. */
export function kv(out) {
    const m = new Map();
    for (const line of String(out ?? '').split('\n')) {
        const i = line.indexOf('=');
        if (i < 0) continue;
        m.set(line.slice(0, i).trim(), line.slice(i + 1).trim());
    }
    return m;
}

/** `@record` delimited machine output → a list of maps. */
export function records(out) {
    const recs = [];
    for (const line of String(out ?? '').split('\n')) {
        if (line.startsWith('@')) {
            recs.push(new Map([['@', line.slice(1).trim()]]));
            continue;
        }
        const i = line.indexOf('=');
        if (i < 0 || !recs.length) continue;
        recs[recs.length - 1].set(line.slice(0, i).trim(), line.slice(i + 1).trim());
    }
    return recs;
}

/** A pair list from Rust (`Vec<(String, String)>`) as a Map. */
export const pairs = (list) => new Map((list ?? []).map(([k, v]) => [k, v]));

/** Seconds → the shortest honest label: `30m`, `1h`, `off`. */
export function secsLabel(s) {
    const n = asNum(s, 0);
    if (n <= 0) return 'off';
    if (n % 3600 === 0) return `${n / 3600}h`;
    if (n % 60 === 0) return `${n / 60}m`;
    return `${n}s`;
}
