// Spike S3 (throwaway): virtual input devices against grab_accelerator, the prefix grab and replay, in a headless
// GNOME Shell 50.1 (L1, inside podman only). Run by spikes/S3/l1.sh through gnome-shell-test-tool as the automation
// script; it plays the extension's part itself, so no extension is installed.
//
// Two virtual keyboards and two virtual pointers: P stands in for the user's physical devices, R is the replay
// device the extension would own. Every case records what it saw instead of asserting, and the script ends by
// printing one RESULTS line; it throws only on a harness failure.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

// evdev codes (linux/input-event-codes.h)
const K = {ESC: 1, A: 30, X: 45, BACKSLASH: 43, CTRL: 29, ALT: 56, SUPER: 125};
const LAT_RUNS = 10;

const sleep = ms => Scripting.sleep(ms);
const mono = () => GLib.get_monotonic_time();
const ms = us => Math.round(us / 100) / 10;

const results = [];
function note(m) {
    console.log(`[s3] ${m}`);
}
function record(id, data) {
    results.push({id, ...data});
    note(`${id} ${JSON.stringify(data)}`);
}

// --- the client's log ---------------------------------------------------------------------------------------------

const logPath = GLib.getenv('S3_CLIENT_LOG');
function clientEvents() {
    try {
        const [, bytes] = GLib.file_get_contents(logPath);
        return new TextDecoder().decode(bytes).split('\n').filter(l => l).map(l => JSON.parse(l));
    } catch {
        return [];
    }
}
function since(mark, client) {
    return clientEvents().slice(mark).filter(e => e.client === client);
}
function brief(evs) {
    return evs.map(e => {
        switch (e.ev) {
        case 'key-press': case 'key-release': return `${e.ev}:${e.key}${e.state & 8 ? '+alt' : ''}`;
        case 'click-press': case 'click-release': return `${e.ev}:${e.button}@${Math.round(e.x)},${Math.round(e.y)}`;
        case 'modifiers': return `mods:${e.state}`;
        case 'active': return `active:${e.value}`;
        default: return e.ev;
        }
    });
}
async function waitUntil(pred, timeoutMs = 2000) {
    const deadline = mono() + timeoutMs * 1000;
    for (;;) {
        const v = pred();
        if (v)
            return v;
        if (mono() > deadline)
            return null;
        // eslint-disable-next-line no-await-in-loop
        await sleep(2);
    }
}
function waitClient(mark, client, pred, timeoutMs) {
    return waitUntil(() => since(mark, client).find(pred), timeoutMs);
}

// --- devices --------------------------------------------------------------------------------------------------------

const seat = global.stage.context.get_backend().get_default_seat();
// Created in run(), not at import: the module is imported before the shell has started the perf helper.
let kbdP, kbdR, ptrP, ptrR;
const devNames = new Map();

// A virtual device has no getter for the ClutterInputDevice its events carry as source device; the seat announces
// each one with device-added, in creation order (the input thread runs the creations in order).
async function createDevices() {
    const added = [];
    const id = seat.connect('device-added', (_s, device) => added.push(device));
    kbdP = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    kbdR = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    ptrP = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    ptrR = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    await waitUntil(() => added.length >= 4, 3000);
    seat.disconnect(id);
    ['P', 'R', 'ptrP', 'ptrR'].forEach((n, i) => added[i] && devNames.set(added[i], n));
    note(`seat ${seat.constructor.name}; virtual devices ${kbdP.constructor.name}; added ` +
        `${added.map(d => `${d.get_device_name()}:${d.get_device_type()}`).join(', ')}`);
}

const down = (dev, code) => dev.notify_key(mono(), code, Clutter.KeyState.PRESSED);
const up = (dev, code) => dev.notify_key(mono(), code, Clutter.KeyState.RELEASED);
const bdown = (dev, b = 1) => dev.notify_button(mono(), b, Clutter.ButtonState.PRESSED);
const bup = (dev, b = 1) => dev.notify_button(mono(), b, Clutter.ButtonState.RELEASED);
const moveTo = (dev, x, y) => dev.notify_absolute_motion(mono(), x, y);

