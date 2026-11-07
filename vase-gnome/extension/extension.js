// The hello extension (docs/linux-tiler/wayfinder.md §5, M0a): it logs enable and disable and draws one St label on
// the primary screen. It is the fixture the L1 harness and the CI lint grow from, not the Tiler.
//
// It already follows the extension's teardown rule (wayfinder §3.1): disable() undoes enable() in separate steps, each
// in its own try/finally, so one throw can't skip the rest, and enable() cleans up after itself when it fails, because
// the shell never calls disable() on an extension whose enable() threw.

import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const LOG_PREFIX = '[vase-hello]';
const LABEL_NAME = 'vase-hello-label';
const MARGIN = 16;

export default class VaseHelloExtension extends Extension {
    enable() {
        console.log(`${LOG_PREFIX} enable`);
        try {
            this._label = new St.Label({name: LABEL_NAME, text: 'vase hello'});
            Main.layoutManager.addChrome(this._label);
            this._monitorsChangedId = Main.layoutManager.connect('monitors-changed', () => this._place());
            this._place();
        } catch (e) {
            this._teardown();
            throw e;
        }
    }

    disable() {
        console.log(`${LOG_PREFIX} disable`);
        this._teardown();
    }

    _place() {
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor || !this._label)
            return;
        this._label.set_position(monitor.x + MARGIN, monitor.y + monitor.height - this._label.height - MARGIN);
    }

    _teardown() {
        try {
            if (this._monitorsChangedId)
                Main.layoutManager.disconnect(this._monitorsChangedId);
        } catch (e) {
            console.error(`${LOG_PREFIX} disconnecting monitors-changed: ${e}`);
        } finally {
            this._monitorsChangedId = 0;
        }

        try {
            // addChrome tracks the actor's destroy signal and untracks it itself.
            this._label?.destroy();
        } catch (e) {
            console.error(`${LOG_PREFIX} destroying the label: ${e}`);
        } finally {
            this._label = null;
        }
    }
}
