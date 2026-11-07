#!/usr/bin/env bash
# Spike S6 (throwaway) at L1: inside chained-tiler-test only (tools/linux/podman.sh spike S6 starts it).
#
# Runs spikes/S6/s6.js in a headless GNOME Shell 50.1 through gnome-shell-test-tool, under dbus-run-session and the L1
# mocked system bus, with TMPDIR inside the container. Prints the [s6] event lines and mutter's own geometry lines,
# then the full log if S6_VERBOSE is set.
set -euo pipefail

if [ ! -e /run/.containerenv ]; then
	echo 's6: refusing to run outside a podman container: gnome-shell never starts on the host' >&2
	exit 2
fi

export TMPDIR=/var/tmp/s6
install -d -m 0700 "$TMPDIR"
work=$(mktemp -d)
export L1_LOG_DIR="$work"
# mutter's own topic logs: "pre-configured at" (window-x11.c:4702), placement and constraints.
export MUTTER_DEBUG="${S6_MUTTER_DEBUG:-geometry,placement}"
# A crash prints its backtrace to the log (the container has no apport).
export SHELL_DEBUG=backtrace-segfaults

# The test tool runs the script only once gnome-shell-perf-helper owns its bus name (scripting.js:363-379); under
# load that sometimes never happens and the shell idles until the timeout. So each attempt must log the script's
# first line within S6_START_TIMEOUT seconds, or it is killed and retried.
status=1
for attempt in 1 2 3 4; do
	: >"$work/shell.log"
	timeout --kill-after=10 "${S6_TIMEOUT:-900}" \
		dbus-run-session -- python3 -I /src/vase-gnome/tests/l1/dbusrunner.py -- \
		gnome-shell-test-tool --headless --disable-animations \
		--extra-filter XLogo --extra-filter Gtk4-demo --extra-filter gtk4-demo --extra-filter Org.gtk.Demo4 \
		--extra-filter org.gtk.Demo4 \
		/src/spikes/S6/s6.js >"$work/shell.log" 2>&1 &
	pid=$!
	started=no
	for _ in $(seq "${S6_START_TIMEOUT:-150}"); do
		if grep -q '\[s6\] {"ev":"run"' "$work/shell.log"; then started=yes; break; fi
		kill -0 "$pid" 2>/dev/null || break
		sleep 1
	done
	if [ "$started" = no ]; then
		echo "s6: attempt $attempt: the script didn't start; killing and retrying" >&2
		pkill -KILL -P "$pid" 2>/dev/null || true
		kill -KILL "$pid" 2>/dev/null || true
		pkill -KILL -f gnome-shell 2>/dev/null || true
		pkill -KILL -f dbus-daemon 2>/dev/null || true
		wait "$pid" 2>/dev/null || true
		continue
	fi
	set +e
	wait "$pid"
	status=$?
	set -e
	echo "s6: attempt $attempt ran, shell exit $status" >&2
	break
done

grep -E '\[s6\]|JS ERROR|Script failed|CRITICAL' "$work/shell.log" | sed -E 's/^.*\[s6\] /[s6] /' || true
echo '--- mutter geometry lines (pre-configured / placement) ---'
grep -E 'pre-configured|Placing window|placed at|Constraining|constrain' "$work/shell.log" | head -400 || true
if [ -n "${S6_VERBOSE:-}" ]; then
	echo '--- full log ---'
	cat "$work/shell.log"
fi
echo "s6: shell exit $status"
