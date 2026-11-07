#!/usr/bin/env bash
# The L1 hello test, inside the chained-tiler-test container (tools/linux/podman.sh l1 starts it).
#
# It packs vase-gnome/extension, then runs gnome-shell-test-tool --headless --extension on it under dbus-run-session
# (a private session bus) and dbusrunner.py (a mocked private system bus), with TMPDIR inside the container. It passes
# when the shell exits 0, hello.js printed PASS, and the shell logged exactly three enables and two disables: the one
# at startup, then the disable, enable, disable, enable of the cycle.
set -euo pipefail

if [ ! -e /run/.containerenv ]; then
	echo 'l1: refusing to run outside a podman container: gnome-shell never starts on the host' >&2
	exit 2
fi

UUID=vase-hello@maartenhaine.github.io
export TMPDIR=/var/tmp/l1
install -d -m 0700 "$TMPDIR"
work=$(mktemp -d)
export L1_LOG_DIR="$work"

cp -r /src/vase-gnome/extension "$work/hello"
gnome-extensions pack "$work/hello" --out-dir "$work" --force
zip="$work/$UUID.shell-extension.zip"

start=$(date +%s.%N)
set +e
timeout --kill-after=10 "${L1_TIMEOUT:-120}" \
	dbus-run-session -- python3 -I /src/vase-gnome/tests/l1/dbusrunner.py -- \
	gnome-shell-test-tool --headless --disable-animations --extension "$zip" /src/vase-gnome/tests/l1/hello.js \
	>"$work/shell.log" 2>&1
status=$?
set -e
end=$(date +%s.%N)

grep -E '\[l1\]|\[vase-hello\]|vase-hello|JS ERROR|Script failed|CRITICAL|dbusrunner:' "$work/shell.log" || true
enables=$(grep -c '\[vase-hello\] enable' "$work/shell.log" || true)
disables=$(grep -c '\[vase-hello\] disable' "$work/shell.log" || true)
passed=no
grep -q '\[l1\] PASS' "$work/shell.log" && passed=yes

printf 'l1: exit %s, PASS line %s, enables %s, disables %s, %s s\n' \
	"$status" "$passed" "$enables" "$disables" "$(awk -v a="$start" -v b="$end" 'BEGIN { printf "%.1f", b - a }')"
if [ "$status" -ne 0 ] || [ "$passed" != yes ] || [ "$enables" -ne 3 ] || [ "$disables" -ne 2 ]; then
	echo 'l1: FAIL; the full shell log follows' >&2
	cat "$work/shell.log" >&2
	exit 1
fi
if [ -n "${L1_VERBOSE:-}" ]; then
	echo 'l1: the full shell log follows'
	cat "$work/shell.log"
fi
echo 'l1: PASS'
