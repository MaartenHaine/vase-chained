#!/usr/bin/env bash
# Spike S2 (throwaway), the L1 half, inside the chained-tiler-test container. spikes/S2/run-l1.sh puts this file in
# place of vase-gnome/tests/l1/run.sh for one `tools/linux/podman.sh l1` (no network, the checkout read-only, no host
# bus or socket) and restores it afterwards. It prints the shell's S2 records and the probe's log.
set -euo pipefail

if [ ! -e /run/.containerenv ]; then
	echo 's2-l1: refusing to run outside a podman container' >&2
	exit 2
fi

export TMPDIR=/var/tmp/l1
install -d -m 0700 "$TMPDIR"
work=$(mktemp -d)
export S2_DIR="$work/ext" S2_OUT="$work/out"
cp -r /src/spikes/S2/extension "$S2_DIR"
mkdir -p "$S2_OUT"
glib-compile-schemas "$S2_DIR/schemas"
if python3 -c 'import gi; gi.require_version("Gtk", "4.0"); from gi.repository import Gtk' 2>/dev/null; then
	export S2_PROBE=1
fi
echo "s2-l1: S2_PROBE=${S2_PROBE:-0}"
dpkg-query -W gnome-shell libmutter-18-0 gjs python3-gi gir1.2-gtk-4.0 ibus 2>&1 || true

set +e
timeout --kill-after=10 "${L1_TIMEOUT:-300}" \
	dbus-run-session -- python3 -I /src/vase-gnome/tests/l1/dbusrunner.py -- \
	gnome-shell-test-tool --headless --disable-animations --extra-filter io.github.maartenhaine.S2Probe \
	/src/spikes/S2/l1/s2.js >"$work/shell.log" 2>&1
status=$?
set -e
echo "s2-l1: shell exit $status"
echo '=== shell.log (s2, errors)'
grep -E '\[s2\]|JS ERROR|Script failed|CRITICAL|TypeError' "$work/shell.log" || true
echo '=== shell.jsonl'
cat "$S2_OUT/shell.jsonl" 2>/dev/null || echo 'none'
echo '=== gtk.jsonl'
cat "$S2_OUT/gtk.jsonl" 2>/dev/null || echo 'none'
echo '=== gtk-wayland.log (keyboard, text-input)'
grep -E 'wl_keyboard|zwp_text_input' "$S2_OUT/gtk-wayland.log" 2>/dev/null | grep -v 'repeat_info' || echo 'none'
if [ -n "${L1_VERBOSE:-}" ]; then
	echo '=== full shell.log'
	cat "$work/shell.log"
fi
