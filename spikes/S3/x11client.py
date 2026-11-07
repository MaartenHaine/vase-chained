"""Spike S3 X11 test client (throwaway): a bare Xlib window, through Xwayland, the way games run.

It logs every key, button, focus and crossing event it receives as JSON lines with CLOCK_MONOTONIC times (the clock
g_get_monotonic_time() reads), to $S3_CLIENT_LOG, in the same shape as client.js. Plain ctypes over libX11, so the
WM_CLASS is known exactly (the test tool only manages X11 windows whose class is on its filter list).

    python3 -I x11client.py NAME
"""

import ctypes
import ctypes.util
import json
import os
import sys
import time

NAME = sys.argv[1] if len(sys.argv) > 1 else 'xwayland'
OUT = open(os.environ['S3_CLIENT_LOG'], 'a', buffering=1)

x = ctypes.CDLL(ctypes.util.find_library('X11') or 'libX11.so.6')


class XAnyEvent(ctypes.Structure):
    _fields_ = [('type', ctypes.c_int), ('serial', ctypes.c_ulong), ('send_event', ctypes.c_int),
                ('display', ctypes.c_void_p), ('window', ctypes.c_ulong)]


class XKeyEvent(ctypes.Structure):
    # XKeyEvent and XButtonEvent share this layout; the last field is keycode or button.
    _fields_ = XAnyEvent._fields_ + [
        ('root', ctypes.c_ulong), ('subwindow', ctypes.c_ulong), ('time', ctypes.c_ulong),
        ('x', ctypes.c_int), ('y', ctypes.c_int), ('x_root', ctypes.c_int), ('y_root', ctypes.c_int),
        ('state', ctypes.c_uint), ('code', ctypes.c_uint), ('same_screen', ctypes.c_int)]


class XEvent(ctypes.Union):
    _fields_ = [('type', ctypes.c_int), ('any', XAnyEvent), ('key', XKeyEvent), ('pad', ctypes.c_long * 24)]


class XClassHint(ctypes.Structure):
    _fields_ = [('res_name', ctypes.c_char_p), ('res_class', ctypes.c_char_p)]


x.XOpenDisplay.restype = ctypes.c_void_p
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XDefaultScreen.argtypes = [ctypes.c_void_p]
x.XRootWindow.restype = ctypes.c_ulong
x.XRootWindow.argtypes = [ctypes.c_void_p, ctypes.c_int]
x.XCreateSimpleWindow.restype = ctypes.c_ulong
x.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong] + [ctypes.c_int] * 2 + [ctypes.c_uint] * 3 + \
    [ctypes.c_ulong] * 2
x.XSetClassHint.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(XClassHint)]
x.XStoreName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_char_p]
x.XSelectInput.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_long]
x.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XFlush.argtypes = [ctypes.c_void_p]
x.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.POINTER(XEvent)]
x.XKeycodeToKeysym.restype = ctypes.c_ulong
x.XKeycodeToKeysym.argtypes = [ctypes.c_void_p, ctypes.c_ubyte, ctypes.c_int]
x.XKeysymToString.restype = ctypes.c_char_p
x.XKeysymToString.argtypes = [ctypes.c_ulong]

KEY_PRESS, KEY_RELEASE, BUTTON_PRESS, BUTTON_RELEASE, ENTER, LEAVE, FOCUS_IN, FOCUS_OUT = 2, 3, 4, 5, 7, 8, 9, 10
MASK = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 21)  # keys, buttons, crossing, focus


def log(**obj):
    OUT.write(json.dumps({'client': NAME, 't': time.monotonic_ns() // 1000, **obj}) + '\n')


dpy = x.XOpenDisplay(None)
if not dpy:
    log(ev='error', error='XOpenDisplay failed')
    sys.exit(1)
win = x.XCreateSimpleWindow(dpy, x.XRootWindow(dpy, x.XDefaultScreen(dpy)), 0, 0, 600, 400, 0, 0, 0xffffff)
hint = XClassHint(b's3x', b'S3client')
x.XSetClassHint(dpy, win, ctypes.byref(hint))
x.XStoreName(dpy, win, f's3-{NAME}'.encode())
x.XSelectInput(dpy, win, MASK)
x.XMapWindow(dpy, win)
x.XFlush(dpy)
log(ev='ready', backend='x11-xlib')

ev = XEvent()
while True:
    x.XNextEvent(dpy, ctypes.byref(ev))
    t = ev.type
    if t in (KEY_PRESS, KEY_RELEASE):
        sym = x.XKeysymToString(x.XKeycodeToKeysym(dpy, ev.key.code, 0))
        # GDK's ALT_MASK is X11's Mod1Mask (8), so brief() in s3.js reads both alike.
        log(ev='key-press' if t == KEY_PRESS else 'key-release', key=(sym or b'?').decode(), keycode=ev.key.code,
            state=ev.key.state)
    elif t in (BUTTON_PRESS, BUTTON_RELEASE):
        log(ev='click-press' if t == BUTTON_PRESS else 'click-release', button=ev.key.code, x=ev.key.x, y=ev.key.y)
    elif t in (ENTER, LEAVE):
        log(ev='pointer-enter' if t == ENTER else 'pointer-leave')
    elif t in (FOCUS_IN, FOCUS_OUT):
        log(ev='focus-enter' if t == FOCUS_IN else 'focus-leave')
