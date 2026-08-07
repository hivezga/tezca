/* The Keybinds page — the whole map, and the editor for it.
 *
 * Every write goes through `tezca keybind`, which owns the override layer at
 * ~/.config/tezca/keybinds.lua and the guards around it: the shipped map is
 * never touched, a rebind is refused if the bind moved since this page read it,
 * and a combo already in use is reported rather than silently doubled up. This
 * file's job is the part a CLI cannot do — read a shortcut off the keyboard.
 *
 * ## Why recording a shortcut needs the compositor's help
 *
 * Hyprland claims a bound combo before the focused window ever sees it. So a
 * plain keydown listener can read exactly the combos you have no reason to
 * rebind: press SUPER+B to move it and Brave opens instead. `tezca keybind
 * capture on` puts the session in an otherwise-empty submap for the duration,
 * which suspends every global bind, and the keys arrive here like ordinary
 * typing. Every path out of the capture — commit, cancel, click away, the
 * window losing focus, the watchdog — turns it back off, and the submap keeps
 * CTRL+ALT+Escape bound to its own release, so a crash mid-capture costs one
 * keypress rather than the session's keyboard.
 *
 * ## Why the key comes from `event.code`
 *
 * `code` is the physical key; `key` is what the layout and the held modifiers
 * make of it. Hyprland's binds are written unshifted — the shipped map says
 * `SUPER + SHIFT + 1`, not `SUPER + exclam` — so the physical key is what
 * reproduces the file's own spelling. The names below are xkb keysyms, which is
 * what `hl.bind` parses.
 */

import { el, row, section, field } from './lib.js';

/* ── Reading a keystroke ─────────────────────────────────────────────────── */

/** Physical modifier keys, so a modifier is tracked even if the webview's
 *  `metaKey` flag does not follow the compositor's idea of Super. */
const MOD_CODES = {
    ControlLeft: 'CTRL', ControlRight: 'CTRL',
    ShiftLeft: 'SHIFT', ShiftRight: 'SHIFT',
    AltLeft: 'ALT', AltRight: 'ALT',
    MetaLeft: 'SUPER', MetaRight: 'SUPER',
    OSLeft: 'SUPER', OSRight: 'SUPER',
};

/** `KeyboardEvent.code` → the keysym `hl.bind` expects. Letters, digits, the
 *  function row and the numeric keypad are ranges, handled below. */
const CODE_KEYS = {
    Space: 'SPACE', Enter: 'Return', Tab: 'Tab', Backspace: 'BackSpace',
    Delete: 'Delete', Insert: 'Insert', Home: 'Home', End: 'End',
    PageUp: 'Page_Up', PageDown: 'Page_Down',
    ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down',
    Minus: 'minus', Equal: 'equal', BracketLeft: 'bracketleft',
    BracketRight: 'bracketright', Backslash: 'backslash', Semicolon: 'semicolon',
    Quote: 'apostrophe', Backquote: 'grave', Comma: 'comma', Period: 'period',
    Slash: 'slash', IntlBackslash: 'less',
    PrintScreen: 'Print', ScrollLock: 'Scroll_Lock', Pause: 'Pause',
    CapsLock: 'Caps_Lock', ContextMenu: 'Menu', NumLock: 'Num_Lock',
    NumpadEnter: 'KP_Enter', NumpadAdd: 'KP_Add', NumpadSubtract: 'KP_Subtract',
    NumpadMultiply: 'KP_Multiply', NumpadDivide: 'KP_Divide',
    NumpadDecimal: 'KP_Decimal',
    // The keys a laptop or a media keyboard puts on their own switches. They
    // only ever reach a window while the binds are suspended, which is exactly
    // when this runs.
    AudioVolumeUp: 'XF86AudioRaiseVolume', AudioVolumeDown: 'XF86AudioLowerVolume',
    AudioVolumeMute: 'XF86AudioMute', MediaPlayPause: 'XF86AudioPlay',
    MediaTrackNext: 'XF86AudioNext', MediaTrackPrevious: 'XF86AudioPrev',
    MediaStop: 'XF86AudioStop',
};

/** The keysym for a keystroke, or null for a key with no name we can write. */
export function keysymOf(e) {
    const code = e.code || '';
    if (CODE_KEYS[code]) return CODE_KEYS[code];
    let m = /^Key([A-Z])$/.exec(code);
    if (m) return m[1];
    m = /^Digit(\d)$/.exec(code);
    if (m) return m[1];
    m = /^F(\d{1,2})$/.exec(code);
    if (m) return `F${m[1]}`;
    m = /^Numpad(\d)$/.exec(code);
    if (m) return `KP_${m[1]}`;
    // No `code` at all (a synthetic event, or a key the webview does not know):
    // a single printable character is still usable, uppercased the way the
    // shipped map writes letters.
    if (e.key && e.key.length === 1 && /[a-z0-9]/i.test(e.key)) return e.key.toUpperCase();
    return null;
}

