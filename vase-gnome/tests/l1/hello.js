// L1: the hello extension in a headless GNOME Shell 50 (docs/linux-tiler/wayfinder.md §5, M0a).
//
// gnome-shell-test-tool installs the extension in a throwaway home and enables it, so the shell starts with it on.
// This script then turns it off, takes a baseline, and runs the enable, disable, enable cycle every L1 case ends with,
// asserting that the disable left nothing behind. A throw here makes the shell exit non-zero, and so the test tool.

import GLib from 'gi://GLib';
import St from 'gi://St';

import {ExtensionState} from 'resource:///org/gnome/shell/misc/extensionUtils.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

import {LeakChecker, actorTree, snapshot} from './leakcheck.js';

const UUID = 'vase-hello@maartenhaine.github.io';
const LABEL_NAME = 'vase-hello-label';
const STATE_TIMEOUT_MS = 5000;

/**
 * @param {string} message
 */
function note(message) {
    console.log(`[l1] ${message}`);
}

/**
 * @param {boolean} condition
 * @param {string} message
 */
function check(condition, message) {
    if (!condition)
        throw new Error(`L1 check failed: ${message}`);
    note(`ok: ${message}`);
}

/**
 * @returns {object} the extension's record in the extension manager
 */
function extension() {
    const ext = Main.extensionManager.lookup(UUID);
    if (!ext)
        throw new Error(`${UUID} is not installed in the test shell`);
    return ext;
}

/**
 * @param {number} state an ExtensionState
 */
async function waitForState(state) {
    const deadline = GLib.get_monotonic_time() + STATE_TIMEOUT_MS * 1000;
    while (extension().state !== state) {
        if (extension().state === ExtensionState.ERROR)
            throw new Error(`${UUID} is in ERROR: ${extension().error}`);
        if (GLib.get_monotonic_time() > deadline)
            throw new Error(`${UUID} did not reach state ${state} (it is ${extension().state})`);
        // eslint-disable-next-line no-await-in-loop
        await Scripting.sleep(20);
    }
    await Scripting.waitLeisure();
}

/**
 * @returns {object[]} the hello labels on the stage
 */
function labels() {
    return [...actorTree(global.stage)].filter(a => a.name === LABEL_NAME);
}

/**
 * @param {string} when
 */
function checkEnabled(when) {
    check(extension().state === ExtensionState.ACTIVE, `${when}: the extension is ACTIVE`);
    const found = labels();
    check(found.length === 1, `${when}: exactly one hello label is on the stage (found ${found.length})`);
    check(found[0].text === 'vase hello', `${when}: the label reads "vase hello"`);
    check(found[0].visible && found[0].mapped, `${when}: the label is shown`);
}

/**
 * The checker has to catch a leak, or a clean result means nothing.
 *
 * @param {LeakChecker} checker
 * @param {object} baseline a snapshot()
 */
function checkTheChecker(checker, baseline) {
    checker.reset();
    let actor, signalId, sourceId, interval, removed;
    checker.record(() => {
        actor = new St.Widget({name: 'l1-planted-leak'});
        Main.layoutManager.uiGroup.add_child(actor);
        signalId = Main.layoutManager.connect('monitors-changed', () => {});
        sourceId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 3600, () => GLib.SOURCE_REMOVE);
        interval = setInterval(() => {}, 3600 * 1000);
        // Taken off the stage but not destroyed: its handler is still connected.
        removed = new St.Widget({name: 'l1-planted-removed'});
        Main.layoutManager.uiGroup.add_child(removed);
        removed.connect('notify::mapped', () => {});
        Main.layoutManager.uiGroup.remove_child(removed);
        // Destroyed: its handler went with it, so it is not a leak.
        const gone = new St.Widget({name: 'l1-planted-destroyed'});
        Main.layoutManager.uiGroup.add_child(gone);
        gone.connect('notify::mapped', () => {});
        gone.destroy();
    });
    const found = checker.leaks({baseline, stateObj: {}});
    actor.destroy();
    removed.destroy();
    Main.layoutManager.disconnect(signalId);
    GLib.Source.remove(sourceId);
    clearInterval(interval);
    check(found.length === 5,
        'the leak checker finds a planted actor, signal, source, setInterval and signal on a removed actor, and ' +
        `skips a destroyed one (found: ${found.join('; ')})`);
    checker.reset();
}

/** Runs the case. */
export async function run() {
    await waitForState(ExtensionState.ACTIVE);
    // The shell starts in the overview, which holds a modal grab; leave it, so the baseline is the plain desktop.
    if (Main.overview.visible) {
        Main.overview.hide();
        await Scripting.sleep(500);
        await Scripting.waitLeisure();
    }
    check(!Main.overview.visible, 'the overview is closed');
    check(Main.modalCount === 0 && global.stage.get_grab_actor() === null, 'no modal grab is up on the desktop');
    checkEnabled('at startup');

    Main.extensionManager.disableExtension(UUID);
    await waitForState(ExtensionState.INACTIVE);
    check(labels().length === 0, 'after the first disable: no hello label is left');

    const checker = new LeakChecker();
    checker.install();
    try {
        const baseline = snapshot();
        checkTheChecker(checker, baseline);
        checker.watchExtension(extension().stateObj);

        Main.extensionManager.enableExtension(UUID);
        await waitForState(ExtensionState.ACTIVE);
        checkEnabled('after enable');
        check(checker.signals.length > 0, `the checker saw the extension connect (${checker.signals.length} signals)`);

        Main.extensionManager.disableExtension(UUID);
        await waitForState(ExtensionState.INACTIVE);
        const leaks = checker.leaks({baseline, stateObj: extension().stateObj});
        for (const leak of leaks)
            note(`leak: ${leak}`);
        check(leaks.length === 0, 'after enable and disable: no actor, signal, source, grab or held object is left');

        Main.extensionManager.enableExtension(UUID);
        await waitForState(ExtensionState.ACTIVE);
        checkEnabled('after the cycle');
    } finally {
        checker.uninstall();
    }
    note('PASS');
}