function devName(device) {
    if (!device)
        return 'null';
    return devNames.get(device) ?? `${device.get_device_name()}`;
}

// --- the extension's part: Alt+A through grab_accelerator, a modal grab actor ---------------------------------------

const ext = {
    action: 0,
    name: null,
    activations: [],
    onActivate: null,
    grab: null,
    actor: null,
    grabEvents: [],
    onGrabEvent: null,
    filterPass: 0,
    filterSeen: [],
};

function grabAlt() {
    ext.action = global.display.grab_accelerator('<Alt>a', Meta.KeyBindingFlags.NONE);
    if (!ext.action)
        throw new Error('grab_accelerator(<Alt>a) failed');
    ext.name = Meta.external_binding_name_for_action(ext.action);
    Main.wm.allowKeybinding(ext.name, Shell.ActionMode.NORMAL);
}
function ungrabAlt() {
    Main.wm.allowKeybinding(ext.name, Shell.ActionMode.NONE);
    global.display.ungrab_accelerator(ext.action);
}

global.display.connect('accelerator-activated', (_d, action, device, _ts) => {
    if (action !== ext.action)
        return;
    ext.activations.push({t: mono(), device: devName(device), modal: Main.modalCount});
    ext.onActivate?.(device);
});

// A second handler on the shell's filter-keybinding signal (true_handled accumulator; the shell's own handler runs
// first and returns false for an allowed binding in NORMAL mode). Returning true makes mutter treat the binding as
// absent for this one event, so the key goes on to the focused client.
global.window_manager.connect('filter-keybinding', (_wm, binding) => {
    if (binding.get_name() !== ext.name)
        return false;
    ext.filterSeen.push({t: mono(), pass: ext.filterPass > 0});
    if (ext.filterPass > 0) {
        ext.filterPass--;
        return true;
    }
    return false;
});

ext.actor = new St.Widget({name: 's3-grab', reactive: true, can_focus: true, width: 1, height: 1, x: 0, y: 0});
Main.layoutManager.uiGroup.add_child(ext.actor);
for (const sig of ['key-press-event', 'key-release-event', 'button-press-event', 'button-release-event']) {
    ext.actor.connect(sig, (_a, event) => {
        const e = {
            t: mono(),
            type: sig.replace('-event', ''),
            code: sig.startsWith('key') ? event.get_key_code() - 8 : event.get_button(),
            state: event.get_state(),
            dev: devName(event.get_source_device()),
        };
        ext.grabEvents.push(e);
        ext.onGrabEvent?.(e);
        return Clutter.EVENT_STOP;
    });
}

function push(mode = Shell.ActionMode.NONE) {
    ext.grab = Main.pushModal(ext.actor, {actionMode: mode});
}
function pop() {
    if (ext.grab) {
        Main.popModal(ext.grab);
        ext.grab = null;
    }
}
function reset() {
    pop();
    ext.onActivate = null;
    ext.onGrabEvent = null;
    ext.filterPass = 0;
}
const grabBrief = evs => evs.map(e => `${e.type}:${e.code}/${e.dev}${e.state & Clutter.ModifierType.MOD1_MASK ? '+alt' : ''}`);

// --- the client window ---------------------------------------------------------------------------------------------

