// Spike S2 (throwaway): the scenarios, run against each focused client. See s2core.js.

import Clutter from 'gi://Clutter';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {K} from './s2core.js';

// A fast Alt+A then '\': a↓ at 0, a↑ at g/3, Alt↑ at 2g/3, '\'↓ at g (all in one main-loop turn when g is 0).
// blocked: the gaps are spun with the main loop blocked, so all four wait in the queue together.
async function fast(s, g, blocked) {
    s.key(K.LEFTALT, true);
    await s.sleep(30);
    const steps = [[K.A, true], [K.A, false], [K.LEFTALT, false], [K.BACKSLASH, true]];
    for (let i = 0; i < steps.length; i++) {
        if (i > 0 && g > 0) {
            if (blocked)
                s.spin(g / 3);
            else
                await s.sleep(g / 3);
        }
        s.key(...steps[i]);
    }
    await s.sleep(40);
    s.key(K.BACKSLASH, false);
}

// The prefix at a human pace, then '\'.
async function slow(s) {
    await s.prefixKey();
    await s.sleep(150);
    await s.tap(K.BACKSLASH, 40);
}

export async function baseline(s) {
    await s.scenario('baseline', async () => {
        await s.tap(K.X);
        await s.tap(K.BACKSLASH);
        await s.tap(K.MINUS);
    });
}

export async function common(s, gaps) {
    await baseline(s);
    await s.scenario('prefix-slow', () => slow(s));
    s.popOn = 'press';
    await s.scenario('prefix-slow-pop-on-press', () => slow(s));
    s.popOn = 'release';
    for (const g of gaps)
        await s.scenario(`fast-${g}ms`, () => fast(s, g, false));
}

