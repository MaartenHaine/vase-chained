// Spike S9's L1 case (docs/linux-spike/S9.md), inside a headless GNOME Shell 50.1 in chained-tiler-test. Throwaway.
//
// The S9 extension starts enabled with no daemon running, so it is already backing off. This script then starts the
// stub daemon (spikes/S9/stub) and drives every case: backoff and reconnect, the peer check, press-to-effect of
// Alt+A then `\` under three paint loads, echo round trips, framing (split lines, garbage, an 8 MB line), EOF and
// SIGKILL, a stopped daemon (missed reply, a sync-write stall, the write cap), a version mismatch, socket
// activation, an upgraded binary, an unreadable binary, and the enable/disable cycle with the leak checker.
// It prints `[s9] RESULT <name> <json>` lines; spikes/S9/l1/run.sh keeps them.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import {ExtensionState} from 'resource:///org/gnome/shell/misc/extensionUtils.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

import {LeakChecker, snapshot} from '../../../vase-gnome/tests/l1/leakcheck.js';

const UUID = 'vase-s9@maartenhaine.github.io';
const STUB = '/opt/s9/release/s9-stub';
const RUNTIME = GLib.get_user_runtime_dir();
const SOCKET = `${RUNTIME}/vase/gnome.sock`;
const KEY_LEFTALT = 56, KEY_A = 30, KEY_BACKSLASH = 43;
const N_PRESSES = 40;
const N_PINGS = 200;
const SIGTERM = 15, SIGSTOP = 19, SIGCONT = 18;

const now = () => GLib.get_monotonic_time();
const note = m => console.log(`[s9] ${m}`);
const failures = [];

function result(name, value) {
    console.log(`[s9] RESULT ${name} ${JSON.stringify(value)}`);
}

function check(cond, what) {
    note(`${cond ? 'ok' : 'FAILED'}: ${what}`);
    if (!cond)
        failures.push(what);
}

const sleep = ms => Scripting.sleep(ms);

async function until(fn, ms, what) {
    const end = now() + ms * 1000;
    while (!fn()) {
        if (now() > end) {
            if (what)
                note(`timed out after ${ms} ms waiting for ${what}`);
            return false;
        }
        // eslint-disable-next-line no-await-in-loop
        await sleep(2);
    }
    return true;
}

function ext() {
    return Main.extensionManager.lookup(UUID).stateObj;
}

function eventsSince(t, kind) {
    return ext().events.filter(e => e.t >= t && (!kind || e.kind === kind));
}

function stats(xs) {
    const v = xs.filter(x => typeof x === 'number' && Number.isFinite(x)).sort((a, b) => a - b);
    if (v.length === 0)
        return {n: 0};
    const q = f => v[Math.min(v.length - 1, Math.floor(f * v.length))];
    const ms = x => Math.round(x / 10) / 100;
    return {n: v.length, min: ms(v[0]), median: ms(q(0.5)), p90: ms(q(0.9)), p99: ms(q(0.99)), max: ms(v.at(-1))};
}

class Daemon {
    constructor(argv) {
        this.argv = argv;
        this.proc = Gio.Subprocess.new(argv, Gio.SubprocessFlags.NONE);
        this.pid = Number(this.proc.get_identifier());
        this.started = now();
        note(`started ${argv.join(' ')} as pid ${this.pid}`);
    }

    kill() {
        this.killedAt = now();
        this.proc.force_exit();
    }

    signal(n) {
        this.proc.send_signal(n);
    }

    async waitExit() {
        const exited = new Promise(resolve => this.proc.wait_async(null, () => resolve(true)));
        await Promise.race([exited, sleep(2000)]);
    }
}

async function freshDaemon(argv = [STUB]) {
    const d = new Daemon(argv);
    const ok = await until(() => ext().events.some(e => e.t >= d.started && e.kind === 'hello'), 5000);
    d.helloAt = ext().events.find(e => e.t >= d.started && e.kind === 'hello')?.t;
    return {d, ok};
}

class Frames {
    constructor() {
        this.times = [];
        this._id = global.stage.connect('after-paint', () => this.times.push(now()));
    }

    stop() {
        global.stage.disconnect(this._id);
        const gaps = this.times.slice(1).map((t, i) => t - this.times[i]);
        const span = (this.times.at(-1) - this.times[0]) / 1e6;
        return {frames: this.times.length, fps: Math.round(this.times.length / span), interval: stats(gaps)};
    }
}

