// Spike S6 (throwaway): Meta.Window::configure on Wayland and Xwayland windows in GNOME Shell 50.1.
//
// Runs as a gnome-shell-test-tool automation script at L1 (spikes/S6/run.sh, inside chained-tiler-test). For each
// case it starts one client, logs every signal of the new window in order (window-created, configure,
// position-changed, size-changed, shown, the actor's first-frame) with the window's rects at that moment, and in
// some cases changes the initial MetaWindowConfig from the configure handler. Each line is "[s6] " + JSON;
// run.sh collects them, report.py turns them into the tables in docs/linux-spike/S6.md.

import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import Mtk from 'gi://Mtk';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

const T0 = GLib.get_monotonic_time();
let seq = 0;

function out(obj) {
    console.log(`[s6] ${JSON.stringify(obj)}`);
}

function r(rect) {
    if (Array.isArray(rect))
        rect = rect.find(v => v && typeof v === 'object' && 'width' in v) ?? rect[0];
    return rect && typeof rect === 'object' ? [rect.x, rect.y, rect.width, rect.height] : null;
}

function tryCall(f) {
    try {
        return f();
    } catch (e) {
        return `throws: ${e.message}`;
    }
}

function us() {
    return GLib.get_monotonic_time() - T0;
}

function clientType(w) {
    return w.get_client_type() === Meta.WindowClientType.WAYLAND ? 'wayland' : 'x11';
}

function windowState(w) {
    const actor = w.get_compositor_private();
    return {
        frame: r(w.get_frame_rect()),
        buffer: r(w.get_buffer_rect()),
        actor: actor ? [Math.round(actor.x), Math.round(actor.y), Math.round(actor.width),
            Math.round(actor.height)] : null,
        actorVisible: actor ? actor.visible : null,
        hidden: w.is_hidden(),
        maximized: w.is_maximized(),
        fullscreen: w.is_fullscreen(),
        monitor: w.get_monitor(),
    };
}

function identity(w) {
    return {
        title: w.get_title(),
        wmClass: w.get_wm_class(),
        wmClassInstance: w.get_wm_class_instance(),
        gtkAppId: w.get_gtk_application_id(),
        sandboxedAppId: w.get_sandboxed_app_id(),
        pid: w.get_pid(),
        windowType: w.get_window_type(),
        transient: w.get_transient_for() !== null,
    };
}

// The case being run. mode: control (configure only observed), possize (set_position then set_size), rect
// (set_rect with an Mtk.Rectangle; run it on its own, it passes garbage), position (set_position only), size
// (set_size only), late (configure connected from an idle after window-created, then possize), offwork (possize
// with a rect that starts above the work area and runs off the right edge, to see the constraints).
let current = null;

function target() {
    const wa = Main.layoutManager.getWorkAreaForMonitor(0);
    return {wa: r(wa), rect: [wa.x + 120, wa.y + 70, 600, 400]};
}

function onConfigure(w, config, how) {
    const c = current;
    // Never config.get_rect() from GJS: it returns MtkRectangle by value, which GJS reads as a pointer (null at
    // (0,0), a segfault otherwise; spikes/S6/gjs-probe.sh). get_position()/get_size() lack (out) annotations.
    const ev = {
        ev: 'configure', seq: seq++, t: us(), id: String(w.get_id()), how,
        initial: config.get_is_initial(), configFullscreen: config.get_is_fullscreen(),
        configPos: tryCall(() => config.get_position()), configSize: tryCall(() => config.get_size()),
        ...identity(w), ...windowState(w),
    };
    if (c && c.window === w && config.get_is_initial()) {
        const [x, y, width, height] = c.mode === 'offwork'
            ? [c.target.wa[0] + 900, c.target.wa[1] - 20, 600, 400] : c.target.rect;
        if (c.mode === 'rect') {
            config.set_rect(new Mtk.Rectangle({x, y, width, height}));
        } else if (c.mode === 'possize' || c.mode === 'late' || c.mode === 'offwork') {
            config.set_position(x, y);
            config.set_size(width, height);
        } else if (c.mode === 'position')
            config.set_position(x, y);
        else if (c.mode === 'size')
            config.set_size(width, height);
        ev.set = c.mode === 'control' ? null : [x, y, width, height];
        c.configureSeen = true;
    }
    out(ev);
}

function track(w) {
    const id = String(w.get_id());
    const log = (ev, extra = {}) => out({ev, seq: seq++, t: us(), id, ...extra, ...windowState(w)});
    const ids = [];
    ids.push(w.connect('position-changed', () => log('position-changed')));
    ids.push(w.connect('size-changed', () => log('size-changed')));
    ids.push(w.connect('shown', () => log('shown')));
    ids.push(w.connect('notify::maximized-horizontally', () => log('notify::maximized-horizontally')));
    const actor = w.get_compositor_private();
    if (actor) {
        actor.connect('first-frame', () => {
            log('first-frame');
            if (current && current.window === w)
                current.firstFrame = true;
        });
    }
    w.connect('unmanaged', () => {
        log('unmanaged');
        if (current && current.window === w)
            current.gone = true;
    });
    return ids;
}

