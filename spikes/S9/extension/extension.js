// Spike S9 (docs/linux-spike/S9.md): the extension half of the proposed transport (wayfinder §3.5), throwaway.
//
// - connects to $XDG_RUNTIME_DIR/vase/gnome.sock with Gio.SocketClient, reconnecting with backoff (100 ms doubling to
//   2 s) and never spawning the daemon;
// - checks the peer: Gio.Socket.get_credentials() (SO_PEERCRED) for the uid and pid, then /proc/<pid>/exe;
// - speaks versioned JSON lines: hello both ways, then keys, effects, pings; a version mismatch is refused;
// - reads lines one of two ways: Gio.DataInputStream.read_line_async ("line", no length cap) or read_bytes_async with
//   its own split and a 1 MiB cap ("chunk");
// - writes only asynchronously, through one queue with a byte cap;
// - Alt+A (grab_accelerator, only while connected) pushes a NONE modal; the next key goes to the daemon; the effects
//   reply is applied (move_resize_frame on the focused window) and the modal pops on reply plus key release, Esc,
//   3 s, a 200 ms missed reply, or EOF;
// - timestamps every step of a press so the L1 case (spikes/S9/l1/s9.js) can read press-to-effect.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const LOG = '[vase-s9]';
const PROTO = 1;
const BACKOFF_MIN_MS = 100;
const BACKOFF_MAX_MS = 2000;
const REPLY_TIMEOUT_MS = 200;
const PREFIX_TIMEOUT_MS = 3000;
const MAX_LINE = 1 << 20;
const MAX_QUEUED = 1 << 20;
const EVDEV_OFFSET = 8; // X/xkb keycode = evdev keycode + 8
const KEY_ESC = 1;

