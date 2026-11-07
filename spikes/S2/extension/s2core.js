// Spike S2 (throwaway): the input contract of docs/linux-tiler/wayfinder.md §3.3, driven by a Clutter virtual
// keyboard inside the shell. Shared by the L3 extension (extension.js) and the L1 automation script (../l1/s2.js).
//
// The prefix under test is the map's candidate: Alt+A through grab_accelerator, Main.pushModal synchronously in the
// accelerator-activated handler, the next key taken by the grab actor, popModal on that key's release (or on its
// press, to compare), a 3 s timeout, Esc, the kill chord checked first, and notify::revoked logged.
//
// Every record goes into this.records with a CLOCK_MONOTONIC time in microseconds (GLib.get_monotonic_time, the
// clock Clutter stamps events with), so shell and client logs line up.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Keyboard from 'resource:///org/gnome/shell/ui/status/keyboard.js';

// evdev codes (linux/input-event-codes.h)
export const K = {
    ESC: 1, MINUS: 12, TAB: 15, Q: 16, E: 18, Y: 21, A: 30, L: 38, BACKSLASH: 43, Z: 44, X: 45,
    LEFTCTRL: 29, LEFTSHIFT: 42, LEFTALT: 56, LEFTMETA: 125, LEFT: 105,
};
const M = Clutter.ModifierType;
const IM_FLAG = Clutter.EventFlags.FLAG_INPUT_METHOD;
const now = () => GLib.get_monotonic_time();
const isModifierSym = sym => (sym >= 0xffe1 && sym <= 0xffee) || sym === 0xfe03 || sym === 0xfe11;

export class S2 {
    constructor({dir, outDir, level}) {
        this.dir = dir;
        this.outDir = outDir;
        this.level = level;
        this.records = [];
        this.prefix = null;
        this.popOn = 'release';
        this.nextMode = Shell.ActionMode.NONE;
        this.prefixTimeoutMs = 3000;
        this.scen = null;
        this.target = null;
        GLib.mkdir_with_parents(outDir, 0o700);
    }

    rec(obj) {
        obj.t = now();
        if (this.scen)
            obj.scenario = this.scen;
        if (this.target)
            obj.target = this.target;
        this.records.push(obj);
        if (obj.ev === 'scenario-start' || obj.ev === 'note' || obj.ev === 'error')
            console.log(`[s2] ${JSON.stringify(obj)}`);
    }

    flush() {
        const text = this.records.map(r => JSON.stringify(r)).join('\n') + '\n';
        GLib.file_set_contents(`${this.outDir}/shell.jsonl`, text);
    }