// Paint load: animated, overlapping, half-transparent St actors over the whole stage, moved on every frame by a
// Clutter.Timeline, and optionally busy JavaScript on every frame (a main loop that is slow, not only a GPU).
function startLoad({actors, busyMs}) {
    const group = new St.Widget({name: 's9-load', x: 0, y: 0, width: 1280, height: 720});
    Main.uiGroup.add_child(group);
    const list = [];
    for (let i = 0; i < actors; i++) {
        const w = new St.Widget({width: 120, height: 120,
            style: `background-color: rgba(${(i * 37) % 255}, ${(i * 91) % 255}, 200, 0.35); border-radius: 12px;`});
        group.add_child(w);
        list.push(w);
    }
    const timeline = new Clutter.Timeline({actor: group, duration: 1000, repeat_count: -1});
    timeline.connect('new-frame', (_tl, msecs) => {
        const t = (msecs / 1000) * Math.PI * 2;
        list.forEach((w, i) => {
            w.set_position(580 + 520 * Math.cos(t + i), 300 + 280 * Math.sin(1.3 * t + i));
            w.rotation_angle_z = (msecs * 0.36 + i * 7) % 360;
        });
        if (busyMs) {
            const end = now() + busyMs * 1000;
            while (now() < end)
                ;
        }
    });
    timeline.start();
    return () => {
        timeline.stop();
        group.destroy();
    };
}

let kbd = null;

export function init() {
    note('module loaded');
}