export async function gtkOnly(s) {
    for (const g of [5, 10])
        await s.scenario(`fast-${g}ms-blocked`, () => fast(s, g, true));

    // Two keys after the command, before its release: typed into the grab, so lost unless replayed.
    await s.scenario('keys-before-pop', async () => {
        await s.prefixKey();
        await s.sleep(60);
        s.key(K.BACKSLASH, true);
        await s.sleep(10);
        await s.tap(K.Q, 10);
        await s.tap(K.Z, 10);
        s.key(K.BACKSLASH, false);
    });

    await s.scenario('timeout', async () => {
        await s.prefixKey();
        await s.sleep(3300);
        await s.tap(K.Y);
    });

    await s.scenario('escape', async () => {
        await s.prefixKey();
        await s.sleep(100);
        await s.tap(K.ESC);
        await s.sleep(100);
        await s.tap(K.Y);
    });

    await s.scenario('kill-chord-outside-grab', async () => {
        await s.chord([K.LEFTCTRL, K.LEFTMETA, K.LEFTALT], K.ESC);
    });
    await s.scenario('kill-chord-in-none-grab', async () => {
        await s.prefixKey();
        await s.sleep(100);
        await s.chord([K.LEFTCTRL, K.LEFTMETA, K.LEFTALT], K.ESC);
    });

    await s.scenario('super-l-outside-grab', async () => {
        await s.chord([K.LEFTMETA], K.L);
    });
    await s.scenario('super-l-in-none-grab', async () => {
        await s.prefixKey();
        await s.sleep(100);
        await s.chord([K.LEFTMETA], K.L);
        await s.sleep(100);
        await s.tap(K.ESC);
    });

    // The same chords inside a NORMAL grab (the Overlay state's mode).
    s.nextMode = Shell.ActionMode.NORMAL;
    await s.scenario('normal-grab-super-l-and-alt-a', async () => {
        await s.prefixKey();
        await s.sleep(100);
        await s.chord([K.LEFTMETA], K.L);
        await s.sleep(100);
        await s.prefixKey();
        await s.sleep(100);
        await s.tap(K.ESC);
    });
    await s.scenario('normal-grab-kill-chord', async () => {
        await s.prefixKey();
        await s.sleep(100);
        await s.chord([K.LEFTCTRL, K.LEFTMETA, K.LEFTALT], K.ESC);
    });
    s.nextMode = Shell.ActionMode.NONE;

    // Another modal on top (a system modal dialog, the lock screen) revokes the prefix's grab.
    await s.scenario('revoked-by-modal', async () => {
        await s.prefixKey();
        await s.sleep(100);
        const other = new St.Widget({reactive: true, can_focus: true, width: 1, height: 1});
        Main.uiGroup.add_child(other);
        other.connect('key-press-event', (_a, e) => {
            s.rec({ev: 'other-modal-key', sym: e.get_key_symbol()});
            return Clutter.EVENT_STOP;
        });
        const g2 = Main.pushModal(other, {actionMode: Shell.ActionMode.SYSTEM_MODAL});
        s.rec({ev: 'note', what: 'pushed second modal', prefixRevoked: s.prefix?.grab.is_revoked()});
        await s.sleep(100);
        await s.tap(K.X);
        await s.sleep(100);
        Main.popModal(g2);
        other.destroy();
        s.rec({ev: 'note', what: 'popped second modal', prefixRevoked: s.prefix?.grab.is_revoked()});
        await s.sleep(100);
        await s.tap(K.BACKSLASH);
    });

    // Can the shell see keys meant for a client without a grab? An event filter added now runs after mutter's
    // (g_list_append), and the stage's captured-event.
    await s.scenario('observe-without-grab', async () => {
        let filterKeys = 0, capturedKeys = 0, fid = 0, error = null;
        const isKey = e => [Clutter.EventType.KEY_PRESS, Clutter.EventType.KEY_RELEASE].includes(e.type());
        try {
            fid = Clutter.Event.add_filter(global.stage, e => {
                if (isKey(e))
                    filterKeys++;
                return Clutter.EVENT_PROPAGATE;
            });
        } catch (e) {
            error = `${e}`;
        }
        const cid = global.stage.connect('captured-event', (_a, e) => {
            if (isKey(e))
                capturedKeys++;
            return Clutter.EVENT_PROPAGATE;
        });
        await s.tap(K.Z);
        await s.tap(K.Z);
        await s.sleep(100);
        if (fid)
            Clutter.Event.remove_filter(fid);
        global.stage.disconnect(cid);
        s.rec({ev: 'observed', filterKeys, capturedKeys, injectedKeyEvents: 4, error});
    });

    // A temporary accelerator for one sticky key: it fires and stops the key; other keys still reach the app.
    await s.scenario('temporary-accelerator', async () => {
        s.shiftLeft = s._grab('<Shift>Left', Shell.ActionMode.NORMAL);
        await s.chord([K.LEFTSHIFT], K.LEFT);
        await s.tap(K.Q);
        global.display.ungrab_accelerator(s.shiftLeft);
        s.rec({ev: 'note', what: 'ungrabbed Shift+Left'});
        s.shiftLeft = 0;
        await s.sleep(50);
        await s.chord([K.LEFTSHIFT], K.LEFT);
    });

    // The same with TRIGGER_RELEASE, so mutter also takes the key's release (keybindings.c process_event).
    await s.scenario('temporary-accelerator-trigger-release', async () => {
        s.shiftLeft = s._grab('<Shift>Left', Shell.ActionMode.NORMAL, Meta.KeyBindingFlags.TRIGGER_RELEASE);
        await s.chord([K.LEFTSHIFT], K.LEFT);
        await s.tap(K.Q);
        global.display.ungrab_accelerator(s.shiftLeft);
        s.shiftLeft = 0;
    });

    // Alt+Tab inside a NORMAL grab: the switcher's own pushModal revokes ours, and gives it back when it closes.
    s.nextMode = Shell.ActionMode.NORMAL;
    await s.scenario('normal-grab-alt-tab', async () => {
        await s.prefixKey();
        await s.sleep(100);
        s.key(K.LEFTALT, true);
        await s.sleep(20);
        await s.tap(K.TAB);
        await s.sleep(400);
        s.key(K.LEFTALT, false);
        await s.sleep(300);
        s.rec({ev: 'note', what: 'after alt-tab', focus: global.display.focus_window?.get_title(),
            prefixArmed: !!s.prefix, prefixRevoked: s.prefix?.grab.is_revoked() ?? null});
        await s.tap(K.ESC);
    });
    s.nextMode = Shell.ActionMode.NONE;
}

