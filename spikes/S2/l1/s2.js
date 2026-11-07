// Spike S2 (throwaway), the L1 half: a gnome-shell-test-tool automation script, run headless inside the
// chained-tiler-test container (spikes/S2/l1/run.sh). The same scenarios as at L3, minus Ghostty and xev (not in
// the image), with the GTK4 probe if the image has python3-gi and GTK 4, the perf helper's window otherwise. There is
// no ibus in the image, so this is the path without an input method.

import GLib from 'gi://GLib';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

import {S2} from '../extension/s2core.js';
import * as Sc from '../extension/scenarios.js';

/** The test tool's entry point. */
export async function run() {
    const s = new S2({dir: GLib.getenv('S2_DIR'), outDir: GLib.getenv('S2_OUT'), level: 'L1'});
    await Scripting.waitLeisure();
    s.rec({ev: 'note', what: 'start', keymap: s.keymapState(), im: s.imState(), mode: Main.sessionMode.currentMode});
    s.setup();
    let re;
    if (GLib.getenv('S2_PROBE') === '1') {
        s.spawn(`WAYLAND_DEBUG=1 exec /usr/bin/python3 -I ${s.dir}/probe_gtk.py ${s.outDir}/gtk.jsonl ` +
            `2>${s.outDir}/gtk-wayland.log`);
        re = /S2Probe|s2-gtk-probe/;
    } else {
        await Scripting.createTestWindow({width: 400, height: 300, textInput: true});
        await Scripting.waitTestWindows();
        re = /perf-helper|Gnome-shell-perf-helper/i;
    }
    // A second window, so Alt+Tab has something to switch to.
    await Scripting.createTestWindow({width: 300, height: 200});
    await Scripting.waitTestWindows();
    const w = await s.waitWindow(re, 60);
    s.target = 'l1-client';
    if (w && await s.focus(w)) {
        await Sc.common(s, [0, 5, 10]);
        await Sc.gtkOnly(s);
    } else {
        s.rec({ev: 'error', what: 'no client window to focus',
            windows: global.display.list_all_windows().map(x => `${x.get_wm_class()}|${x.get_title()}`)});
    }
    s.target = null;
    s.rec({ev: 'note', what: 'done', keymap: s.keymapState(), im: s.imState()});
    s.teardown();
    s.flush();
    console.log('[s2] DONE');
}
