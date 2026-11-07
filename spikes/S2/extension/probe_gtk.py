#!/usr/bin/python3 -I
# Spike S2 (throwaway): a GTK4 window with one entry that logs, as JSON lines with CLOCK_MONOTONIC times, every key
# event GTK delivers, focus changes, preedit and every change of the entry's text. Run with WAYLAND_DEBUG=1 to also
# get the wl_keyboard and zwp_text_input_v3 traffic on stderr.
import json
import sys
import time

import gi

gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: E402

out = open(sys.argv[1], 'a', buffering=1)


def log(kind, **kw):
    kw['kind'] = kind
    kw['t'] = time.monotonic()
    out.write(json.dumps(kw) + '\n')


def on_activate(app):
    win = Gtk.ApplicationWindow(application=app, title='s2-gtk-probe')
    win.set_default_size(420, 120)
    entry = Gtk.Entry()
    win.set_child(entry)

    key = Gtk.EventControllerKey()
    key.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
    win.add_controller(key)

    def pressed(c, keyval, keycode, state):
        log('key-pressed', name=Gdk.keyval_name(keyval), keycode=keycode, state=int(state),
            evtime=c.get_current_event_time())
        return False

    def released(c, keyval, keycode, state):
        log('key-released', name=Gdk.keyval_name(keyval), keycode=keycode, state=int(state),
            evtime=c.get_current_event_time())

    key.connect('key-pressed', pressed)
    key.connect('key-released', released)
    key.connect('modifiers', lambda c, st: log('modifiers', state=int(st)) or False)

    focus = Gtk.EventControllerFocus()
    entry.add_controller(focus)
    focus.connect('enter', lambda c: log('focus-enter'))
    focus.connect('leave', lambda c: log('focus-leave'))
    win.connect('notify::is-active', lambda w, p: log('is-active', value=w.is_active()))
    entry.connect('changed', lambda e: log('changed', text=e.get_text()))
    entry.get_delegate().connect('preedit-changed', lambda t, s: log('preedit', text=s))

    settings = Gtk.Settings.get_default()
    log('start', im_module=settings.props.gtk_im_module, wall=time.time(),
        gtk=f'{Gtk.get_major_version()}.{Gtk.get_minor_version()}.{Gtk.get_micro_version()}')
    win.present()


app = Gtk.Application(application_id='io.github.maartenhaine.S2Probe', flags=Gio.ApplicationFlags.NON_UNIQUE)
app.connect('activate', on_activate)
GLib.set_prgname('io.github.maartenhaine.S2Probe')
app.run([sys.argv[0]])
