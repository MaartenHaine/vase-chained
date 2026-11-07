#!/usr/bin/env bash
# Spike S9's L1 run, inside chained-tiler-test (spikes/S9/run.sh l1 starts it). Throwaway.
# Packs spikes/S9/extension, then runs gnome-shell-test-tool --headless --extension with spikes/S9/l1/s9.js under
# dbus-run-session and the M0a dbusrunner (a mocked private system bus), with TMPDIR inside the container. The stub
# daemon is the binary in the chained-tiler-s9-target volume, mounted read-only at /opt/s9.
set -euo pipefail

if [ ! -e /run/.containerenv ]; then
	echo 's9: refusing to run outside a podman container' >&2
	exit 2
fi
[ -x /opt/s9/release/s9-stub ] || { echo 's9: /opt/s9/release/s9-stub is missing (spikes/S9/run.sh build)' >&2; exit 2; }

UUID=vase-s9@maartenhaine.github.io
export TMPDIR=/var/tmp/s9
install -d -m 0700 "$TMPDIR"
work=$(mktemp -d)

cp -r /src/spikes/S9/extension "$work/s9"
gnome-extensions pack "$work/s9" --out-dir "$work" --force
echo "s9: gnome-shell $(gnome-shell --version 2>/dev/null || true), gjs $(gjs --version 2>/dev/null || true), load $(cut -d' ' -f1-3 /proc/loadavg), $(nproc) cpus"

set +e
timeout --kill-after=10 "${S9_TIMEOUT:-600}" \
	dbus-run-session -- python3 -I /src/vase-gnome/tests/l1/dbusrunner.py -- \
	gnome-shell-test-tool --headless --disable-animations --extra-filter org.gtk.Demo4 --extension "$work/$UUID.shell-extension.zip" \
	/src/spikes/S9/l1/s9.js >"$work/shell.log" 2>&1
status=$?
set -e

echo "s9: shell exit $status"
echo '----- full shell log -----'
cat "$work/shell.log"