async function startClient(name, backend) {
    const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
    launcher.setenv('GDK_BACKEND', backend, true);
    launcher.setenv('GSK_RENDERER', 'cairo', true);
    launcher.setenv('GTK_A11Y', 'none', true);
    launcher.setenv('NO_AT_BRIDGE', '1', true);
    launcher.unsetenv('GTK_IM_MODULE');
    const mark = clientEvents().length;
    const argv = backend === 'x11'
        ? ['python3', '-I', GLib.getenv('S3_X11_CLIENT'), name]
        : ['gjs', '-m', GLib.getenv('S3_CLIENT_JS'), name];
    const proc = launcher.spawnv(argv);
    const win = await waitUntil(() => global.display.list_all_windows().find(w => w.get_title() === `s3-${name}`),
        20000);
    if (!win) {
        note(`no s3-${name} window; windows: ${global.display.list_all_windows()
            .map(w => `${w.get_title()}/${w.get_wm_class()}`).join(', ')}; X11 class filter ` +
            `${GLib.getenv('MUTTER_WM_CLASS_FILTER')}`);
        proc.force_exit();
        return null;
    }
    await waitClient(mark, name, e => e.ev === 'ready', 10000);
    await sleep(300);
    win.move_frame(true, 200, 150);
    win.activate(global.get_current_time());
    await sleep(500);
    await Scripting.waitLeisure();
    const r = win.get_frame_rect();
    note(`client ${name}: wm_class ${win.get_wm_class()} type ${win.get_client_type()} frame ${r.x},${r.y} ` +
        `${r.width}x${r.height} focused ${global.display.focus_window === win}`);
    return {name, win, proc};
}

function centre(c) {
    const r = c.win.get_frame_rect();
    return [r.x + Math.round(r.width / 2), r.y + Math.round(r.height / 2)];
}

async function focusClient(c) {
    c.win.activate(global.get_current_time());
    await waitUntil(() => global.display.focus_window === c.win, 1000);
    await sleep(50);
}

// Arms the prefix the way the extension will: the accelerator callback pushes the modal synchronously.
function armOnActivate() {
    ext.onActivate = () => {
        if (!ext.grab)
            push(Shell.ActionMode.NONE);
    };
}

async function tapAltA(dev) {
    down(dev, K.ALT);
    down(dev, K.A);
    up(dev, K.A);
    up(dev, K.ALT);
    await sleep(60);
}

// --- cases ----------------------------------------------------------------------------------------------------------

// K1: does a virtual keyboard trigger a grab_accelerator binding?
async function caseK1(c) {
    await focusClient(c);
    const mark = clientEvents().length;
    const a0 = ext.activations.length;
    const t0 = mono();
    await tapAltA(kbdP);
    await waitUntil(() => ext.activations.length > a0, 1000);
    await sleep(200);
    const acts = ext.activations.slice(a0);
    const evs = since(mark, c.name);
    record(`K1/${c.name}`, {
        q: 'virtual Alt+A triggers the grab_accelerator binding',
        activations: acts.length,
        device: acts[0]?.device,
        activateMs: acts[0] ? ms(acts[0].t - t0) : null,
        clientGotA: evs.some(e => e.ev === 'key-press' && /^[aA]$/.test(e.key)),
        client: brief(evs),
    });
}

// K2: prefix, then an unbound key (PassThrough): pop on its release, then replay it through R.
async function caseK2(c, runs) {
    const lat = [];
    let detail = null;
    for (let i = 0; i < runs; i++) {
        // eslint-disable-next-line no-await-in-loop
        await focusClient(c);
        reset();
        armOnActivate();
        const g0 = ext.grabEvents.length;
        const a0 = ext.activations.length;
        let replayT = 0, popT = 0;
        ext.onGrabEvent = e => {
            if (e.type === 'key-release' && e.code === K.X && e.dev === 'P') {
                pop();
                popT = mono();
                replayT = mono();
                down(kbdR, K.X);
                up(kbdR, K.X);
            }
        };
        const mark = clientEvents().length;
        down(kbdP, K.ALT);
        down(kbdP, K.A);
        up(kbdP, K.A);
        up(kbdP, K.ALT);
        // eslint-disable-next-line no-await-in-loop
        const armed = await waitUntil(() => ext.grab, 1000);
        down(kbdP, K.X);
        up(kbdP, K.X);
        // eslint-disable-next-line no-await-in-loop
        const got = await waitClient(mark, c.name, e => e.ev === 'key-press' && e.key === 'x', 1500);
        // eslint-disable-next-line no-await-in-loop
        await sleep(100);
        if (got)
            lat.push(got.t - replayT);
        if (i === 0) {
            detail = {
                armed: !!armed,
                grab: grabBrief(ext.grabEvents.slice(g0)),
                activations: ext.activations.slice(a0).map(a => `${a.device}/modal${a.modal}`),
                client: brief(since(mark, c.name)),
                popToReplayUs: replayT - popT,
            };
        }
        reset();
    }
    record(`K2/${c.name}`, {
        q: 'a PassThrough key taken by the prefix grab is replayed to the app after popModal',
        delivered: `${lat.length}/${runs}`,
        replayToClientMs: stats(lat),
        ...detail,
    });
}

