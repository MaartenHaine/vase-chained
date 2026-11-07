// Spike S4 (throwaway): move_frame then move_resize_frame on real clients, in a headless GNOME Shell 50.1 with two
// virtual monitors laid out like the owner's desktop (docs/linux-spike/S4.md). Run by spikes/S4/run.sh, inside the
// chained-tiler-s4 container only, as gnome-shell's --automation-script.
//
// Environment:
//   S4_LAYOUT   'xml' (monitors.xml in the throwaway config dir decides), 'dbus' (this script calls
//               ApplyMonitorsConfig), or 'none' (leave mutter's default, record it, measure nothing)
//   S4_SCALE    the scale wanted for Meta-0 (the 4K-like monitor) when S4_LAYOUT=dbus
//   S4_CLIENTS  comma-separated client names (see CLIENTS)
//
// Every result is one line `[s4] <json>` in the shell log.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

const QUIET_MS = Number(GLib.getenv('S4_QUIET_MS') || 2000); // no size-changed or position-changed for this long: settled
const STEP_TIMEOUT_MS = Math.max(12000, QUIET_MS + 4000);
const MAP_TIMEOUT_MS = 40000;

const CLIENTS = {
    gtk4: {argv: ['gtk4-demo'], env: {GDK_BACKEND: 'wayland'}, kind: 'GTK4'},
    adw: {argv: ['adwaita-1-demo'], env: {GDK_BACKEND: 'wayland'}, kind: 'libadwaita'},
    ghostty: {
        argv: ['ghostty', '--gtk-single-instance=false', '--window-decoration=auto', '--font-family=DejaVu Sans Mono',
            '--font-size=11', '-e', 'sleep', '600'],
        env: {GDK_BACKEND: 'wayland'}, kind: 'GTK4 terminal (Ghostty)',
    },
    xterm: {argv: ['xterm', '-fa', 'DejaVu Sans Mono', '-fs', '11', '-e', 'sleep', '600'], env: {}, kind: 'Xwayland'},
    xterm2x: {
        // Font doubled, standing in for Xft.dpi 192, which gsd-xsettings would set on the real desktop when the X11
        // scale is 2 (mutter only publishes the factor, meta-x11-display.c update_ui_scaling_factor).
        argv: ['xterm', '-fa', 'DejaVu Sans Mono', '-fs', '22', '-e', 'sleep', '600'], env: {}, kind: 'Xwayland',
    },
};

const now = () => GLib.get_monotonic_time() / 1000; // ms

/**
 * @param {string} what
 * @param {object} data
 */
function emit(what, data = {}) {
    console.log(`[s4] ${JSON.stringify({what, ...data})}`);
}

/**
 * @param {Meta.Window} w
 * @returns {object}
 */
function frame(w) {
    const r = w.get_frame_rect();
    return {x: r.x, y: r.y, w: r.width, h: r.height};
}

/**
 * @param {Meta.Window} w
 * @returns {object}
 */
function buffer(w) {
    const r = w.get_buffer_rect();
    return {x: r.x, y: r.y, w: r.width, h: r.height};
}

/** @returns {object} the monitors as the shell sees them */
function monitors() {
    const lm = Main.layoutManager;
    const ws = global.workspace_manager.get_active_workspace();
    return {
        primaryIndex: lm.primaryIndex,
        monitors: lm.monitors.map(m => {
            const wa = ws.get_work_area_for_monitor(m.index);
            return {
                index: m.index, x: m.x, y: m.y, width: m.width, height: m.height,
                geometryScale: m.geometry_scale, displayScale: global.display.get_monitor_scale(m.index),
                workArea: {x: wa.x, y: wa.y, w: wa.width, h: wa.height},
            };
        }),
        stage: {w: global.stage.width, h: global.stage.height},
        layoutMode: global.backend.get_monitor_manager().get_layout_mode?.() ?? null,
    };
}

/**
 * @param {string} method
 * @param {GLib.Variant|null} params
 * @returns {Promise<GLib.Variant>}
 */
