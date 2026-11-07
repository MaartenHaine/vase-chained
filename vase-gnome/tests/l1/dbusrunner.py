"""A mocked system bus for the L1 shell, then the command after `--`.

A trimmed copy of gnome-shell's tests/gnomeshell_dbusrunner.py (50.1), without its MutterDBusRunner base: the shell
under test gets a private system bus on which python-dbusmock answers for the system services it looks up, so it never
reaches a real logind, polkit, NetworkManager or display manager (docs/linux-tiler/wayfinder.md F18). The session bus
is the private one dbus-run-session started around this script.

Run it as `dbus-run-session -- python3 -I dbusrunner.py -- COMMAND...`, inside the chained-tiler-test container only.
"""

import os
import subprocess
import sys

import dbusmock

# Templates python-dbusmock ships, started on the private system bus. gnome-shell's runner also mocks AccountsService
# from a template of its own, which the archive's python3-dbusmock doesn't have. logind is left out: without
# /run/systemd/seats the shell uses its dummy login manager anyway (js/misc/loginManager.js, haveSystemd()).
SYSTEM_TEMPLATES = ['polkitd', 'networkmanager', 'upower']

# Services without a dbusmock template: (bus name, object path, interface, system bus?, methods).
CLASS_MOCKS = [
    ('org.gnome.DisplayManager', '/org/gnome/DisplayManager/Manager', 'org.gnome.DisplayManager.Manager', True,
     [('RegisterSession', 'a{sv}', '', '')]),
    ('org.freedesktop.GeoClue2', '/org/freedesktop/GeoClue2/Manager', 'org.freedesktop.GeoClue2.Manager', True,
     [('AddAgent', 's', '', '')]),
    ('org.gnome.SessionManager', '/org/gnome/SessionManager', 'org.gnome.SessionManager', False,
     [('Setenv', 'ss', '', '')]),
    ('org.freedesktop.impl.portal.PermissionStore', '/org/freedesktop/impl/portal/PermissionStore',
     'org.freedesktop.impl.portal.PermissionStore', False,
     [('Lookup', 'ss', 'a{sas}v',
       'ret = (dbus.Dictionary([], signature="sas"), dbus.String("NONE", variant_level=2))')]),
    ('org.gnome.Shell.CalendarServer', '/org/gnome/Shell/CalendarServer', 'org.gnome.Shell.CalendarServer', False,
     [('SetTimeRange', 'xxb', '', '')]),
]


class Mocks(dbusmock.DBusTestCase):
    """Holds the bus and the mock processes."""


def die(message):
    print(f'dbusrunner: {message}', file=sys.stderr)
    sys.exit(2)


def main(argv):
    if '--' not in argv or argv.index('--') == len(argv) - 1:
        die('usage: dbusrunner.py -- COMMAND...')
    command = argv[argv.index('--') + 1:]

    if not os.path.exists('/run/.containerenv'):
        die('refusing to run outside a podman container')
    if not os.environ.get('DBUS_SESSION_BUS_ADDRESS'):
        die('no session bus: run this under dbus-run-session')

    log_dir = os.environ.get('L1_LOG_DIR', os.environ.get('TMPDIR', '/var/tmp'))
    mock_log = open(os.path.join(log_dir, 'dbusmock.log'), 'w')
    processes = []

    Mocks.start_system_bus()
    print(f'dbusrunner: private system bus at {os.environ["DBUS_SYSTEM_BUS_ADDRESS"]}', flush=True)
    try:
        for template in SYSTEM_TEMPLATES:
            process, _obj = Mocks.spawn_server_template(template, {}, mock_log, system_bus=True)
            processes.append(process)

        for name, path, interface, system_bus, methods in CLASS_MOCKS:
            processes.append(Mocks.spawn_server(name, path, interface, system_bus=system_bus, stdout=mock_log))
            obj = Mocks.get_dbus(system_bus=system_bus).get_object(name, path)
            obj.AddMethods(interface, methods, dbus_interface=dbusmock.MOCK_IFACE)

        return subprocess.call(command)
    finally:
        for process in processes:
            process.terminate()
            process.wait()
        Mocks.tearDownClass()
        mock_log.close()


if __name__ == '__main__':
    sys.exit(main(sys.argv))