// K3: fast typing: x pressed before the command key's release, still held at the pop.
//  K3a pops on the command key's release and replays x at once (x still held on P);
//  K3b keeps the grab until every key pressed under it is released, then replays.
async function caseK3(c, variant) {
    await focusClient(c);
    reset();
    armOnActivate();
    const held = new Set();
    const g0 = ext.grabEvents.length;
    let replayed = false;
    const replayX = () => {
        replayed = true;
        down(kbdR, K.X);
        up(kbdR, K.X);
    };
    ext.onGrabEvent = e => {
        if (e.type === 'key-press')
            held.add(e.code);
        if (e.type === 'key-release')
            held.delete(e.code);
        if (variant === 'a' && e.type === 'key-release' && e.code === K.BACKSLASH) {
            pop();
            replayX();
        }
        if (variant === 'b' && e.type === 'key-release' && [...held].every(k => k === K.ALT) && ext.grab &&
            ext.grabEvents.slice(g0).some(g => g.code === K.BACKSLASH && g.type === 'key-release')) {
            pop();
            replayX();
        }
    };
    const mark = clientEvents().length;
    await tapAltA(kbdP);
    await waitUntil(() => ext.grab, 1000);
    down(kbdP, K.BACKSLASH);
    await sleep(5);
    down(kbdP, K.X);
    await sleep(5);
    up(kbdP, K.BACKSLASH);
    await sleep(40);
    up(kbdP, K.X);
    await sleep(300);
    const evs = since(mark, c.name);
    record(`K3${variant}/${c.name}`, {
        q: variant === 'a'
            ? 'replay at the command key release, while the taken key is still physically held'
            : 'hold the grab until every key pressed under it is released, then replay',
        replayed,
        grab: grabBrief(ext.grabEvents.slice(g0)),
        client: brief(evs),
        clientGotXPress: evs.some(e => e.ev === 'key-press' && e.key === 'x'),
        clientGotXRelease: evs.some(e => e.ev === 'key-release' && e.key === 'x'),
    });
    reset();
}