/** The modifiers held for a keystroke, in the order `tezca keybind` stores
 *  them: SUPER first, then alphabetically. Both the event's own flags and the
 *  physically-tracked set count — either alone has been seen to miss Super. */
export function modsOf(e, held = new Set()) {
    const m = new Set(held);
    if (e.ctrlKey) m.add('CTRL');
    if (e.shiftKey) m.add('SHIFT');
    if (e.altKey) m.add('ALT');
    const supered = e.metaKey
        || e.getModifierState?.('Meta')
        || e.getModifierState?.('Super')
        || e.getModifierState?.('OS');
    if (supered) m.add('SUPER');
    return [...m].sort((a, b) => (a === 'SUPER' ? -1 : b === 'SUPER' ? 1 : a < b ? -1 : 1));
}

export const comboText = (mods, key) => [...mods, key].filter(Boolean).join(' + ');

/**
 * Open the recorder and resolve with `{mods, key}`, or null if it was cancelled.
 *
 * Resolution waits for `capture off` to come back rather than racing it: the
 * caller's next step is a rebind, which reloads Hyprland's config, and doing
 * that while still inside the capture submap would leave the session in it.
 */
function recordCombo(ctx, bind) {
    return new Promise((resolve) => {
        const shown = el('div.capture-combo.waiting', 'Press a shortcut');
        const warn = el('div.capture-warn');
        warn.hidden = true;
        const panel = el('div.capture',
            el('div.capture-what', bind.desc || `line ${bind.line}`),
            shown,
            warn,
            el('div.capture-foot',
                el('span', 'esc cancels'),
                el('span', 'ctrl+alt+esc force-releases the keyboard')));
        const scrim = el('div.scrim.center', panel);
        document.body.append(scrim);

        const held = new Set();
        let settled = false;
        let watchdog = 0;
        const finish = (value) => {
            if (settled) return;
            settled = true;
            clearTimeout(watchdog);
            window.removeEventListener('keydown', onKeyDown, true);
            window.removeEventListener('keyup', onKeyUp, true);
            window.removeEventListener('blur', onBlur);
            scrim.remove();
            ctx.read(['keybind', 'capture', 'off']).then(() => resolve(value), () => resolve(value));
        };

        const paint = (mods, key) => {
            const text = comboText(mods, key);
            shown.textContent = text || 'Press a shortcut';
            shown.classList.toggle('waiting', !text);
        };

        function onKeyDown(e) {
            // Nothing else in the window may act on these — Ctrl+K would open
            // the palette underneath the recorder.
            e.preventDefault();
            e.stopPropagation();
            const mod = MOD_CODES[e.code];
            if (mod) {
                held.add(mod);
                paint(modsOf(e, held), '');
                return;
            }
            const key = keysymOf(e);
            const mods = modsOf(e, held);
            // Escape cancels — but only on its own, so a modified Escape is
            // still a combo you can put a binding on.
            if (e.code === 'Escape' && !mods.length) {
                finish(null);
                return;
            }
            if (!key) {
                warn.hidden = false;
                warn.textContent = 'That key has no name Hyprland can bind — try another.';
                return;
            }
            warn.hidden = true;
            paint(mods, key);
            // A beat on the recorded combo, so you see what was taken rather
            // than the panel vanishing under your fingers.
            setTimeout(() => finish({ mods, key }), 160);
        }

        function onKeyUp(e) {
            e.preventDefault();
            e.stopPropagation();
            const mod = MOD_CODES[e.code];
            if (mod) {
                held.delete(mod);
                if (!settled) paint(modsOf(e, held), '');
            }
        }

        // Losing focus means the keys are going somewhere else, and the binds
        // have to come back before they land there.
        const onBlur = () => finish(null);
        scrim.addEventListener('mousedown', () => finish(null));
        window.addEventListener('keydown', onKeyDown, true);
        window.addEventListener('keyup', onKeyUp, true);
        window.addEventListener('blur', onBlur);
        watchdog = setTimeout(() => finish(null), 30000);

        // Ungrabbed, the recorder still works for a combo nothing owns yet — so
        // say what will happen rather than refusing to open.
        ctx.read(['keybind', 'capture', 'on']).then((ok) => {
            // Closed before the grab landed — cancelling faster than one round
            // trip is easy with a click, and the release that already ran was a
            // release of nothing. Undo it now or the session stays suspended.
            if (settled) {
                if (ok !== null) ctx.read(['keybind', 'capture', 'off']);
                return;
            }
            if (ok === null) {
                warn.hidden = false;
                warn.textContent =
                    'Global shortcuts could not be suspended, so a combo that is already '
                    + 'bound will run its action instead of being recorded.';
            }
        });
    });
}