function key(code, down) {
    kbd.notify_key(now(), code, down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
}

async function armPrefix() {
    const e = ext();
    const before = e.presses.length;
    const tAlt = now();
    key(KEY_LEFTALT, true);
    key(KEY_A, true);
    await sleep(15);
    key(KEY_A, false);
    key(KEY_LEFTALT, false);
    const armed = await until(() => e.presses.length > before, 1000);
    return {armed, tAlt, p: e.presses[before]};
}

// One prefix command: Alt+A, then `\` 40 ms later, held 30 ms. Returns the press's timestamps.
async function prefixCommand() {
    const {armed, tAlt, p} = await armPrefix();
    if (!armed)
        return {armed: false};
    await sleep(40);
    const tCmd = now();
    key(KEY_BACKSLASH, true);
    await sleep(30);
    key(KEY_BACKSLASH, false);
    const done = await until(() => p.t_paint || p.missed || (p.t_pop && !p.t_reply), 1500, 'the effect');
    await until(() => p.t_pop, 3500, 'the pop');
    if (!done) {
        const rel = {};
        for (const [k, v] of Object.entries(p))
            rel[k] = k.startsWith('t_') ? Math.round((v - tCmd) / 100) / 10 : v;
        const fw = global.display.focus_window;
        note(`incomplete press (ms after the command key): ${JSON.stringify(rel)}; focus ${fw?.get_id()} ${fw?.title}; ` +
            `key focus ${global.stage.get_key_focus()?.name}; grab ${global.stage.get_grab_actor()?.name}`);
    }
    return {...p, armed: true, tAlt, tCmd};
}

function pressRow(r) {
    return {
        arm: r.t_modal - r.tAlt, // Alt+A press to the modal being up: local, no socket
        dispatch: r.t_key - r.tCmd, // `\` press to the grab actor's handler
        roundtrip: r.t_reply - r.t_key, // key sent to effects parsed back (socket, daemon, socket)
        applied: r.t_applied - r.tCmd, // press to move_resize_frame returned
        frame: r.t_frame - r.tCmd, // press to the window's frame changing (client acked the configure)
        effect: r.t_paint - r.tCmd, // press to the first stage paint after that: press-to-effect
        configure: r.t_frame - r.t_applied, // the Wayland configure/ack/commit part alone
        daemon: (r.daemon_us ?? NaN), // inside the stub
        parse: (r.parse_us ?? NaN),
    };
}

async function pressSeries(label, n) {
    const frames = new Frames();
    const rows = [];
    let missed = 0, notArmed = 0, wrong = 0;
    for (let i = 0; i < n; i++) {
        // eslint-disable-next-line no-await-in-loop
        const r = await prefixCommand();
        if (!r.armed) {
            notArmed++;
            continue;
        }
        if (r.missed || !r.t_paint) {
            missed++;
            continue;
        }
        if (r.reached && r.target && (r.reached[0] !== r.target[0] || r.reached[2] !== r.target[2]))
            wrong++;
        rows.push(pressRow(r));
        // eslint-disable-next-line no-await-in-loop
        await sleep(150);
    }
    const out = {label, n, notArmed, missed, wrongFrame: wrong, frames: frames.stop()};
    for (const k of ['arm', 'dispatch', 'roundtrip', 'applied', 'frame', 'effect', 'configure', 'daemon', 'parse'])
        out[k] = stats(rows.map(r => r[k]));
    result(`press.${label}`, out);
    return out;
}

async function pingSeries(label, n) {
    const rtts = [], daemon = [];
    for (let i = 0; i < n; i++) {
        // eslint-disable-next-line no-await-in-loop
        const r = await ext().ping();
        if (r) {
            rtts.push(r.rtt_us);
            daemon.push(r.dt_us);
        }
    }
    const out = {label, n, rtt: stats(rtts), daemon: stats(daemon)};
    result(`ping.${label}`, out);
    return out;
}

async function focusTarget(win) {
    win.activate(global.display.get_current_time_roundtrip());
    await until(() => global.display.focus_window === win, 2000, 'the target window to take focus');
}

// ---- the cases ----------------------------------------------------------------------------------------------------

async function caseBackoff() {
    // The extension started with no daemon: it has been failing to connect since enable().
    await until(() => ext().events.filter(e => e.kind === 'connect-failed').length >= 6, 8000, 'six failed connects');
    const fails = ext().events.filter(e => e.kind === 'connect-failed');
    const gaps = fails.slice(1).map((e, i) => Math.round((e.t - fails[i].t) / 1000));
    result('backoff.before-first-daemon', {attempts: fails.length, gaps_ms: gaps, errors: [...new Set(fails.map(f => f.error))],
        connect_us: stats(fails.map(f => f.connect_us))});
    const {d, ok} = await freshDaemon();
    check(ok, 'the extension connects to a daemon started while it backs off');
    result('backoff.first-connect', {spawn_to_hello_ms: Math.round((d.helloAt - d.started) / 1000)});
    return d;
}

function casePeer(d) {
    const peer = ext().peer;
    result('peer.ok', peer);
    check(peer.ok && peer.pid === d.pid, `SO_PEERCRED names the daemon itself (pid ${peer.pid} = ${d.pid})`);
    check(peer.exe === STUB, `/proc/<pid>/exe is ${peer.exe}`);
}

async function caseLatency(target) {
    await focusTarget(target);
    const idle = await pressSeries('idle', N_PRESSES);
    ext().localMode = true;
    await pressSeries('idle-local', N_PRESSES);
    ext().localMode = false;
    check(idle.notArmed === 0, 'Clutter virtual-keyboard Alt+A triggers the grab_accelerator binding');
    await pingSeries('idle', N_PINGS);

    // paint load: two GTK4 clients animating every frame (gtk4-demo's fishbowl, separate processes) plus 300
    // animated actors over the stage. (The perf helper's own redraws:true windows share the target's process, and
    // with them the target never committed a resize at all: see S9.md.)
    const launcher = new Gio.SubprocessLauncher({flags: Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE});
    launcher.setenv('WAYLAND_DISPLAY', 'gnome-shell-test-display', true);
    launcher.setenv('GDK_BACKEND', 'wayland', true);
    const nBefore = global.display.list_all_windows().length;
    const demos = [0, 1].map(() => launcher.spawnv(['gtk4-demo', '--run=fishbowl']));
    await until(() => global.display.list_all_windows().length >= nBefore + 2, 20000, 'two gtk4-demo windows');
    result('load.clients', {windows: global.display.list_all_windows().map(w => `${w.get_wm_class()} ${w.get_frame_rect().width}x${w.get_frame_rect().height}`)});
    await sleep(2000);
    let stop = startLoad({actors: 300, busyMs: 0});
    await focusTarget(target);
    await pressSeries('paint', N_PRESSES);
    ext().localMode = true;
    await pressSeries('paint-local', N_PRESSES);
    ext().localMode = false;
    await pingSeries('paint', N_PINGS);
    // the same with a 16 KB draw list in every effects reply
    ext().send({t: 'ctl', op: 'draw_bytes', n: 16384});
    await pressSeries('paint-draw16k', N_PRESSES);
    ext().send({t: 'ctl', op: 'draw_bytes', n: 0});
    // the same with the capped chunk reader instead of DataInputStream.read_line_async
    ext().readerMode = 'chunk';
    let t = now();
    ext().reconnectNow();
    await until(() => eventsSince(t, 'hello').length > 0, 3000, 'the reconnect');
    await focusTarget(target);
    await pressSeries('paint-chunk', N_PRESSES);
    ext().readerMode = 'line';
    t = now();
    ext().reconnectNow();
    await until(() => eventsSince(t, 'hello').length > 0, 3000, 'the reconnect');
    stop();

    // heavy: the same plus 8 ms of JavaScript on every frame
    stop = startLoad({actors: 300, busyMs: 8});
    await focusTarget(target);
    await pressSeries('heavy', N_PRESSES);
    ext().localMode = true;
    await pressSeries('heavy-local', N_PRESSES);
    ext().localMode = false;
    await pingSeries('heavy', N_PINGS);
    return () => {
        stop();
        for (const d of demos)
            d.force_exit();
    };
}

async function caseFraming() {
    const e = ext();
    // a reply written in two halves 50 ms apart is joined, not parsed in pieces
    e.send({t: 'ctl', op: 'split_next'});
    const r = await e.ping();
    result('framing.split', r);
    check(r !== null && r.rtt_us >= 50000, 'a line split across two writes arrives once, whole');

    // JSON parse cost in the shell, no socket
    const parse = {};
    for (const n of [2048, 65536, 1 << 20]) {
        const s = JSON.stringify({v: 1, t: 'effects', draw: 'x'.repeat(n)});
        const t0 = now();
        JSON.parse(s);
        parse[n] = now() - t0;
    }
    result('framing.json-parse-us', parse);

    // an 8 MB line through read_line_async: no cap, and how long the main loop stalls
    let frames = new Frames();
    let t = now();
    e.send({t: 'ctl', op: 'big_line', n: 8 << 20});
    await until(() => eventsSince(t, 'msg-blob').length > 0, 10000, 'the 8 MB blob');
    let blob = eventsSince(t, 'msg-blob')[0];
    result('framing.8mb-line-reader', {received: Boolean(blob), bytes: blob?.bytes, parse_us: blob?.parse_us,
        request_to_parsed_ms: blob ? Math.round((blob.t - t) / 1000) : null, frames: frames.stop()});

    // the same through the capped chunk reader: refused at 1 MiB
    e.readerMode = 'chunk';
    t = now();
    e.reconnectNow();
    await until(() => eventsSince(t, 'hello').length > 0, 3000, 'the reconnect');
    frames = new Frames();
    t = now();
    e.send({t: 'ctl', op: 'big_line', n: 8 << 20});
    await until(() => eventsSince(t, 'protocol-error').length > 0, 10000, 'the cap');
    const capped = eventsSince(t, 'protocol-error')[0];
    result('framing.8mb-chunk-reader', {event: capped, after_ms: capped ? Math.round((capped.t - t) / 1000) : null,
        frames: frames.stop()});
    check(Boolean(capped), 'the chunk reader closes the connection on a line over 1 MiB');
    await until(() => eventsSince(t, 'hello').length > 0, 3000, 'the reconnect');
    e.readerMode = 'line';
    t = now();
    e.reconnectNow();
    await until(() => eventsSince(t, 'hello').length > 0, 3000, 'the reconnect');

    // garbage: a non-JSON line is a protocol error; the extension drops the connection and reconnects
    t = now();
    e.send({t: 'ctl', op: 'garbage'});
    await until(() => eventsSince(t, 'hello').length > 0, 3000, 'the reconnect after garbage');
    result('framing.garbage', eventsSince(t).map(x => ({kind: x.kind, ms: Math.round((x.t - t) / 1000), detail: x.detail})));
}

async function caseEof(d) {
    const e = ext();
    const {armed} = await armPrefix();
    check(armed && Main.modalCount === 1, 'the prefix is armed (modal up) before the kill');
    const t = now();
    d.kill();
    await until(() => eventsSince(t, 'eof').length > 0, 2000, 'EOF');
    const eof = eventsSince(t, 'eof')[0];
    const press = e.presses.at(-1);
    result('eof.sigkill', {kill_to_eof_us: eof ? eof.t - t : null, kill_to_pop_us: press.t_pop ? press.t_pop - t : null,
        pop_reason: press.pop_reason, modalCount: Main.modalCount, prefix_grabbed: Boolean(e._action)});
    check(Main.modalCount === 0 && !e._action, 'EOF pops the modal and ungrabs Alt+A');
    await d.waitExit();
    // backoff while the daemon is down; the socket file is still there, so connect is refused, not ENOENT
    await sleep(4500);
    const fails = eventsSince(t, 'connect-failed');
    result('eof.backoff', {attempts: fails.length, at_ms: fails.map(f => Math.round((f.t - t) / 1000)),
        errors: [...new Set(fails.map(f => f.error))]});
    const {d: d2, ok} = await freshDaemon();
    check(ok, 'the extension reconnects to a restarted daemon');
    result('eof.reconnect', {spawn_to_hello_ms: Math.round((d2.helloAt - d2.started) / 1000)});
    return d2;
}

async function caseHang(d, target) {
    const e = ext();
    await focusTarget(target);
    d.signal(SIGSTOP);
    await sleep(100);
    const r = await prefixCommand();
    result('hang.missed-reply', {missed: r.missed, key_to_pop_ms: r.t_pop ? Math.round((r.t_pop - r.t_key) / 1000) : null,
        pop_reason: r.pop_reason, modalCount: Main.modalCount});
    check(r.missed && Main.modalCount === 0, 'a stopped daemon costs a 200 ms missed reply, then the modal pops');

    // what a synchronous write would do to the compositor: fill the socket buffer of a stopped daemon
    const conn = e._conn;
    const sock = conn.get_socket();
    sock.set_timeout(1);
    const frames = new Frames();
    const t0 = now();
    let written = 0, error = null;
    try {
        const chunk = new GLib.Bytes(new Uint8Array(65536).fill(32));
        for (let i = 0; i < 64; i++)
            written += conn.get_output_stream().write_bytes(chunk, null);
    } catch (err) {
        error = err.message;
    }
    const blocked = now() - t0;
    await sleep(100);
    result('hang.sync-write', {written, blocked_ms: Math.round(blocked / 1000), error, frames: frames.stop()});
    sock.set_timeout(0);
    e.reconnectNow(); // that connection's stream now holds a cut line; start clean
    d.signal(SIGCONT);
    await until(() => e._conn && e.events.at(-1).kind === 'hello', 3000, 'the reconnect');

    // async writes to a stopped daemon: the main loop keeps painting, and the queue cap drops the connection
    d.signal(SIGSTOP);
    await sleep(100);
    const frames2 = new Frames();
    const t1 = now();
    let sent = 0;
    while (e.send({t: 'delta', pad: 'z'.repeat(4000)}))
        sent++;
    const loopUs = now() - t1;
    await sleep(500);
    result('hang.async-flood', {messages_queued: sent, enqueue_loop_ms: Math.round(loopUs / 1000),
        cap_event: eventsSince(t1, 'queue-full')[0] ?? null, frames: frames2.stop()});
    d.signal(SIGCONT);
    d.kill();
    await d.waitExit();
    return (await freshDaemon()).d;
}

async function caseVersion(d) {
    d.kill();
    await d.waitExit();
    const t = now();
    const bad = new Daemon([STUB, '--proto', '2']);
    await sleep(2500);
    const refused = eventsSince(t, 'version-refused');
    result('version.mismatch', {refusals: refused.length, first: refused[0] ?? null,
        hellos: eventsSince(t, 'hello').length, prefix_grabbed: Boolean(ext()._action)});
    check(refused.length > 0 && eventsSince(t, 'hello').length === 0, 'a v2 daemon is refused, never connected');
    bad.kill();
    await bad.waitExit();
}

async function caseActivation() {
    const t = now();
    const emu = new Daemon(['python3', '-I', '/src/spikes/S9/l1/activate.py', SOCKET, STUB]);
    await until(() => eventsSince(t, 'peer-refused').length > 0, 5000, 'a refused peer');
    const r = eventsSince(t, 'peer-refused')[0];
    result('activation.peer', {...r, emulator_pid: emu.pid});
    check(r && r.pid === emu.pid && r.exe !== STUB,
        'under socket activation SO_PEERCRED names the process that listened, not the daemon');
    emu.signal(SIGTERM);
    await emu.waitExit();
}

function copyBinary(dst, mode) {
    const p = Gio.Subprocess.new(['install', '-D', '-m', mode, STUB, dst], Gio.SubprocessFlags.NONE);
    p.wait(null);
    return p.get_exit_status();
}

async function caseUpgrade() {
    const path = `${RUNTIME}/s9bin/vase-gnome`;
    copyBinary(path, '0755');
    ext().expectedExe = path;
    let t = now();
    const d = new Daemon([path]);
    await until(() => eventsSince(t, 'hello').length > 0, 5000, 'a connect to the copy');
    const before = ext().peer;
    // an upgrade the way install(1) and most package tools do it: a new file renamed over the old path
    copyBinary(`${path}.new`, '0755');
    Gio.File.new_for_path(`${path}.new`).move(Gio.File.new_for_path(path), Gio.FileCopyFlags.OVERWRITE, null, null);
    t = now();
    ext().reconnectNow();
    await until(() => eventsSince(t, 'peer-refused').length > 0, 3000, 'a refused peer after the upgrade');
    const after = eventsSince(t, 'peer-refused')[0];
    result('upgrade.exe', {before_exe: before?.exe, after_exe: after?.exe, refused: Boolean(after)});
    check(after?.exe === `${path} (deleted)`, 'a daemon whose binary was replaced reads as "<path> (deleted)"');
    d.kill();
    await d.waitExit();
}

async function caseUnreadable() {
    const path = `${RUNTIME}/s9bin/vase-gnome-x`;
    copyBinary(path, '0111');
    ext().expectedExe = path;
    const t = now();
    const d = new Daemon([path]);
    await until(() => eventsSince(t, 'peer-refused').length > 0 || eventsSince(t, 'hello').length > 0, 5000, 'a peer check');
    const r = eventsSince(t).find(x => x.kind === 'peer-refused' || x.kind === 'peer-ok');
    result('unreadable.exe', r);
    check(r?.kind === 'peer-refused' && r.exeError, 'an execute-only binary makes the daemon non-dumpable, so /proc/<pid>/exe is denied');
    d.kill();
    await d.waitExit();
    ext().expectedExe = STUB;
}

async function caseCycle() {
    const {d} = await freshDaemon();
    const checker = new LeakChecker();
    checker.install();
    try {
        const baseline = snapshot();
        checker.watchExtension(ext());
        let t = now();
        Main.extensionManager.disableExtension(UUID);
        await until(() => Main.extensionManager.lookup(UUID).state === ExtensionState.INACTIVE, 3000, 'INACTIVE');
        await sleep(300);
        const leaks = checker.leaks({baseline, stateObj: ext()});
        result('cycle.disable', {leaks, modalCount: Main.modalCount});
        check(leaks.length === 0, 'disable leaves no actor, signal, source or grab');
        t = now();
        Main.extensionManager.enableExtension(UUID);
        await until(() => ext().events?.some(e => e.kind === 'hello'), 3000, 'a hello after enable');
        const hello = ext().events.find(e => e.kind === 'hello');
        result('cycle.enable', {enable_to_hello_ms: hello ? Math.round((hello.t - t) / 1000) : null});
        check(Boolean(hello), 'enable reconnects to the running daemon');
    } finally {
        checker.uninstall();
    }
    d.kill();
    await d.waitExit();
}

export async function run() {
    const wanted = (GLib.getenv('S9_CASES') || '').split(',').filter(Boolean);
    const want = c => wanted.length === 0 || wanted.includes(c);
    note('run() started');
    kbd = Clutter.get_default_backend().get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    await until(() => Main.extensionManager.lookup(UUID)?.state === ExtensionState.ACTIVE, 5000, 'ACTIVE');
    if (Main.overview.visible) {
        Main.overview.hide();
        await sleep(500);
    }
    result('env', {load: new TextDecoder().decode(GLib.file_get_contents('/proc/loadavg')[1]).trim(), shell: global.session_mode,
        cpus: GLib.get_num_processors()});

    let d = await caseBackoff();
    casePeer(d);

    Scripting.createTestWindow({width: 640, height: 480});
    await Scripting.waitTestWindows();
    const target = global.display.list_all_windows().find(w => w.get_wm_class() === 'org.gnome.Shell.PerfHelper');
    result('target', {id: target.get_id(), wm_class: target.get_wm_class(), client: target.get_client_type()});
    let stopLoad = null;
    if (want('latency'))
        stopLoad = await caseLatency(target);
    if (want('framing'))
        await caseFraming();
    if (want('eof'))
        d = await caseEof(d);
    if (want('hang'))
        d = await caseHang(d, target);
    stopLoad?.();
    if (want('version'))
        await caseVersion(d);
    else
        d.kill();
    if (want('activation'))
        await caseActivation();
    if (want('upgrade'))
        await caseUpgrade();
    if (want('unreadable'))
        await caseUnreadable();
    if (want('cycle'))
        await caseCycle();

    note(failures.length ? `FAIL: ${failures.join('; ')}` : 'PASS');
}
