#!/usr/bin/env bash
# Spike S3 at L1, inside the chained-tiler-test container only (tools/linux/podman.sh spike S3 starts it).
#
# A headless GNOME Shell 50.1 under gnome-shell-test-tool, on a private session bus (dbus-run-session) and the L1
# harness's mocked system bus, runs spikes/S3/s3.js as its automation script. The script starts the test client
# (spikes/S3/client.js) once as a Wayland client and once through Xwayland. Everything prints to stdout: the shell's
# [s3] lines, then the client's log.
set -euo pipefail

if [ ! -e /run/.containerenv ]; then
	echo 's3: refusing to run outside a podman container: gnome-shell never starts on the host' >&2
	exit 2
fi

export TMPDIR=/var/tmp/s3
install -d -m 0700 "$TMPDIR"
work=$(mktemp -d)
export L1_LOG_DIR="$work"
export S3_CLIENT_LOG="$work/client.jsonl"
export S3_CLIENT_JS=/src/spikes/S3/client.js
export S3_X11_CLIENT=/src/spikes/S3/x11client.py
export S3_SCHEMA_DIR="$work/schemas"
: >"$S3_CLIENT_LOG"
mkdir "$S3_SCHEMA_DIR"
glib-compile-schemas --strict --targetdir="$S3_SCHEMA_DIR" /src/spikes/S3/schemas

set +e
timeout --kill-after=10 "${L1_TIMEOUT:-300}" \
	dbus-run-session -- python3 -I /src/vase-gnome/tests/l1/dbusrunner.py -- \
	gnome-shell-test-tool --headless --disable-animations --extra-filter S3client \
	/src/spikes/S3/s3.js >"$work/shell.log" 2>&1
status=$?
set -e

echo "s3: shell exit $status"
echo '--- shell log, [s3] lines and errors'
grep -E '\[s3\]|JS ERROR|Script failed|CRITICAL|WARNING|Xwayland|dbusrunner:' "$work/shell.log" || true
echo '--- client log'
cat "$S3_CLIENT_LOG"
if [ -n "${L1_VERBOSE:-}" ] || [ "$status" -ne 0 ]; then
	echo '--- full shell log'
	cat "$work/shell.log"
fi