function displayConfig(method, params) {
    // Asynchronous: the shell itself answers org.gnome.Mutter.DisplayConfig, so a sync call would deadlock.
    return new Promise((resolve, reject) => {
        Gio.DBus.session.call('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
            'org.gnome.Mutter.DisplayConfig', method, params, null, Gio.DBusCallFlags.NONE, -1, null,
            (conn, res) => {
                try {
                    resolve(conn.call_finish(res));
                } catch (e) {
                    reject(e);
                }
            });
    });
}

/** @returns {Promise<object>} GetCurrentState, unpacked */
async function currentState() {
    const v = await displayConfig('GetCurrentState', null);
    const [serial, mons, logical, props] = v.recursiveUnpack();
    return {
        serial,
        monitors: mons.map(([[connector, vendor, product, serialNo], modes, mprops]) => ({
            connector, vendor, product, serial: serialNo,
            modes: modes.map(([id, w, h, rate, preferredScale, supportedScales, modeProps]) => ({
                id, w, h, rate, preferredScale, supportedScales: supportedScales.map(s => Math.round(s * 1e4) / 1e4),
                current: !!modeProps['is-current'], preferred: !!modeProps['is-preferred'],
            })),
            builtin: !!mprops['is-builtin'], displayName: mprops['display-name'],
        })),
        logical: logical.map(([x, y, scale, transform, primary, specs]) => ({
            x, y, scale, transform, primary, connectors: specs.map(s => s[0]),
        })),
        layoutMode: props['layout-mode'], supportsChangingLayoutMode: props['supports-changing-layout-mode'],
        globalScaleRequired: props['global-scale-required'],
    };
}

/**
 * @param {object} state from currentState()
 * @param {number} scale0 the scale for Meta-0
 * @returns {Promise<void>}
 */
async function applyOwnerLayout(state, scale0) {
    const mode = c => state.monitors.find(m => m.connector === c).modes.find(m => m.current) ??
        state.monitors.find(m => m.connector === c).modes[0];
    const m0 = mode('Meta-0');
    const logicalWidth0 = Math.round(m0.w / scale0);
    const args = new GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})', [
        state.serial, 1 /* temporary */,
        [
            [0, 0, scale0, 0, false, [['Meta-0', m0.id, {}]]],
            [logicalWidth0, 0, 1.0, 0, true, [['Meta-1', mode('Meta-1').id, {}]]],
        ],
        {'layout-mode': new GLib.Variant('u', 1 /* logical */)},
    ]);
    const changed = new Promise(resolve => {
        const id = Main.layoutManager.connect('monitors-changed', () => {
            Main.layoutManager.disconnect(id);
            resolve();
        });
    });
    const t0 = now();
    await displayConfig('ApplyMonitorsConfig', args);
    await changed;
    emit('apply', {scale0, ms: Math.round(now() - t0)});
    await Scripting.sleep(500);
}

/**
 * @param {string} name
 * @returns {Promise<Meta.Window>}
 */
async function launch(name) {
    const c = CLIENTS[name];
    // S4_WAYLAND_DEBUG=1: the client's protocol trace (configure, ack_configure, commit) goes to the shell log.
    const trace = !!GLib.getenv('S4_WAYLAND_DEBUG');
    const launcher = new Gio.SubprocessLauncher({flags: trace ? Gio.SubprocessFlags.NONE
        : Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_MERGE});
    if (trace)
        launcher.setenv('WAYLAND_DEBUG', '1', true);
    for (const [k, v] of Object.entries(c.env))
        launcher.setenv(k, v, true);
    launcher.setenv('LIBGL_ALWAYS_SOFTWARE', '1', true);
    const window = new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            global.display.disconnect(id);
            reject(new Error(`${name}: no window in ${MAP_TIMEOUT_MS} ms`));
        }, MAP_TIMEOUT_MS);
        const id = global.display.connect('window-created', (_d, w) => {
            if (w.get_window_type() !== Meta.WindowType.NORMAL || w.get_transient_for())
                return;
            global.display.disconnect(id);
            clearTimeout(timer);
            resolve(w);
        });
    });
    const t0 = now();
    const proc = launcher.spawnv(c.argv);
    const w = await window;
    const created = now();
    const actor = w.get_compositor_private();
    await new Promise(resolve => {
        const id = actor.connect('first-frame', () => {
            actor.disconnect(id);
            resolve();
        });
    });
    const firstFrame = now();
    await quiet(w, 1500);
    emit('mapped', {
        client: name, kind: c.kind, x11: w.get_client_type() === Meta.WindowClientType.X11,
        wmClass: w.get_wm_class(), description: w.get_description(),
        createdMs: Math.round(created - t0), firstFrameMs: Math.round(firstFrame - t0),
        frame: frame(w), buffer: buffer(w), monitor: w.get_monitor(),
        minSize: w.get_min_size?.() ?? null, maxSize: w.get_max_size?.() ?? null,
        allowsResize: w.allows_resize(), allowsMove: w.allows_move(),
        maximized: w.is_maximized(),
    });
    w._s4proc = proc;
    return w;
}

