// Spike S2 (throwaway), the L3 half: started by tools/dev-shell.sh in the tiler-dev mode (spikes/S2/run-l3.sh puts
// this directory in place of vase-gnome/extension for the run). It starts three clients inside the nested shell
// (a GTK4 entry, Ghostty, xev through Xwayland), runs the scenarios against each with a Clutter virtual keyboard,
// writes $HOME/s2/shell.jsonl (HOME is the run dir's) and ends the nested shell.

import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {S2} from './s2core.js';
import * as Sc from './scenarios.js';

export default class S2Extension extends Extension {
    enable() {
        console.log('[s2] enable');
        this._s2 = new S2({dir: this.path, outDir: `${GLib.get_home_dir()}/s2`, level: 'L3'});
        if (Main.layoutManager._startingUp) {
            this._startId = Main.layoutManager.connect('startup-complete', () => {
                Main.layoutManager.disconnect(this._startId);
                this._startId = 0;
                this._run();
            });
        } else {
            this._run();
        }
    }

    disable() {
        console.log('[s2] disable');
        if (this._startId)
            Main.layoutManager.disconnect(this._startId);
        this._s2?.teardown();
        this._s2 = null;
    }

    async _run() {
        const s = this._s2;
        try {
            s.rec({ev: 'note', what: 'start', keymap: s.keymapState(), im: s.imState(),
                display: GLib.getenv('DISPLAY'), wayland: GLib.getenv('WAYLAND_DISPLAY')});
            const end = GLib.get_monotonic_time() + 30e6;
            while (!Main.inputMethod?._context && GLib.get_monotonic_time() < end)
                await s.sleep(200);
            s.rec({ev: 'note', what: 'ibus', im: s.imState(), keymap: s.keymapState()});
            s.setup();

            const out = s.outDir, dir = s.dir;
            s.spawn(`WAYLAND_DEBUG=1 exec /usr/bin/python3 -I ${dir}/probe_gtk.py ${out}/gtk.jsonl ` +
                `2>${out}/gtk-wayland.log`);
            s.spawn(`exec /usr/bin/ghostty --gtk-single-instance=false --title=s2-ghostty ` +
                `-e /usr/bin/python3 -I ${dir}/bytelog.py ${out}/ghostty.jsonl >${out}/ghostty.log 2>&1`);
            if (GLib.getenv('DISPLAY')) {
                s.spawn(`exec /usr/bin/stdbuf -oL /usr/bin/xev -name s2-xev -event keyboard -event focus ` +
                    `>${out}/xev.log 2>&1`);
            }

            const targets = [
                ['gtk', /S2Probe|s2-gtk-probe/, async () => {
                    await Sc.common(s, [0, 2, 5, 10, 20]);
                    await Sc.gtkOnly(s);
                }],
                ['ghostty', /ghostty/i, () => Sc.common(s, [5, 10])],
                ['xev', /s2-xev|Event Tester|xev/i, () => Sc.common(s, [5, 10])],
            ];
            for (const [name, re, fn] of targets) {
                const w = await s.waitWindow(re, name === 'gtk' ? 60 : 30);
                s.target = name;
                if (!w) {
                    s.rec({ev: 'error', what: `no ${name} window`,
                        windows: global.display.list_all_windows().map(x => `${x.get_wm_class()}|${x.get_title()}`)});
                    s.target = null;
                    continue;
                }
                await s.sleep(1500); // let the client settle (Ghostty's shell, xev's map)
                if (!await s.focus(w))
                    s.rec({ev: 'error', what: `couldn't focus ${name}`});
                await fn();
                s.target = null;
            }
            s.rec({ev: 'note', what: 'done', keymap: s.keymapState(), im: s.imState()});
        } catch (e) {
            s.rec({ev: 'error', what: `${e}\n${e.stack}`});
        }
        s.teardown();
        s.flush();
        console.log('[s2] DONE');
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
            global.context.terminate();
            return GLib.SOURCE_REMOVE;
        });
    }
}
