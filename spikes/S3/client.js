// Spike S3 test client (throwaway). A GTK4 window that logs every key, click, focus change and pointer crossing it
// receives, as JSON lines with CLOCK_MONOTONIC times, to $S3_CLIENT_LOG. Run with GDK_BACKEND=wayland (a native
// Wayland client) or GDK_BACKEND=x11 (an Xwayland client, the way games run).
//
//   gjs -m client.js NAME

import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';

const name = ARGV[0] ?? 's3client';
GLib.set_prgname('s3client');

const out = Gio.File.new_for_path(GLib.getenv('S3_CLIENT_LOG')).append_to(Gio.FileCreateFlags.NONE, null);

function log(obj) {
    const line = JSON.stringify({client: name, t: GLib.get_monotonic_time(), ...obj});
    out.write_all(new TextEncoder().encode(`${line}\n`), null);
    out.flush(null);
}

const app = new Gtk.Application({
    application_id: 'org.vase.S3Client',
    flags: Gio.ApplicationFlags.NON_UNIQUE,
});

app.connect('activate', () => {
    const win = new Gtk.ApplicationWindow({application: app, title: `s3-${name}`, default_width: 600, default_height: 400});
    const area = new Gtk.DrawingArea({hexpand: true, vexpand: true, focusable: true, can_focus: true});
    win.set_child(area);

    const keys = new Gtk.EventControllerKey({propagation_phase: Gtk.PropagationPhase.CAPTURE});
    keys.connect('key-pressed', (_c, keyval, keycode, state) => {
        log({ev: 'key-press', key: Gdk.keyval_name(keyval), keycode, state});
        return true;
    });
    keys.connect('key-released', (_c, keyval, keycode, state) => {
        log({ev: 'key-release', key: Gdk.keyval_name(keyval), keycode, state});
    });
    keys.connect('modifiers', (_c, state) => {
        log({ev: 'modifiers', state});
        return false;
    });
    win.add_controller(keys);

    const click = new Gtk.GestureClick({button: 0});
    click.connect('pressed', (g, n, x, y) => log({ev: 'click-press', button: g.get_current_button(), n, x, y}));
    click.connect('released', (g, n, x, y) => log({ev: 'click-release', button: g.get_current_button(), n, x, y}));
    area.add_controller(click);

    const motion = new Gtk.EventControllerMotion();
    motion.connect('enter', (_c, x, y) => log({ev: 'pointer-enter', x, y}));
    motion.connect('leave', () => log({ev: 'pointer-leave'}));
    area.add_controller(motion);

    const focus = new Gtk.EventControllerFocus();
    focus.connect('enter', () => log({ev: 'focus-enter'}));
    focus.connect('leave', () => log({ev: 'focus-leave'}));
    win.add_controller(focus);

    win.connect('notify::is-active', () => log({ev: 'active', value: win.is_active}));
    win.present();
    area.grab_focus();
    log({ev: 'ready', backend: GLib.getenv('GDK_BACKEND'), prgname: GLib.get_prgname(), display: win.get_display().constructor.name});
});

app.run([]);