/**
 * Waits until the window has had no size or position change for `ms`.
 *
 * @param {Meta.Window} w
 * @param {number} ms
 */
async function quiet(w, ms) {
    let last = now();
    const ids = ['size-changed', 'position-changed'].map(s => w.connect(s, () => {
        last = now();
    }));
    const deadline = now() + STEP_TIMEOUT_MS;
    while (now() - last < ms && now() < deadline)
        // eslint-disable-next-line no-await-in-loop
        await Scripting.sleep(50);
    ids.forEach(id => w.disconnect(id));
}

/**
 * One tiling request and every frame change after it, until nothing changed for QUIET_MS.
 *
 * mode 'mf+mrf': move_frame(true, x, y), then move_resize_frame(true, x, y, w, h) (Tiling Assistant's order)
 * mode 'mrf':    move_resize_frame alone
 * mode 'overlap': 'mf+mrf' to `first`, then at once 'mf+mrf' to `target`, as a fast relayout would
 *
 * @param {string} client
 * @param {Meta.Window} w
 * @param {object} target {label, x, y, w, h, mode, first}
 */
async function step(client, w, target) {
    const {label, mode = 'mf+mrf'} = target;
    emit('step-start', {client, label});
    const events = [];
    const t0 = now();
    let last = t0;
    const ids = ['size-changed', 'position-changed'].map(s => w.connect(s, () => {
        last = now();
        events.push({sig: s === 'size-changed' ? 'size' : 'pos', t: Math.round((now() - t0) * 10) / 10, ...frame(w)});
    }));
    const before = frame(w);
    const monitorBefore = w.get_monitor();
    let afterFirst = null;
    if (mode === 'overlap') {
        const f = target.first;
        w.move_frame(true, f.x, f.y);
        w.move_resize_frame(true, f.x, f.y, f.w, f.h);
        afterFirst = frame(w);
        if (target.delay)
            await Scripting.sleep(target.delay);
        emit('second-request', {client, label});
    }
    const tSecond = now() - t0;
    if (mode !== 'mrf')
        w.move_frame(true, target.x, target.y);
    const afterMove = frame(w);
    w.move_resize_frame(true, target.x, target.y, target.w, target.h);
    const afterCall = frame(w);
    const callMs = now() - t0;

    const deadline = t0 + STEP_TIMEOUT_MS;
    while (now() - last < QUIET_MS && now() < deadline)
        // eslint-disable-next-line no-await-in-loop
        await Scripting.sleep(25);
    ids.forEach(id => w.disconnect(id));

    const final = frame(w);
    const same = (a, b) => a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
    const sizeEvents = events.filter(e => e.sig === 'size');
    const settleMs = events.length ? events[events.length - 1].t : 0;
    // The first time the frame was at its final rect and stayed there.
    let settledAt = 0;
    for (let i = events.length - 1; i >= 0 && same(events[i], final); i--)
        settledAt = events[i].t;
    const reached = same(final, target);
    // Did the window show an older request's rect after the newer one was made?
    const visitedFirst = mode === 'overlap' &&
        events.some(e => e.x === target.first.x && e.y === target.first.y);
    // After the newer request was made: did a late ack put the window back at the older request's position?
    const jumpBack = mode === 'overlap' &&
        events.some(e => e.t > tSecond && e.x === target.first.x && e.y === target.first.y);
    emit('step', {
        client, label, mode, target: {x: target.x, y: target.y, w: target.w, h: target.h},
        first: target.first ?? null, delay: target.delay ?? 0, tSecond: Math.round(tSecond * 10) / 10, jumpBack, before, afterFirst, afterMove, afterCall, final, reached, visitedFirst,
        delta: {x: final.x - target.x, y: final.y - target.y, w: final.w - target.w, h: final.h - target.h},
        monitorBefore, monitorAfter: w.get_monitor(), scaleAfter: global.display.get_monitor_scale(w.get_monitor()),
        callMs: Math.round(callMs * 10) / 10, settleMs, settledAt, sizeChanges: sizeEvents.length,
        timedOut: now() >= deadline, buffer: buffer(w), events,
    });
}