global.display.connect('window-created', (_d, w) => {
    out({
        ev: 'window-created', seq: seq++, t: us(), id: String(w.get_id()), client: clientType(w),
        hasActor: w.get_compositor_private() !== null, ...identity(w), ...windowState(w),
    });
    const c = current;
    const ours = c && !c.window && w.get_window_type() === Meta.WindowType.NORMAL;
    if (ours)
        c.window = w;
    track(w);
    if (ours && c.mode === 'late') {
        // What happens to a handler connected one main-loop turn too late.
        GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            out({ev: 'late-connect', seq: seq++, t: us(), id: String(w.get_id()), ...windowState(w)});
            w.connect('configure', (win, config) => onConfigure(win, config, 'late'));
            return GLib.SOURCE_REMOVE;
        });
    } else {
        w.connect('configure', (win, config) => onConfigure(win, config, 'in-window-created'));
    }
});

async function waitFor(pred, ms) {
    const deadline = GLib.get_monotonic_time() + ms * 1000;
    while (!pred()) {
        if (GLib.get_monotonic_time() > deadline)
            return false;
        // eslint-disable-next-line no-await-in-loop
        await Scripting.sleep(20);
    }
    return true;
}

function spawn(argv, x11) {
    const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.NONE});
    if (x11) {
        launcher.unsetenv('WAYLAND_DISPLAY');
        launcher.setenv('GDK_BACKEND', 'x11', true);
    } else {
        launcher.setenv('GDK_BACKEND', 'wayland', true);
    }
    launcher.setenv('GTK_A11Y', 'none', true);
    return launcher.spawnv(argv);
}

const CLIENTS = {
    // gnome-shell-perf-helper: a GTK4 window over Wayland, made through the test tool's own D-Bus helper.
    'perf-wl': {
        start: () => Scripting.createTestWindow({width: 640, height: 480}),
        stop: () => Scripting.destroyTestWindows(),
    },
    'perf-wl-max': {
        start: () => Scripting.createTestWindow({width: 640, height: 480, maximized: true}),
        stop: () => Scripting.destroyTestWindows(),
    },
    'gtk4demo-wl': {argv: ['gtk4-demo'], x11: false},
    'gtk4demo-x11': {argv: ['gtk4-demo'], x11: true},
    'xlogo-x11': {argv: ['xlogo', '-geometry', '300x200'], x11: true},
    'xlogo-x11-pos': {argv: ['xlogo', '-geometry', '300x200+40+300'], x11: true},
};

async function runCase(clientName, mode) {
    const spec = CLIENTS[clientName];
    current = {client: clientName, mode, target: target(), window: null, firstFrame: false, gone: false};
    out({ev: 'case', seq: seq++, t: us(), client: clientName, mode, target: current.target});
    let proc = null;
    if (spec.start)
        await spec.start();
    else
        proc = spawn(spec.argv, spec.x11);

    const mapped = await waitFor(() => current.firstFrame, 30000);
    await Scripting.sleep(1500);
    // waitLeisure() can hang at startup in this container, so it is bounded.
    await Promise.race([Scripting.waitLeisure(), Scripting.sleep(5000)]);
    const w = current.window;
    out({
        ev: 'settled', seq: seq++, t: us(), client: clientName, mode, firstFrame: mapped,
        configureSeen: Boolean(current.configureSeen), id: w ? String(w.get_id()) : null,
        ...(w ? {...identity(w), ...windowState(w)} : {}),
    });

    if (spec.stop)
        await spec.stop();
    else if (proc)
        proc.force_exit();
    if (w)
        await waitFor(() => current.gone, 10000);
    await Scripting.sleep(300);
    current = null;
}

export async function run() {
    out({ev: 'run', t: us(), overview: Main.overview.visible});
    if (Main.overview.visible) {
        Main.overview.hide();
        await Scripting.sleep(500);
    }
    out({ev: 'overview-hidden', t: us(), overview: Main.overview.visible});
    const mon = Main.layoutManager.primaryMonitor;
    out({
        ev: 'start', mutter: Meta.MAJOR_VERSION ?? null, monitor: [mon.x, mon.y, mon.width, mon.height],
        scale: mon.geometry_scale, workArea: r(Main.layoutManager.getWorkAreaForMonitor(0)),
        display: GLib.getenv('DISPLAY'), wayland: GLib.getenv('WAYLAND_DISPLAY'),
    });

    const modes = (GLib.getenv('S6_MODES') || 'control,possize,position,size,late,offwork').split(',');
    const clients = (GLib.getenv('S6_CLIENTS') || Object.keys(CLIENTS).join(',')).split(',');
    for (const clientName of clients) {
        for (const mode of modes) {
            try {
                // eslint-disable-next-line no-await-in-loop
                await runCase(clientName, mode);
            } catch (e) {
                out({ev: 'error', client: clientName, mode, error: String(e), stack: e.stack});
                current = null;
            }
        }
    }
    out({ev: 'done'});
}