// K4: "Alt+A, then Alt+A: send one Alt+A". The second Alt+A arrives in the NONE grab; pop on its release and replay
// Alt+A through R. Strategies: naive (binding still live), allow (Main.wm.allowKeybinding NONE during the replay),
// filter (a one-shot filter-keybinding handler), regrab (ungrab, replay, wait, grab again).
// altHeld: whether P still holds Alt when the replay runs (the common "hold Alt, tap A twice").
async function caseK4(c, strategy, altHeld) {
    await focusClient(c);
    reset();
    armOnActivate();
    const g0 = ext.grabEvents.length;
    const a0 = ext.activations.length;
    const f0 = ext.filterSeen.length;
    const oldAction = ext.action;
    let replayT = 0;
    let replayDone = false;
    const doReplay = () => {
        ext.onActivate = null; // a re-arm is only recorded, not acted on
        if (strategy === 'allow')
            Main.wm.allowKeybinding(ext.name, Shell.ActionMode.NONE);
        if (strategy === 'filter')
            ext.filterPass = 2;
        if (strategy === 'regrab')
            ungrabAlt();
        replayT = mono();
        down(kbdR, K.ALT);
        down(kbdR, K.A);
        up(kbdR, K.A);
        up(kbdR, K.ALT);
        replayDone = true;
    };
    let aPresses = 0;
    ext.onGrabEvent = e => {
        if (e.type === 'key-press' && e.code === K.A)
            aPresses++;
        if (e.type === 'key-release' && e.code === K.A && aPresses >= 1 && ext.grab) {
            const activationsUnderGrab = ext.activations.length - a0;
            ext.k4ActivationsUnderGrab = activationsUnderGrab;
            pop();
            if (altHeld)
                doReplay();
        }
    };
    const mark = clientEvents().length;
    down(kbdP, K.ALT);
    down(kbdP, K.A);
    up(kbdP, K.A);
    await waitUntil(() => ext.grab, 1000);
    down(kbdP, K.A);
    up(kbdP, K.A);
    await waitUntil(() => !ext.grab, 1000);
    if (!altHeld) {
        up(kbdP, K.ALT);
        await sleep(60);
        doReplay();
    }
    const got = await waitClient(mark, c.name, e => e.ev === 'key-press' && /^[aA]$/.test(e.key) && e.t > replayT,
        1000);
    await waitClient(mark, c.name, e => e.ev === 'key-release' && /^[aA]$/.test(e.key) && e.t > replayT, 500);
    if (altHeld)
        up(kbdP, K.ALT);
    await sleep(150);
    // restore
    let regrabbed = null;
    if (strategy === 'allow')
        Main.wm.allowKeybinding(ext.name, Shell.ActionMode.NORMAL);
    if (strategy === 'regrab') {
        grabAlt();
        regrabbed = {oldAction, newAction: ext.action, name: ext.name};
    }
    const leftover = ext.filterPass;
    ext.filterPass = 0;
    const evs = since(mark, c.name);
    record(`K4-${strategy}-${altHeld ? 'altheld' : 'altup'}/${c.name}`, {
        q: 'Alt+A, Alt+A: replay one Alt+A to the app after the pop without re-arming',
        replayDone,
        activationsTotal: ext.activations.length - a0,
        activationsUnderGrab: ext.k4ActivationsUnderGrab,
        reArmedBy: ext.activations.slice(a0 + 1).map(a => a.device),
        clientGotAltA: got ? `${got.key} state ${got.state} alt=${!!(got.state & 8)}` : null,
        replayToClientMs: got ? ms(got.t - replayT) : null,
        filterSeen: ext.filterSeen.slice(f0).map(f => (f.pass ? 'passed' : 'kept')),
        filterPassLeft: leftover,
        regrabbed,
        grab: grabBrief(ext.grabEvents.slice(g0)),
        client: brief(evs),
    });
    // After the case: a fresh Alt+A must still arm (nothing left disabled).
    reset();
    const a1 = ext.activations.length;
    await tapAltA(kbdP);
    await waitUntil(() => ext.activations.length > a1, 1000);
    record(`K4-${strategy}-${altHeld ? 'altheld' : 'altup'}/${c.name}/after`, {
        q: 'Alt+A still arms after the replay',
        armsAgain: ext.activations.length > a1,
    });
    reset();
}

// O1: Alt+A inside a NORMAL (overlay) grab fires the accelerator; does the grab actor also see the key?
// N1: the same inside a NONE grab.
async function caseModes(c) {
    for (const [id, mode] of [['O1', Shell.ActionMode.NORMAL], ['N1', Shell.ActionMode.NONE]]) {
        // eslint-disable-next-line no-await-in-loop
        await focusClient(c);
        reset();
        push(mode);
        const g0 = ext.grabEvents.length;
        const a0 = ext.activations.length;
        const mark = clientEvents().length;
        // eslint-disable-next-line no-await-in-loop
        await tapAltA(kbdP);
        // eslint-disable-next-line no-await-in-loop
        await sleep(150);
        record(`${id}/${c.name}`, {
            q: `virtual Alt+A inside a ${id === 'O1' ? 'NORMAL' : 'NONE'} pushModal`,
            activations: ext.activations.length - a0,
            grab: grabBrief(ext.grabEvents.slice(g0)),
            client: brief(since(mark, c.name)),
        });
        reset();
    }
}