/**
 * @param {Meta.Window} w
 * @returns {Promise<string>} xprop's WM_NORMAL_HINTS for an X11 window
 */
function xprop(w) {
    return new Promise(resolve => {
        try {
            const p = Gio.Subprocess.new(['xprop', '-id', w.get_description(), 'WM_NORMAL_HINTS'],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_MERGE);
            p.communicate_utf8_async(null, null, (_p, res) => {
                try {
                    resolve(p.communicate_utf8_finish(res)[1]);
                } catch (e) {
                    resolve(`error: ${e.message}`);
                }
            });
        } catch (e) {
            resolve(`error: ${e.message}`);
        }
    });
}

/** @returns {Promise<string>} xwininfo -root, for the X11 root size (X pixels) */
function xroot() {
    return new Promise(resolve => {
        const p = Gio.Subprocess.new(['xwininfo', '-root'],
            Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_MERGE);
        p.communicate_utf8_async(null, null, (_p, res) => {
            const out = p.communicate_utf8_finish(res)[1];
            resolve(out.split('\n').filter(l => /Width|Height/.test(l)).map(l => l.trim()).join('; '));
        });
    });
}

/**
 * The targets, in stage coordinates, from the work areas: like vase's panes, with odd sizes and offsets so that
 * cell snapping and rounding show.
 *
 * @returns {object[]}
 */
function targets() {
    const lm = Main.layoutManager;
    const ws = global.workspace_manager.get_active_workspace();
    const prim = lm.primaryIndex;
    const other = lm.monitors.findIndex(m => m.index !== prim);
    const wa = i => ws.get_work_area_for_monitor(i);
    const p = wa(prim), o = wa(other);
    const half = Math.floor(p.width / 2);
    const ohalf = Math.floor(o.width / 2);
    return [
        {label: 'primary-left-half', mon: prim, x: p.x + 4, y: p.y + 4, w: half - 6, h: p.height - 8},
        {label: 'primary-right-half', mon: prim, x: p.x + half + 2, y: p.y + 4, w: p.width - half - 6, h: p.height - 8},
        {label: 'other-left-half', mon: other, x: o.x + 4, y: o.y + 4, w: ohalf - 6, h: o.height - 8},
        {label: 'other-odd-quarter', mon: other, x: o.x + ohalf + 3, y: o.y + 7, w: ohalf - 11, h: Math.floor(o.height / 2) - 13},
        {label: 'back-to-primary-top-right', mon: prim, x: p.x + half + 1, y: p.y + 1, w: p.width - half - 3, h: Math.floor(p.height / 2) - 3},
        // move_resize_frame alone, across monitors (from the primary to the other one)
        {label: 'mrf-only-to-other', mode: 'mrf', x: o.x + 10, y: o.y + 10, w: ohalf - 20, h: o.height - 20},
        // two requests back to back: first to the primary, then at once back to the other monitor
        {label: 'overlap-primary-then-other', mode: 'overlap', x: o.x + ohalf + 2, y: o.y + 2, w: ohalf - 4,
            h: o.height - 4, first: {x: p.x + 2, y: p.y + 2, w: half - 4, h: p.height - 4}},
        // and the same within one monitor
        {label: 'overlap-within-other', mode: 'overlap', x: o.x + 2, y: o.y + 2, w: ohalf - 4, h: o.height - 4,
            first: {x: o.x + ohalf + 2, y: o.y + 2, w: ohalf - 4, h: Math.floor(o.height / 2) - 4}},
    ].concat(GLib.getenv('S4_DELAYS') ? GLib.getenv('S4_DELAYS').split(',').map(Number).flatMap(delay => [
        // the older request's configure is in flight when the newer one is made
        {label: `overlap-${delay}ms-other-then-primary`, mode: 'overlap', delay, x: p.x + half + 2, y: p.y + 2,
            w: p.width - half - 4, h: p.height - 4, first: {x: o.x + 2, y: o.y + 2, w: ohalf - 4, h: o.height - 4}},
        {label: `overlap-${delay}ms-primary-then-other`, mode: 'overlap', delay, x: o.x + ohalf + 2, y: o.y + 2,
            w: ohalf - 4, h: o.height - 4, first: {x: p.x + 2, y: p.y + 2, w: half - 4, h: Math.floor(p.height / 2)}},
        // the same within one monitor: the older request's ack changes no monitor
        {label: `overlap-${delay}ms-within-other`, mode: 'overlap', delay, x: o.x + 2, y: o.y + 2, w: ohalf - 4,
            h: o.height - 4, first: {x: o.x + ohalf + 2, y: o.y + 2, w: ohalf - 4, h: Math.floor(o.height / 2) - 4}},
        {label: `overlap-${delay}ms-within-primary`, mode: 'overlap', delay, x: p.x + 2, y: p.y + 2, w: half - 4,
            h: p.height - 4, first: {x: p.x + half + 2, y: p.y + 2, w: p.width - half - 4, h: Math.floor(p.height / 2)}},
    ]) : []);
}

