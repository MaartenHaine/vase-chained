// M0a hello (docs/linux-tiler/wayfinder.md §5): logs enable and disable and draws one St label.
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

export default class VaseHello extends Extension {
    enable() {
        console.log(`${this.uuid}: enable`);
        this._label = new St.Label({text: 'vase hello', x: 48, y: 48});
        Main.layoutManager.addTopChrome(this._label);
    }

    disable() {
        console.log(`${this.uuid}: disable`);
        this._label?.destroy();
        this._label = null;
    }
}