// KC: an addKeybinding chord with NON_MASKABLE (the kill chord), from a virtual keyboard, outside and inside a NONE grab.
let killFired = [];
function addKillChord() {
    const dir = GLib.getenv('S3_SCHEMA_DIR');
    const src = Gio.SettingsSchemaSource.new_from_directory(dir, Gio.SettingsSchemaSource.get_default(), false);
    const settings = new Gio.Settings({settings_schema: src.lookup('org.vase.s3', false)});
    const action = Main.wm.addKeybinding('s3-kill', settings, Meta.KeyBindingFlags.NON_MASKABLE, Shell.ActionMode.ALL,
        (_display, _window, event) => {
            killFired.push({t: mono(), dev: devName(event?.get_source_device?.() ?? null), modal: Main.modalCount});
        });
    note(`addKeybinding s3-kill: action ${action}`);
}
async function tapKill(dev) {
    down(dev, K.CTRL);
    down(dev, K.SUPER);
    down(dev, K.ALT);
    down(dev, K.ESC);
    up(dev, K.ESC);
    up(dev, K.ALT);
    up(dev, K.SUPER);
    up(dev, K.CTRL);
    await sleep(150);
}
async function caseKill(c) {
    await focusClient(c);
    reset();
    killFired = [];
    const mark = clientEvents().length;
    await tapKill(kbdP);
    record(`KC-idle/${c.name}`, {
        q: 'virtual Ctrl+Super+Alt+Esc triggers an addKeybinding NON_MASKABLE chord',
        fired: killFired.length,
        dev: killFired[0]?.dev,
        client: brief(since(mark, c.name)),
    });
    killFired = [];
    push(Shell.ActionMode.NONE);
    const g0 = ext.grabEvents.length;
    await tapKill(kbdP);
    record(`KC-none/${c.name}`, {
        q: 'the same chord inside a NONE grab (F14: filtered, the grab handler must check it)',
        fired: killFired.length,
        grabSawEsc: ext.grabEvents.slice(g0).some(e => e.type === 'key-press' && e.code === K.ESC),
        grab: grabBrief(ext.grabEvents.slice(g0)),
    });
    reset();
}

// P1: a virtual click on the client.
// P2: a click under an overlay grab (NORMAL) reaches the grab actor; pop on its release and replay it through R.
// P3: the same, but popping on the press and replaying while P's button is still down.
async function casePointer(c, runs) {
    await focusClient(c);
    reset();
    const [cx, cy] = centre(c);
    let mark = clientEvents().length;
    const where = () => {
        const [px, py] = global.get_pointer();
        const actor = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, px, py);
        return {pointer: `${px},${py}`, actor: `${actor?.constructor.name}:${actor?.name ?? ''}`,
            overview: Main.overview.visible, modal: Main.modalCount,
            focus: global.display.focus_window?.get_title() ?? null};
    };
    // A fresh virtual pointer starts at (0,0), on the hot corner's pressure barrier: its first absolute motion
    // there ended at x=0 and opened the overview (out/run4-hotcorner.log). Warp off the corner first.
    seat.warp_pointer(640, 600);
    await sleep(80);
    if (Main.overview.visible) {
        Main.overview.hide();
        await sleep(500);
        await focusClient(c);
    }
    const before = where();
    moveTo(ptrP, cx, cy);
    await sleep(80);
    const moved = where();
    bdown(ptrP);
    bup(ptrP);
    await waitClient(mark, c.name, e => e.ev === 'click-release', 1000);
    await sleep(50);
    record(`P1/${c.name}`, {
        q: 'a virtual pointer click reaches the client',
        target: `${cx},${cy}`,
        before,
        moved,
        after: where(),
        client: brief(since(mark, c.name)),
    });
    // The same click again, now that the pointer has moved once.
    await focusClient(c);
    mark = clientEvents().length;
    moveTo(ptrP, cx + 5, cy + 5);
    await sleep(80);
    const moved2 = where();
    bdown(ptrP);
    bup(ptrP);
    await waitClient(mark, c.name, e => e.ev === 'click-release', 1000);
    await sleep(50);
    record(`P1b/${c.name}`, {q: 'a second virtual click', moved: moved2, after: where(),
        client: brief(since(mark, c.name))});

    const lat = [];
    let detail = null;
    for (let i = 0; i < runs; i++) {
        // eslint-disable-next-line no-await-in-loop
        await focusClient(c);
        reset();
        moveTo(ptrP, cx + i, cy);
        // eslint-disable-next-line no-await-in-loop
        await sleep(40);
        push(Shell.ActionMode.NORMAL);
        const g0 = ext.grabEvents.length;
        let replayT = 0;
        ext.onGrabEvent = e => {
            if (e.type === 'button-release' && e.dev === 'ptrP') {
                pop();
                replayT = mono();
                bdown(ptrR);
                bup(ptrR);
            }
        };
        mark = clientEvents().length;
        bdown(ptrP);
        // eslint-disable-next-line no-await-in-loop
        await sleep(10);
        bup(ptrP);
        // eslint-disable-next-line no-await-in-loop
        const got = await waitClient(mark, c.name, e => e.ev === 'click-press', 1500);
        // eslint-disable-next-line no-await-in-loop
        await sleep(80);
        if (got)
            lat.push(got.t - replayT);
        if (i === 0) {
            detail = {
                grab: grabBrief(ext.grabEvents.slice(g0)),
                client: brief(since(mark, c.name)),
            };
        }
        reset();
    }
    record(`P2/${c.name}`, {
        q: 'a click taken by an overlay grab is replayed to the window under the pointer after popModal',
        delivered: `${lat.length}/${runs}`,
        replayToClientMs: stats(lat),
        ...detail,
    });

    await focusClient(c);
    reset();
    push(Shell.ActionMode.NORMAL);
    const g0 = ext.grabEvents.length;
    ext.onGrabEvent = e => {
        if (e.type === 'button-press' && e.dev === 'ptrP') {
            pop();
            bdown(ptrR);
            bup(ptrR);
        }
    };
    mark = clientEvents().length;
    bdown(ptrP);
    await sleep(60);
    bup(ptrP);
    await sleep(250);
    record(`P3/${c.name}`, {
        q: 'replay while the physical button is still down',
        grab: grabBrief(ext.grabEvents.slice(g0)),
        client: brief(since(mark, c.name)),
    });
    reset();
}