/* ── Writing a change ────────────────────────────────────────────────────── */

/** Who else holds this combo, out of `conflict: SUPER + B is already bound to X`. */
const conflictHolder = (stderr) =>
    (/already bound to (.+)$/m.exec(stderr || '') || [])[1] || 'another binding';

/**
 * Rebind one line, asking first if the combo is taken (`rebind` exits 2 for
 * that, and only that). `--expect-*` carries the combo this page displayed, so a
 * map that changed underneath — another window, a `keybind reset` — is refused
 * rather than applied to whatever is there now.
 */
async function rebind(ctx, b, mods, key) {
    const args = [
        'keybind', 'rebind', '--line', String(b.line),
        '--mods', mods.join(' '), '--key', key,
        '--expect-mods', b.mods, '--expect-key', b.key,
    ];
    const r = await ctx.run(args);
    if (r.code !== 2) return r.code === 0;
    const ok = window.confirm(
        `${comboText(mods, key)} is already bound to ${conflictHolder(r.stderr)}.\n\n`
        + 'Bind it here as well? Both bindings will stay in the map.',
    );
    if (!ok) return false;
    const forced = await ctx.run([...args, '--force']);
    return forced.code === 0;
}

/* ── The page ────────────────────────────────────────────────────────────── */

/** One bind: its combo (click to record a new one), what it does, and — for a
 *  bind that launches something — the command, editable in place. */
function bindRow(ctx, b) {
    const combo = b.editable
        ? el('button.combo', { type: 'button', title: 'Record a new shortcut' }, b.combo)
        : el('span.combo.fixed', {
            title: 'This one is held in conf.d/keybinds.lua — a hold-to-drag bind or a '
                + 'multi-step action, neither of which the override layer can reproduce.',
        }, b.combo);
    combo.classList.toggle('changed', b.overridden);
    if (b.editable) {
        combo.addEventListener('click', async () => {
            const got = await recordCombo(ctx, b);
            if (!got) return;
            if (await rebind(ctx, b, got.mods, got.key)) ctx.reload();
        });
    }

    const body = el('div.keybody', el('span.desc', b.desc || `line ${b.line}`));
    if (b.exec) {
        const cmd = field(b.exec, (v) => {
            if (!v.trim()) {
                cmd.setValue(b.exec); // an empty command would be refused anyway
                return;
            }
            ctx.run([
                'keybind', 'set-action', '--line', String(b.line), '--exec', v,
                '--expect-mods', b.mods, '--expect-key', b.key,
            ], { reload: true });
        });
        cmd.classList.add('keycmd');
        body.append(cmd);
    } else {
        body.append(el('span.keyaction', { title: b.action }, b.action || '—'));
    }

    const r = el('div.keyrow', combo, body);
    if (b.overridden) {
        const undo = el('button.keyreset', { type: 'button', title: 'Back to the shipped default' },
            '↺');
        undo.addEventListener('click', () =>
            ctx.run(['keybind', 'reset', '--line', String(b.line)], { reload: true }));
        r.append(undo);
    }
    return r;
}

export async function keybindsPage(ctx) {
    const sections = await ctx.invoke('tz_keybinds');
    const out = [el('h1.pagetitle', 'Keybinds')];
    if (!sections.length) {
        out.push(el('div.pagehint', 'No keybinds were reported — check `tezca keybind list`.'));
        return out;
    }

    const all = sections.flatMap((s) => s.binds);
    const changed = all.filter((b) => b.overridden).length;

    out.push(el('div.pagehint',
        'Click a shortcut to record a new one. Every change goes to '
        + '~/.config/tezca/keybinds.lua — the shipped map is never edited, so an undo '
        + 'is a line removed rather than a line rewritten.'));

    out.push(section('Your changes'));
    const bar = el('div.rowline');
    const undo = el('button.btn', { type: 'button' }, 'Undo last change');
    undo.addEventListener('click', () => ctx.run(['keybind', 'restore'], { reload: true }));
    const reset = el('button.btn.danger', { type: 'button', disabled: changed === 0 }, 'Reset all');
    reset.addEventListener('click', () => {
        if (window.confirm(`Drop all ${changed} customised binding(s) and go back to the `
            + 'shipped map?')) {
            ctx.run(['keybind', 'reset'], { reload: true });
        }
    });
    bar.append(undo, reset);
    out.push(row(
        changed ? `${changed} of ${all.length} bindings differ from the shipped map`
            : 'Every binding is at its shipped default',
        'Undo steps back one change; reset drops all of them at once.',
        bar,
    ));

    for (const s of sections) {
        out.push(section(s.title));
        const list = el('div.keylist');
        for (const b of s.binds) list.append(bindRow(ctx, b));
        out.push(list);
    }
    return out;
}