    sleep(ms) {
        return new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, Math.max(0, Math.round(ms)), () => {
            resolve();
            return GLib.SOURCE_REMOVE;
        }));
    }

    // Spins with the main loop blocked: events injected meanwhile queue up unprocessed.
    spin(ms) {
        const end = now() + ms * 1000;
        while (now() < end)
            ;
    }

    // ------------------------------------------------------------------------------------------------ setup

    setup() {
        const seat = Clutter.get_default_backend().get_default_seat();
        this.vk = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);

        this.accelId = global.display.connect('accelerator-activated', this._onAccel.bind(this));
        this.deaccelId = global.display.connect('accelerator-deactivated', (_d, action) =>
            this.rec({ev: 'accelerator-deactivated', action}));
        this.altA = this._grab('<Alt>a', Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW);
        // gsd-media-keys grabs Super+L (screensaver) through org.gnome.Shell.GrabAccelerators, which does exactly
        // this (shellDBus.js _grabAcceleratorForSender); the nested shell has no gsd, so the spike stands in for it.
        this.superL = this._grab('<Super>l', Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW);

        const src = Gio.SettingsSchemaSource.new_from_directory(`${this.dir}/schemas`,
            Gio.SettingsSchemaSource.get_default(), false);
        const settings = new Gio.Settings({settings_schema: src.lookup('org.gnome.shell.extensions.vase-s2', true)});
        this.killAction = Main.wm.addKeybinding('s2-kill-chord', settings, Meta.KeyBindingFlags.NON_MASKABLE,
            Shell.ActionMode.ALL, () => {
                this.rec({ev: 'binding', name: 'kill-chord', armed: !!this.prefix});
                if (this.prefix)
                    this._pop(this.prefix, 'kill-binding');
            });
        this.rec({ev: 'note', what: 'setup', altA: this.altA, superL: this.superL, killAction: this.killAction});
    }

    _grab(accel, modes, flags = Meta.KeyBindingFlags.NONE) {
        const action = global.display.grab_accelerator(accel, flags);
        if (action === Meta.KeyBindingAction.NONE) {
            this.rec({ev: 'error', what: `grab_accelerator ${accel} failed`});
            return 0;
        }
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), modes);
        return action;
    }

    teardown() {
        if (this.prefix)
            this._pop(this.prefix, 'teardown');
        for (const a of [this.altA, this.superL, this.shiftLeft]) {
            if (a)
                global.display.ungrab_accelerator(a);
        }
        if (this.accelId)
            global.display.disconnect(this.accelId);
        if (this.deaccelId)
            global.display.disconnect(this.deaccelId);
        if (this.killAction)
            Main.wm.removeKeybinding('s2-kill-chord');
        this.vk = null;
    }

    // ------------------------------------------------------------------------------------------------ the prefix

    _onAccel(display, action, _device, _timestamp) {
        const tAccel = now();
        const ev = Clutter.get_current_event();
        const info = ev ? {evTime: ev.get_time_us(), im: !!(ev.get_flags() & IM_FLAG), state: ev.get_state()} : {};
        if (action === this.superL || action === this.shiftLeft) {
            this.rec({ev: 'binding', name: action === this.superL ? 'super-l' : 'shift-left', ...info,
                armed: !!this.prefix});
            return;
        }
        if (action !== this.altA)
            return;
        if (this.prefix) {
            this.rec({ev: 'alt-a-while-armed', ...info});
            return;
        }
        this._arm(this.nextMode, info, tAccel);
    }

    _arm(mode, info, tAccel) {
        const catcher = new St.Widget({reactive: true, can_focus: true, x: 0, y: 0, width: 1, height: 1});
        Main.uiGroup.add_child(catcher);
        const grab = Main.pushModal(catcher, {actionMode: mode});
        const tArmed = now();
        const [, , mods] = global.get_pointer();
        const p = {catcher, grab, mode, tArmed, command: null};
        this.prefix = p;
        p.pressId = catcher.connect('key-press-event', (_a, e) => this._key(p, e, true));
        p.releaseId = catcher.connect('key-release-event', (_a, e) => this._key(p, e, false));
        p.revokedId = grab.connect('notify::revoked', () =>
            this.rec({ev: 'revoked', value: grab.is_revoked(), topActor: global.stage.get_grab_actor()?.toString()}));
        p.timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, this.prefixTimeoutMs, () => {
            p.timeoutId = 0;
            this._pop(p, 'timeout');
            return GLib.SOURCE_REMOVE;
        });
        this.rec({
            ev: 'armed', mode, ...info, tAccel, tArmed,
            accelLagUs: info.evTime ? tAccel - info.evTime : null,
            armLagUs: info.evTime ? tArmed - info.evTime : null,
            pushModalUs: tArmed - tAccel,
            modsAtArm: mods, altHeldAtArm: !!(mods & M.MOD1_MASK),
            keyFocusIsCatcher: global.stage.get_key_focus() === catcher,
            grabActorIsCatcher: global.stage.get_grab_actor() === catcher,
            actionMode: Main.actionMode,
        });
    }

    _key(p, e, press) {
        const t = now();
        const sym = e.get_key_symbol();
        const code = e.get_key_code();
        const state = e.get_state();
        const flags = e.get_flags();
        const r = {
            ev: press ? 'grab-press' : 'grab-release', sym, code, uni: e.get_key_unicode(), state,
            im: !!(flags & IM_FLAG), repeated: !!(flags & Clutter.EventFlags.FLAG_REPEATED),
            evTime: e.get_time_us(), lagUs: t - e.get_time_us(), revoked: p.grab.is_revoked(),
        };
        this.rec(r);
        if (!press) {
            if (p.command && p.command.code === code && this.popOn === 'release')
                this._pop(p, 'command-release');
            return Clutter.EVENT_STOP;
        }
        // The kill chord first: inside a NONE grab no binding fires, NON_MASKABLE ones included (F14).
        if (sym === Clutter.KEY_Escape && (state & M.CONTROL_MASK) && (state & M.MOD1_MASK) &&
            (state & (M.SUPER_MASK | M.MOD4_MASK))) {
            this._pop(p, 'kill-chord-in-grab');
            return Clutter.EVENT_STOP;
        }
        if (isModifierSym(sym))
            return Clutter.EVENT_STOP;
        if (sym === Clutter.KEY_Escape) {
            this._pop(p, 'escape');
            return Clutter.EVENT_STOP;
        }
        if (p.command) {
            this.rec({ev: 'key-after-command', sym, code});
            return Clutter.EVENT_STOP;
        }
        p.command = {code, sym, t, evTime: r.evTime};
        this.rec({ev: 'command', sym, code, uni: r.uni, state, im: r.im, lagUs: r.lagUs,
            sinceArmUs: t - p.tArmed});
        if (this.popOn === 'press')
            this._pop(p, 'command-press');
        return Clutter.EVENT_STOP;
    }

    _pop(p, why) {
        if (this.prefix !== p)
            return;
        this.prefix = null;
        if (p.timeoutId)
            GLib.source_remove(p.timeoutId);
        p.catcher.disconnect(p.pressId);
        p.catcher.disconnect(p.releaseId);
        p.grab.disconnect(p.revokedId);
        let error = null;
        try {
            Main.popModal(p.grab);
        } catch (e) {
            error = `${e}`;
        }
        p.catcher.destroy();
        this.rec({ev: 'popped', why, heldMs: (now() - p.tArmed) / 1000, error, actionMode: Main.actionMode,
            grabActor: global.stage.get_grab_actor()?.toString() ?? null});
    }

    async waitIdle(ms = 4000) {
        const end = now() + ms * 1000;
        while (this.prefix && now() < end)
            await this.sleep(10);
        if (this.prefix)
            this.rec({ev: 'error', what: 'prefix still armed'});
    }

    // ------------------------------------------------------------------------------------------------ keys

    key(code, down) {
        const t = now();
        this.vk.notify_key(t, code, down ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
        this.rec({ev: 'inject', code, down, tIssue: t});
    }

    async tap(code, hold = 25) {
        this.key(code, true);
        await this.sleep(hold);
        this.key(code, false);
        await this.sleep(25);
    }

    async chord(mods, code) {
        for (const m of mods) {
            this.key(m, true);
            await this.sleep(15);
        }
        await this.tap(code);
        for (const m of [...mods].reverse()) {
            this.key(m, false);
            await this.sleep(15);
        }
    }

    // Alt+A at a human pace.
    async prefixKey() {
        await this.chord([K.LEFTALT], K.A);
    }

    // ------------------------------------------------------------------------------------------------ helpers

    async scenario(name, fn) {
        this.scen = name;
        this.rec({ev: 'scenario-start'});
        try {
            await fn();
            await this.waitIdle();
        } catch (e) {
            this.rec({ev: 'error', what: `${e}\n${e.stack}`});
            if (this.prefix)
                this._pop(this.prefix, 'scenario-error');
        }
        await this.sleep(400);
        this.rec({ev: 'scenario-end'});
        this.scen = null;
        this.flush();
    }

    findWindow(match) {
        return global.display.list_all_windows().find(w =>
            match.test(`${w.get_wm_class() ?? ''} ${w.get_title() ?? ''} ${w.get_gtk_application_id() ?? ''}`));
    }

    async waitWindow(match, secs = 40) {
        const end = now() + secs * 1e6;
        let w;
        while (!(w = this.findWindow(match)) && now() < end)
            await this.sleep(100);
        return w ?? null;
    }

    async focus(win) {
        // The shell opens the overview at startup; keys would go to its search entry.
        for (let i = 0; i < 5 && global.display.focus_window !== win; i++) {
            if (Main.overview.visible)
                Main.overview.hide();
            await this.sleep(400);
            Main.activateWindow(win, global.display.get_current_time_roundtrip());
            await this.sleep(600);
        }
        const ok = global.display.focus_window === win && !Main.overview.visible;
        this.rec({ev: 'focus', ok, title: win.get_title(), wmClass: win.get_wm_class(),
            overview: Main.overview.visible, actionMode: Main.actionMode,
            client: win.get_client_type() === Meta.WindowClientType.X11 ? 'x11' : 'wayland', ...this.imState()});
        return ok;
    }

    imState() {
        const im = Main.inputMethod;
        let source = null;
        try {
            source = Keyboard.getInputSourceManager().currentSource?.id ?? null;
        } catch (e) {
            source = `error ${e}`;
        }
        return {imFocus: !!im?.currentFocus, ibusContext: !!im?._context, inputSource: source};
    }

    keymapState() {
        const r = {};
        try {
            const ism = Keyboard.getInputSourceManager();
            const km = ism._keyboardManager;
            r.xkbOptions = km._xkbOptions === undefined ? 'undefined' : km._xkbOptions;
            r.currentKeymap = km._currentKeymap ?? null;
            r.shortName = km.shortName;
            r.ibusReady = ism._ibusReady;
            r.sources = Object.values(ism.inputSources ?? {}).map(s => `${s.type}:${s.id}`);
        } catch (e) {
            r.error = `${e}`;
        }
        return r;
    }

    spawn(cmd) {
        try {
            GLib.spawn_async(null, ['/bin/sh', '-c', cmd], null, GLib.SpawnFlags.DEFAULT, null);
            this.rec({ev: 'note', what: 'spawned', cmd});
        } catch (e) {
            this.rec({ev: 'error', what: `spawn ${cmd}: ${e}`});
        }
    }
}