/** Runs the spike. */
export async function run() {
    const layout = GLib.getenv('S4_LAYOUT') ?? 'xml';
    emit('env', {
        layout, workspacesOnlyOnPrimary: new Gio.Settings({schema_id: 'org.gnome.mutter'}).get_boolean('workspaces-only-on-primary'),
        loadavg: new TextDecoder().decode(GLib.file_get_contents('/proc/loadavg')[1]).trim(),
        experimentalFeatures: new Gio.Settings({schema_id: 'org.gnome.mutter'}).get_strv('experimental-features'),
        x11Display: GLib.getenv('DISPLAY'), waylandDisplay: GLib.getenv('WAYLAND_DISPLAY'),
    });
    await Scripting.waitLeisure();
    if (Main.overview.visible) {
        Main.overview.hide();
        await Scripting.sleep(500);
        await Scripting.waitLeisure();
    }

    const startState = await currentState();
    emit('state-at-start', startState);
    emit('monitors-at-start', monitors());

    if (layout === 'dbus') {
        const scale0 = Number(GLib.getenv('S4_SCALE') ?? '1.5');
        try {
            await applyOwnerLayout(startState, scale0);
        } catch (e) {
            emit('apply-error', {scale0, error: e.message});
            throw e;
        }
        emit('state-after-apply', await currentState());
        emit('monitors-after-apply', monitors());
    }
    if (layout === 'none')
        return;

    emit('xwayland-root', {root: await xroot(), stage: {w: global.stage.width, h: global.stage.height}});

    const names = (GLib.getenv('S4_CLIENTS') ?? 'gtk4,adw,ghostty,xterm').split(',').filter(Boolean);
    for (const name of names) {
        let w = null;
        try {
            // eslint-disable-next-line no-await-in-loop
            w = await launch(name);
            if (w.get_client_type() === Meta.WindowClientType.X11)
                // eslint-disable-next-line no-await-in-loop
                emit('x11-hints', {client: name, hints: await xprop(w)});
            for (const t of targets()) {
                // eslint-disable-next-line no-await-in-loop
                await step(name, w, t);
            }
            if (w.get_client_type() === Meta.WindowClientType.X11)
                // eslint-disable-next-line no-await-in-loop
                emit('x11-hints-after', {client: name, hints: await xprop(w)});
        } catch (e) {
            emit('client-error', {client: name, error: e.message, stack: e.stack});
        } finally {
            if (w) {
                const gone = new Promise(resolve => {
                    const id = w.connect('unmanaged', () => resolve(id));
                    setTimeout(resolve, 3000);
                });
                w._s4proc?.force_exit();
                // eslint-disable-next-line no-await-in-loop
                await gone;
            }
        }
    }
    emit('done');
}
