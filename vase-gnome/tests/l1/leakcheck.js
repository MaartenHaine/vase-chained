// What an extension leaves behind after disable() (docs/linux-tiler/wayfinder.md §5: every L1 case ends with an
// enable, disable, enable cycle and asserts that nothing is left behind).
//
// While the extension's own enable() or disable() runs, the checker records every signal connection, main-loop source,
// accelerator and keybinding it makes. After disable() each must be gone. It also compares the modal state
// (Main.modalCount, the stage's grab actor) and the stage's actor tree with a snapshot() taken while the extension was
// off, and looks for GObjects the extension object still holds.
//
// Main-loop sources are the GLib id adders (timeout_add, idle_add and their variants) and every GLib.Source attached
// from JavaScript, which covers setTimeout and setInterval: GJS builds them on GLib.timeout_source_new() and
// source.attach(). Connections on an actor that was destroyed went with it; an actor that was only taken off the stage
// is still checked.
//
// What it can't see: anything done later from a callback or after an await (it records only synchronous work inside
// enable() and disable()); sources attached from C on the extension's behalf; connections through objects that copy
// their signal methods with Signals.addSignalMethods instead of extending Signals.EventEmitter; and a pushModal that was
// popped by someone else (Main is a module namespace, so pushModal can't be wrapped).

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Signals from 'resource:///org/gnome/shell/misc/signals.js';

const SOURCE_ADDERS = ['timeout_add', 'timeout_add_seconds', 'idle_add', 'timeout_add_once', 'timeout_add_seconds_once', 'idle_add_once'];

/**
 * @param {Clutter.Actor} root
 * @returns {Set<Clutter.Actor>} the actor and all its descendants
 */
export function actorTree(root) {
    const seen = new Set();
    const walk = actor => {
        seen.add(actor);
        for (const child of actor.get_children())
            walk(child);
    };
    walk(root);
    return seen;
}

/**
 * What leaks() compares against: take it while the extension is off.
 *
 * @returns {{actors: Set<Clutter.Actor>, modalCount: number, grabActor: ?Clutter.Actor}}
 */
export function snapshot() {
    return {
        actors: actorTree(global.stage),
        modalCount: Main.modalCount,
        grabActor: global.stage.get_grab_actor(),
    };
}

export class LeakChecker {
    constructor() {
        this._recording = 0;
        this._restore = [];
        this.reset();
    }

    reset() {
        this.signals = [];
        this.sources = [];
        this._destroyed = new WeakSet();
        this._watched = new WeakSet();
        this.accelerators = new Set();
        this.keybindings = new Set();
    }

    /**
     * Runs fn with recording on.
     *
     * @param {Function} fn
     * @returns {*} what fn returns
     */
    record(fn) {
        this._recording++;
        try {
            return fn();
        } finally {
            this._recording--;
        }
    }

    /**
     * Wraps the extension object's enable() and disable() so that their synchronous work is recorded.
     *
     * @param {object} stateObj the extension's Extension instance
     */
    watchExtension(stateObj) {
        for (const name of ['enable', 'disable']) {
            const original = stateObj[name];
            const checker = this;
            stateObj[name] = function (...args) {
                return checker.record(() => original.apply(this, args));
            };
            this._restore.push(() => delete stateObj[name]);
        }
    }

    /** Installs the recording wrappers. */
    install() {
        const checker = this;
        const wrap = (owner, name, after) => {
            const original = owner[name];
            if (typeof original !== 'function')
                throw new Error(`leakcheck: ${name} is not a function, the check would be blind`);
            owner[name] = function (...args) {
                const result = original.apply(this, args);
                if (checker._recording > 0)
                    after(this, result, args);
                return result;
            };
            this._restore.push(() => {
                owner[name] = original;
            });
        };

        // Each actor the extension connects to is watched for its destroy, through the unwrapped connect, so that
        // leaks() skips only actors that are really gone.
        const rawConnect = GObject.Object.prototype.connect;
        const watch = obj => {
            if (!(obj instanceof Clutter.Actor) || this._watched.has(obj))
                return;
            this._watched.add(obj);
            rawConnect.call(obj, 'destroy', actor => this._destroyed.add(actor));
        };

        for (const name of ['connect', 'connect_after']) {
            wrap(GObject.Object.prototype, name, (obj, id, args) => {
                watch(obj);
                this.signals.push({obj, id, signal: args[0], kind: 'gobject'});
            });
        }
        wrap(Signals.EventEmitter.prototype, 'connect', (obj, id, args) => {
            watch(obj);
            this.signals.push({obj, id, signal: args[0], kind: 'js'});
        });

        for (const name of SOURCE_ADDERS)
            wrap(GLib, name, (_obj, id) => this.sources.push({id, adder: `GLib.${name}`}));
        // g_timeout_add and the other adders attach in C, so this sees only JavaScript's attaches: setTimeout,
        // setInterval and sources attached by hand.
        wrap(GLib.Source.prototype, 'attach', (source, id) =>
            this.sources.push({id, source, adder: 'GLib.Source.attach (setTimeout, setInterval or by hand)'}));

        wrap(global.display, 'grab_accelerator', (_obj, action) => this.accelerators.add(action));
        wrap(global.display, 'ungrab_accelerator', (_obj, _ok, args) => this.accelerators.delete(args[0]));
        wrap(Main.wm, 'addKeybinding', (_obj, _r, args) => this.keybindings.add(args[0]));
        wrap(Main.wm, 'removeKeybinding', (_obj, _r, args) => this.keybindings.delete(args[0]));
    }

    /** Removes every wrapper. */
    uninstall() {
        while (this._restore.length > 0)
            this._restore.pop()();
    }

    /**
     * Everything recorded since the last reset() that is still alive.
     *
     * @param {object} params
     * @param {object} params.baseline a snapshot() taken while the extension was off
     * @param {object} params.stateObj the extension's Extension instance
     * @returns {string[]} one line per leak; empty when nothing was left behind
     */
    leaks({baseline, stateObj}) {
        const leaks = [];
        const stage = actorTree(global.stage);

        for (const {obj, id, signal, kind} of this.signals) {
            // A connection on a destroyed actor went with the actor (and asking a disposed object about it warns).
            if (this._destroyed.has(obj))
                continue;
            const connected = kind === 'gobject'
                ? GObject.signal_handler_is_connected(obj, id)
                : obj.signalHandlerIsConnected(id);
            if (connected)
                leaks.push(`signal ${signal} (handler ${id}) on ${obj}`);
        }

        const context = GLib.MainContext.default();
        for (const {id, source, adder} of this.sources) {
            const alive = source ? !source.is_destroyed() : context.find_source_by_id(id);
            if (alive)
                leaks.push(`main-loop source ${id} from ${adder}`);
        }

        if (Main.modalCount !== baseline.modalCount)
            leaks.push(`Main.modalCount is ${Main.modalCount}, it was ${baseline.modalCount}`);
        if (global.stage.get_grab_actor() !== baseline.grabActor)
            leaks.push(`the stage's grab is on ${global.stage.get_grab_actor()}, it was on ${baseline.grabActor}`);
        for (const action of this.accelerators)
            leaks.push(`accelerator action ${action} still grabbed`);
        for (const name of this.keybindings)
            leaks.push(`keybinding ${name} still added`);

        for (const actor of stage) {
            if (!baseline.actors.has(actor))
                leaks.push(`actor ${actor} (name ${actor.name}) is on the stage`);
        }

        for (const [key, value] of Object.entries(stateObj)) {
            if (value instanceof GObject.Object)
                leaks.push(`the extension still holds ${key} = ${value}`);
        }
        return leaks;
    }
}