const now = () => GLib.get_monotonic_time();
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export default class VaseS9Extension extends Extension {
    enable() {
        console.log(`${LOG} enable`);
        try {
            this.expectedExe = '/opt/s9/release/s9-stub';
            this.readerMode = 'line';
            this.socketPath = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'vase', 'gnome.sock']);
            this.events = [];
            this.presses = [];
            this._pings = new Map();
            this._seq = 0;
            this._backoff = BACKOFF_MIN_MS;
            this._state = 'idle';
            this._grabActor = new St.Widget({name: 'vase-s9-grab', reactive: true, can_focus: true, width: 0, height: 0});
            Main.uiGroup.add_child(this._grabActor);
            this._keyPressId = this._grabActor.connect('key-press-event', (_a, ev) => this._onGrabKey(ev, true));
            this._keyReleaseId = this._grabActor.connect('key-release-event', (_a, ev) => this._onGrabKey(ev, false));
            this._accelId = global.display.connect('accelerator-activated', (_d, action) => {
                if (action === this._action)
                    this._onPrefix();
            });
            this._connect();
        } catch (e) {
            this._teardown();
            throw e;
        }
    }

    disable() {
        console.log(`${LOG} disable`);
        this._teardown();
    }

    _event(kind, extra = {}) {
        const e = {t: now(), kind, ...extra};
        this.events.push(e);
        console.log(`${LOG} ${kind} ${JSON.stringify(extra)}`);
        return e;
    }

    // ---- connection ------------------------------------------------------------------------------------------

    _connect() {
        this._retryId = 0;
        this._cancellable = new Gio.Cancellable();
        const cancellable = this._cancellable;
        const t0 = now();
        const client = new Gio.SocketClient();
        client.connect_async(Gio.UnixSocketAddress.new(this.socketPath), cancellable, (c, res) => {
            let conn;
            try {
                conn = c.connect_finish(res);
            } catch (e) {
                if (e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    return;
                this._event('connect-failed', {error: e.message, connect_us: now() - t0, next_ms: this._backoff});
                this._scheduleRetry();
                return;
            }
            if (cancellable.is_cancelled()) {
                conn.close(null);
                return;
            }
            this._onConnected(conn, now() - t0);
        });
    }

    _scheduleRetry() {
        if (this._retryId)
            GLib.source_remove(this._retryId);
        const delay = this._backoff;
        this._backoff = Math.min(this._backoff * 2, BACKOFF_MAX_MS);
        this._retryId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
            this._retryId = 0;
            this._connect();
            return GLib.SOURCE_REMOVE;
        });
    }

    _peerCheck(conn) {
        const t0 = now();
        const creds = conn.get_socket().get_credentials();
        const pid = creds.get_unix_pid();
        const uid = creds.get_unix_user();
        const myUid = new Gio.Credentials().get_unix_user();
        let exe = null, exeError = null;
        try {
            exe = GLib.file_read_link(`/proc/${pid}/exe`);
        } catch (e) {
            exeError = e.message;
        }
        const ok = uid === myUid && exe === this.expectedExe;
        return {ok, pid, uid, myUid, exe, exeError, expected: this.expectedExe, check_us: now() - t0};
    }

    _onConnected(conn, connectUs) {
        const peer = this._peerCheck(conn);
        this._event(peer.ok ? 'peer-ok' : 'peer-refused', {...peer, connect_us: connectUs});
        if (!peer.ok) {
            conn.close(null);
            this._scheduleRetry();
            return;
        }
        this._conn = conn;
        this.peer = peer;
        this._out = conn.get_output_stream();
        this._queue = [];
        this._queued = 0;
        this._writing = false;
        this._pending = new Uint8Array(0);
        this._connectedAt = now();
        const monitor = Main.layoutManager.primaryIndex;
        const wa = Main.layoutManager.getWorkAreaForMonitor(monitor);
        this.send({t: 'hello', role: 'extension', instance: `shell-${new Gio.Credentials().get_unix_pid()}`,
            work_area: [wa.x, wa.y, wa.width, wa.height]});
        if (this.readerMode === 'line') {
            this._in = new Gio.DataInputStream({base_stream: conn.get_input_stream(), close_base_stream: false});
            this._readLine();
        } else {
            this._in = conn.get_input_stream();
            this._readChunk();
        }
    }

    _readLine() {
        const cancellable = this._cancellable;
        this._in.read_line_async(GLib.PRIORITY_DEFAULT, cancellable, (stream, res) => {
            let line;
            try {
                [line] = stream.read_line_finish(res);
            } catch (e) {
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._disconnect('read-error', e.message);
                return;
            }
            if (line === null) {
                this._disconnect('eof');
                return;
            }
            const tRecv = now();
            if (this._onLine(decoder.decode(line), tRecv))
                this._readLine();
        });
    }

    _readChunk() {
        const cancellable = this._cancellable;
        this._in.read_bytes_async(65536, GLib.PRIORITY_DEFAULT, cancellable, (stream, res) => {
            let bytes;
            try {
                bytes = stream.read_bytes_finish(res);
            } catch (e) {
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._disconnect('read-error', e.message);
                return;
            }
            const chunk = bytes.toArray();
            if (chunk.length === 0) {
                this._disconnect('eof');
                return;
            }
            const tRecv = now();
            const buf = new Uint8Array(this._pending.length + chunk.length);
            buf.set(this._pending);
            buf.set(chunk, this._pending.length);
            let start = 0;
            for (let nl = buf.indexOf(10, start); nl !== -1; nl = buf.indexOf(10, start)) {
                if (!this._onLine(decoder.decode(buf.subarray(start, nl)), tRecv))
                    return;
                start = nl + 1;
            }
            this._pending = buf.slice(start);
            if (this._pending.length > MAX_LINE) {
                this._disconnect('protocol-error', `line over ${MAX_LINE} bytes`);
                return;
            }
            this._readChunk();
        });
    }

    send(msg) {
        if (!this._conn)
            return false;
        const bytes = encoder.encode(`${JSON.stringify({v: PROTO, ...msg})}\n`);
        if (this._queued + bytes.length > MAX_QUEUED) {
            this._disconnect('queue-full', `${this._queued} bytes waiting`);
            return false;
        }
        this._queue.push(bytes);
        this._queued += bytes.length;
        this._flush();
        return true;
    }

    _flush() {
        if (this._writing || this._queue.length === 0 || !this._conn)
            return;
        this._writing = true;
        const head = this._queue[0];
        const cancellable = this._cancellable;
        this._out.write_bytes_async(new GLib.Bytes(head), GLib.PRIORITY_DEFAULT, cancellable, (stream, res) => {
            let n;
            try {
                n = stream.write_bytes_finish(res);
            } catch (e) {
                this._writing = false;
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._disconnect('write-error', e.message);
                return;
            }
            this._writing = false;
            this._queued -= n;
            if (n < head.length)
                this._queue[0] = head.subarray(n);
            else
                this._queue.shift();
            this.lastWriteDone = now();
            this._flush();
        });
    }

    _onLine(text, tRecv) {
        let msg;
        const t0 = now();
        try {
            msg = JSON.parse(text);
        } catch (e) {
            this._disconnect('protocol-error', `not JSON: ${text.slice(0, 40)}`);
            return false;
        }
        const parseUs = now() - t0;
        if (msg?.v !== PROTO) {
            this._disconnect('version-refused', `daemon speaks v${msg?.v}, extension v${PROTO}`);
            return false;
        }
        switch (msg.t) {
        case 'hello':
            this._backoff = BACKOFF_MIN_MS;
            this._event('hello', {build: msg.build, pid: msg.pid, since_connect_us: now() - this._connectedAt});
            this._grabPrefix();
            break;
        case 'pong': {
            const p = this._pings.get(msg.seq);
            this._pings.delete(msg.seq);
            p?.({rtt_us: tRecv - p.sent, dt_us: msg.dt_us, parse_us: parseUs});
            break;
        }
        case 'effects':
            this._onEffects(msg, tRecv, parseUs, text.length);
            break;
        default:
            this._event(`msg-${msg.t}`, {bytes: text.length, parse_us: parseUs, recv_to_parsed_us: now() - tRecv});
        }
        return true;
    }

    ping() {
        return new Promise(resolve => {
            const seq = ++this._seq;
            resolve.sent = now();
            this._pings.set(seq, resolve);
            if (!this.send({t: 'ping', seq}))
                resolve(null);
        });
    }

    reconnectNow() {
        this._disconnect('reconnect-requested');
    }

    _disconnect(kind, detail = '') {
        if (!this._conn && !this._retryId && !this._cancellable)
            return;
        this._event(kind, {detail, queued: this._queued ?? 0});
        this._releaseKeys(`disconnect (${kind})`);
        this._closeConnection();
        this._scheduleRetry();
    }

    _closeConnection() {
        this._cancellable?.cancel();
        this._cancellable = null;
        if (this._conn) {
            try {
                this._conn.close(null);
            } catch (e) {
                console.error(`${LOG} close: ${e.message}`);
            }
        }
        this._conn = null;
        this._in = null;
        this._out = null;
        this._queue = [];
        this._queued = 0;
        this._writing = false;
        for (const p of this._pings?.values() ?? [])
            p(null);
        this._pings?.clear();
    }

    // ---- keys --------------------------------------------------------------------------------------------------

    _grabPrefix() {
        if (this._action)
            return;
        this._action = global.display.grab_accelerator('<Alt>a', Meta.KeyBindingFlags.NONE);
        if (this._action === Meta.KeyBindingAction.NONE) {
            this._action = 0;
            this._event('grab-failed');
            return;
        }
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._action), Shell.ActionMode.NORMAL);
    }

    _ungrabPrefix() {
        if (!this._action)
            return;
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._action), Shell.ActionMode.NONE);
        global.display.ungrab_accelerator(this._action);
        this._action = 0;
    }

    _onPrefix() {
        const press = {t_accel: now()};
        if (this._state !== 'idle' || !this._conn)
            return;
        this._grab = Main.pushModal(this._grabActor, {actionMode: Shell.ActionMode.NONE});
        press.t_modal = now();
        this._press = press;
        this.presses.push(press);
        this._state = 'prefix';
        this._prefixTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PREFIX_TIMEOUT_MS, () => {
            this._prefixTimeoutId = 0;
            this._releaseKeys('3 s prefix timeout');
            return GLib.SOURCE_REMOVE;
        });
    }

    _onGrabKey(event, pressed) {
        const p = this._press;
        if (!p)
            return true;
        const evdev = event.get_key_code() - EVDEV_OFFSET;
        if (!pressed) {
            if (evdev === p.keycode) {
                p.released = true;
                if (p.t_reply)
                    this._releaseKeys('reply and release');
            }
            return true;
        }
        if (evdev === KEY_ESC) {
            this._releaseKeys('Esc');
            return true;
        }
        if (p.keycode !== undefined)
            return true; // the spike times one command key per prefix; later keys are swallowed
        p.t_key = now();
        p.keycode = evdev;
        p.event_time_ms = event.get_time();
        p.seq = ++this._seq;
        const win = global.display.focus_window;
        const r = win?.get_frame_rect();
        if (this.localMode) {
            // Baseline without the socket: the same effect the stub would send, computed in the key handler.
            const wa = Main.layoutManager.getWorkAreaForMonitor(Main.layoutManager.primaryIndex);
            const half = Math.floor(wa.width / 2);
            this._right = !(this._right ?? false);
            const rect = this._right ? [wa.x + half, wa.y, half, wa.height] : [wa.x, wa.y, Math.floor(wa.width * 0.6), wa.height];
            p.t_queued = now();
            this._onEffects({seq: p.seq, key_mode: 'idle', dt_us: 0,
                effects: win ? [{op: 'place', id: win.get_id(), rect}, {op: 'raise', id: win.get_id()}] : []}, now(), 0, 0);
            return true;
        }
        this.send({t: 'key', seq: p.seq, keycode: evdev, mods: event.get_state(),
            focus: win ? {id: win.get_id(), rect: [r.x, r.y, r.width, r.height]} : null});
        p.t_queued = now();
        this._replyTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, REPLY_TIMEOUT_MS, () => {
            this._replyTimeoutId = 0;
            p.missed = true;
            this._event('missed-reply', {seq: p.seq});
            this._releaseKeys('200 ms missed reply');
            return GLib.SOURCE_REMOVE;
        });
        return true;
    }

    _onEffects(msg, tRecv, parseUs, bytes) {
        const p = this._press;
        if (!p || p.seq !== msg.seq) {
            this._event('late-effects', {seq: msg.seq});
            return;
        }
        if (this._replyTimeoutId) {
            GLib.source_remove(this._replyTimeoutId);
            this._replyTimeoutId = 0;
        }
        p.t_reply = tRecv;
        p.parse_us = parseUs;
        p.reply_bytes = bytes;
        p.daemon_us = msg.dt_us;
        for (const e of msg.effects) {
            // Look the window up again for every effect (wayfinder §3.2); unknown ids are ignored.
            const win = global.display.list_all_windows().find(w => w.get_id() === e.id);
            if (!win) {
                this._event('unknown-window', {id: e.id});
                continue;
            }
            if (e.op === 'place') {
                const [x, y, w, h] = e.rect;
                // The first frame change of either kind, then the next stage paint: the effect on screen.
                const ids = [];
                const onChange = kind => {
                    p[`t_${kind}`] ??= now();
                    if (p.t_frame)
                        return;
                    p.t_frame = now();
                    const paintId = global.stage.connect('after-paint', () => {
                        global.stage.disconnect(paintId);
                        p.t_paint = now();
                        const fr = win.get_frame_rect();
                        p.reached = [fr.x, fr.y, fr.width, fr.height];
                        for (const id of ids)
                            win.disconnect(id);
                    });
                };
                ids.push(win.connect('size-changed', () => onChange('size')));
                ids.push(win.connect('position-changed', () => onChange('position')));
                p.target = [x, y, w, h];
                win.move_resize_frame(true, x, y, w, h);
                p.t_applied = now();
            } else if (e.op === 'raise') {
                win.raise();
            }
        }
        if (msg.key_mode === 'idle' && p.released)
            this._releaseKeys('reply and release');
    }

    _releaseKeys(why) {
        if (this._replyTimeoutId) {
            GLib.source_remove(this._replyTimeoutId);
            this._replyTimeoutId = 0;
        }
        if (this._prefixTimeoutId) {
            GLib.source_remove(this._prefixTimeoutId);
            this._prefixTimeoutId = 0;
        }
        if (this._grab) {
            Main.popModal(this._grab);
            this._grab = null;
            if (this._press) {
                this._press.t_pop = now();
                this._press.pop_reason = why;
            }
        }
        this._press = null;
        this._state = 'idle';
        if (why.startsWith('disconnect'))
            this._ungrabPrefix();
    }

    // ---- teardown ----------------------------------------------------------------------------------------------

    _teardown() {
        const steps = [
            () => this._releaseKeys('disable'),
            () => this._ungrabPrefix(),
            () => {
                if (this._accelId)
                    global.display.disconnect(this._accelId);
                this._accelId = 0;
            },
            () => {
                if (this._retryId)
                    GLib.source_remove(this._retryId);
                this._retryId = 0;
            },
            () => this._closeConnection(),
            () => {
                this._grabActor?.destroy();
                this._grabActor = null;
            },
        ];
        for (const step of steps) {
            try {
                step();
            } catch (e) {
                console.error(`${LOG} teardown step: ${e}`);
            }
        }
    }
}
