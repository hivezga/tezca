/* The Displays page: a drag-to-arrange canvas over per-monitor controls.
 *
 * The arithmetic — fitting the arrangement into the canvas, snapping an edge to
 * a neighbour, detecting overlap — stays in `arrange.rs`. It is the one part of
 * this page with a real test suite, being wrong means writing a bad layout to
 * the compositor, and a second definition of "flush" in JavaScript would drift
 * from the first. This file is the interaction; Rust is the geometry.
 */

import { el, row, section, segmented, toggle, asBool, asNum } from './lib.js';

/** How long a new mode has to be confirmed before it reverts itself. */
const REVERT_SECONDS = 12;

export async function displaysPage(ctx) {
    const d = await ctx.invoke('tz_displays');
    const out = [el('h1.pagetitle', 'Displays')];

    if (!d.monitors.length) {
        out.push(el('div.pagehint', 'No monitors were reported — check `tezca display list`.'));
        return out;
    }

    /* ── The canvas ──────────────────────────────────────────────────────── */

    out.push(section('Arrangement'));
    const canvas = el('div.arrange');
    const status = el('div.arrange-status');
    const original = d.rects.map((r) => [r.x, r.y]);
    let rects = d.rects.map((r) => ({ ...r }));
    let selected = 0;
    let view = { scale: 1, ox: 0, oy: 0, tol: 0 };

    const paint = async () => {
        const box = canvas.getBoundingClientRect();
        view = await ctx.invoke('tz_fit', { rects, w: box.width || 690, h: box.height || 230 });
        canvas.textContent = '';
        rects.forEach((r, i) => {
            const n = el('div.mon' + (i === selected ? '.on' : '') + (r.disabled ? '.off' : ''), {
                style:
                    `left:${r.x * view.scale + view.ox}px;top:${r.y * view.scale + view.oy}px;` +
                    `width:${r.w * view.scale}px;height:${r.h * view.scale}px`,
            }, el('div.mon-name', r.name), el('div.mon-detail', r.detail));
            n.addEventListener('pointerdown', (e) => startDrag(e, i, n));
            canvas.append(n);
        });
        const [moved, clash] = await ctx.invoke('tz_layout_status', { rects, original });
        status.classList.toggle('warn', clash);
        status.textContent = clash
            ? '⚠ two screens overlap'
            : moved.length === 0
                ? 'layout matches the compositor'
                : `${moved.length} moved · not applied`;
        applyBtn.disabled = moved.length === 0 || clash;
        for (const [i, r] of rects.entries()) {
            if (clash) canvas.children[i]?.classList.add('clash');
            void r;
        }
    };

    const startDrag = (e, i, node) => {
        selected = i;
        const box = canvas.getBoundingClientRect();
        // Deltas are applied to the position the drag *began* at, not
        // accumulated, so a drag that snaps and then pulls away does not drift.
        const start = { x: rects[i].x, y: rects[i].y, px: e.clientX, py: e.clientY };
        node.classList.add('dragging');
        node.setPointerCapture(e.pointerId);

        const move = async (ev) => {
            const lx = start.x + Math.round((ev.clientX - start.px) / view.scale);
            const ly = start.y + Math.round((ev.clientY - start.py) / view.scale);
            const [sx, sy] = await ctx.invoke('tz_snap', { rects, i, x: lx, y: ly, tol: view.tol });
            rects[i] = { ...rects[i], x: sx, y: sy };
            const n = canvas.children[i];
            if (n) {
                n.style.left = `${sx * view.scale + view.ox}px`;
                n.style.top = `${sy * view.scale + view.oy}px`;
            }
        };
        const up = async () => {
            node.removeEventListener('pointermove', move);
            node.removeEventListener('pointerup', up);
            node.classList.remove('dragging');
            await paint();
        };
        node.addEventListener('pointermove', move);
        node.addEventListener('pointerup', up);
        void box;
    };

    const applyBtn = el('button.btn.accent', { type: 'button' }, 'Apply layout');
    applyBtn.addEventListener('click', async () => {
        const [moved] = await ctx.invoke('tz_layout_status', { rects, original });
        for (const [name, x, y] of moved) {
            await ctx.run(['display', 'set', name, '--pos', `${x}x${y}`], { quiet: true });
        }
        // A layout is one decision, so one countdown covers the whole apply.
        confirmOrRevert(ctx, holder, 'Keep this arrangement?', async () => {
            for (const [i, r] of rects.entries()) {
                const [ox, oy] = original[i];
                if (r.x !== ox || r.y !== oy) {
                    await ctx.run(['display', 'set', r.name, '--pos', `${ox}x${oy}`], { quiet: true });
                }
            }
            rects = d.rects.map((r) => ({ ...r }));
            paint();
        }, () => {
            original.splice(0, original.length, ...rects.map((r) => [r.x, r.y]));
            paint();
        });
    });

    const tidyBtn = el('button.btn', { type: 'button' }, 'Tidy up');
    tidyBtn.addEventListener('click', async () => {
        rects = await ctx.invoke('tz_tidy', { rects });
        paint();
    });
    const resetBtn = el('button.btn', { type: 'button' }, 'Revert');
    resetBtn.addEventListener('click', () => {
        rects = d.rects.map((r) => ({ ...r }));
        paint();
    });

    const holder = el('div.stackcol', canvas, el('div.rowline', status, el('div', { style: 'flex:1' }), tidyBtn, resetBtn, applyBtn));
    out.push(el('div.row.stack', el('div.row-control', holder)));
    out.push(el('div.pagehint', 'Drag a screen to move it; edges snap to their neighbours. Nothing is applied until you say so.'));
    // The canvas has no size until it is in the document.
    queueMicrotask(paint);

    /* ── Per-monitor controls ────────────────────────────────────────────── */

    for (const m of d.monitors) {
        out.push(section(`${m.name}${m.desc ? ` · ${m.desc}` : ''}`));

        // Resolution and refresh rate are two controls, not one list of every
        // combination: this panel reports 24 modes, and a segmented control with
        // 24 segments is a horizontal scrollbar wearing a costume. The CLI takes
        // them separately too (`--resolution`, `--refresh`), so a rate change
        // keeps the resolution it was already at.
        const modes = m.modes || [];
        const resolutions = [...new Set(modes.map((x) => x.split('@')[0]))];
        const rates = modes
            .filter((x) => x.startsWith(`${m.res}@`))
            .map((x) => x.split('@')[1]);
        if (resolutions.length > 1) {
            out.push(row('Resolution', 'Applying a mode starts a countdown you have to confirm.',
                segmented(resolutions.map((x) => [x, x]), m.res,
                    (v) => applyWith(ctx, holder, m.name, ['--resolution', v]))));
        }
        if (rates.length > 1) {
            const cur = rates.find((r) => Math.abs(Number(r) - Number(m.rate)) < 0.01) || rates[0];
            out.push(row('Refresh rate', null,
                segmented(rates.map((x) => [x, `${trimRate(x)} Hz`]), cur,
                    (v) => applyWith(ctx, holder, m.name, ['--refresh', v]))));
        }
        out.push(row('Scale', null,
            segmented([['1', '1×'], ['1.25', '1.25×'], ['1.5', '1.5×'], ['2', '2×'], ['auto', 'auto']],
                String(asNum(m.scale, 1)), (v) => applyWith(ctx, holder, m.name, ['--scale', v]))));
        out.push(row('Rotation', null,
            segmented([['0', 'none'], ['1', '90°'], ['2', '180°'], ['3', '270°']],
                String(asNum(m.transform, 0)), (v) => applyWith(ctx, holder, m.name, ['--transform', v]))));
        out.push(row('Adaptive sync', 'VRR. Fullscreen only is the safe default on multi-monitor.',
            segmented([['off', 'off'], ['on', 'on'], ['fullscreen', 'fullscreen']],
                m.vrr || 'off', (v) => ctx.run(['display', 'set', m.name, '--vrr', v]))));
        out.push(row('Colour depth', '10-bit needs the link bandwidth to carry it.',
            segmented([['8', '8-bit'], ['10', '10-bit']], String(asNum(m.bitdepth, 8)),
                (v) => ctx.run(['display', 'set', m.name, '--bitdepth', v]))));
        // DPMS, not an enable flag: `tezca display` has no --enable/--disable,
        // and blanking is the reversible thing you actually want from a panel
        // you might be looking at. It is deliberately not persisted.
        out.push(row('Output awake', 'Blanks this screen until something wakes it. Not persisted.',
            toggle(!m.disabled, (v) => ctx.run(['display', 'dpms', m.name, v ? 'on' : 'off']))));

        const reset = el('button.btn', { type: 'button' }, 'Reset to shipped config');
        reset.addEventListener('click', () => ctx.run(['display', 'reset', m.name]));
        out.push(row('Reset', null, reset));
    }

    /* ── Profiles ────────────────────────────────────────────────────────── */

    out.push(section('Layout profiles'));
    const list = (await ctx.read(['display', 'profile', 'list'])) || '';
    const names = list.split('\n').map((s) => s.trim()).filter(Boolean);
    const nameField = el('input.field', { type: 'text', placeholder: 'profile name' });
    const save = el('button.btn', { type: 'button' }, 'Save current');
    save.addEventListener('click', () => {
        const n = nameField.value.trim();
        if (n) ctx.run(['display', 'profile', 'save', n], { reload: true });
    });
    out.push(row('Save this arrangement', 'Recall it later without re-dragging.',
        el('div.rowline', nameField, save)));
    for (const n of names) {
        const apply = el('button.btn', { type: 'button' }, 'Apply');
        apply.addEventListener('click', () => ctx.run(['display', 'profile', 'apply', n]));
        const rm = el('button.btn.danger', { type: 'button' }, 'Delete');
        rm.addEventListener('click', () => ctx.run(['display', 'profile', 'rm', n], { reload: true }));
        out.push(row(n, null, el('div.rowline', apply, rm)));
    }
    return out;
}