function stats(arr) {
    if (!arr.length)
        return null;
    const s = [...arr].sort((a, b) => a - b);
    return {n: s.length, min: ms(s[0]), median: ms(s[Math.floor(s.length / 2)]), max: ms(s[s.length - 1])};
}

async function suite(c) {
    const cases = [
        () => caseK1(c),
        () => caseK2(c, LAT_RUNS),
        () => caseK3(c, 'a'),
        () => caseK3(c, 'b'),
        () => caseK4(c, 'naive', true),
        () => caseK4(c, 'allow', true),
        () => caseK4(c, 'filter', true),
        () => caseK4(c, 'filter', false),
        () => caseK4(c, 'regrab', true),
        () => caseModes(c),
        () => caseKill(c),
        () => casePointer(c, LAT_RUNS),
    ];
    for (const run of cases) {
        try {
            // eslint-disable-next-line no-await-in-loop
            await run();
        } catch (e) {
            record('ERROR', {client: c.name, error: `${e}`, stack: e.stack});
            reset();
        }
        // eslint-disable-next-line no-await-in-loop
        await sleep(100);
    }
}

/** Runs the spike. */
export async function run() {
    note('run() entered');
    await createDevices();
    if (Main.overview.visible) {
        Main.overview.hide();
        await sleep(500);
    }
    await Scripting.waitLeisure();
    grabAlt();
    note(`grab_accelerator <Alt>a: action ${ext.action} name ${ext.name}`);
    addKillChord();

    for (const [name, backend] of [['wayland', 'wayland'], ['xwayland', 'x11']]) {
        // eslint-disable-next-line no-await-in-loop
        const c = await startClient(name, backend);
        if (!c) {
            record(`START/${name}`, {error: 'the client window never appeared'});
            continue;
        }
        // eslint-disable-next-line no-await-in-loop
        await suite(c);
        c.win.delete(global.get_current_time());
        // eslint-disable-next-line no-await-in-loop
        await sleep(300);
        c.proc.force_exit();
    }
    note(`RESULTS ${JSON.stringify(results)}`);
    note('DONE');
}