/** `165.00` → `165`, `143.97` → `143.97` — the trailing zeros are noise. */
const trimRate = (r) => {
    const v = Number(r);
    if (!Number.isFinite(v)) return String(r);
    return Math.abs(v - Math.round(v)) < 0.005 ? String(Math.round(v)) : v.toFixed(2).replace(/0+$/, '').replace(/\.$/, '');
};

const applyWith = (ctx, holder, name, flags) =>
    ctx.run(['display', 'set', name, ...flags]).then(() =>
        confirmOrRevert(ctx, holder, `Keep ${name} as it is now?`, () => ctx.run(['display', 'reset', name])));

/**
 * The countdown that undoes a display change you cannot see the result of.
 *
 * A mode your monitor cannot show leaves a black screen and no way to click
 * "undo", so the undo has to happen on its own. Twelve seconds is long enough
 * to find the button and short enough that you are not waiting on a dead
 * screen.
 */
function confirmOrRevert(ctx, holder, question, revert, onKeep) {
    const old = holder.querySelector('.revert');
    if (old) old.remove();

    let left = REVERT_SECONDS;
    const count = el('span.mono', String(left));
    const keep = el('button.btn.accent', { type: 'button' }, 'Keep');
    const undo = el('button.btn', { type: 'button' }, 'Revert now');
    const box = el('div.revert',
        el('span.revert-text', question),
        el('span.mono', 'reverting in '), count, keep, undo);

    const stop = () => {
        clearInterval(timer);
        box.remove();
    };
    const timer = setInterval(() => {
        left -= 1;
        count.textContent = String(left);
        if (left <= 0) {
            stop();
            revert();
        }
    }, 1000);
    keep.addEventListener('click', () => {
        stop();
        onKeep?.();
    });
    undo.addEventListener('click', () => {
        stop();
        revert();
    });
    holder.append(box);
}

export { asBool };
