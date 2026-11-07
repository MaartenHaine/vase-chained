#!/bin/sh
# Spike S2 (throwaway): one L1 run through tools/linux/podman.sh l1, with spikes/S2/l1/run.sh standing in for
# vase-gnome/tests/l1/run.sh (restored from git afterwards, whatever happens). Output goes to
# spikes/S2/results/l1-<time>.txt.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd -P)
target=$repo/vase-gnome/tests/l1/run.sh
out=$repo/spikes/S2/results/l1-$(date +%Y%m%dT%H%M%S).txt

[ -z "$(git -C "$repo" status --porcelain -- "$target")" ] || { echo "run-l1: $target has local changes" >&2; exit 1; }
while [ "$(cut -d' ' -f1 /proc/loadavg | cut -d. -f1)" -ge 40 ]; do
	printf 'run-l1: load %s, waiting\n' "$(cut -d' ' -f1 /proc/loadavg)" >&2
	sleep 30
done

trap 'git -C "$repo" checkout -- "$target"' EXIT
cp "$repo/spikes/S2/l1/run.sh" "$target"
st=0
L1_TIMEOUT=300 nice -n 19 "$repo/tools/linux/podman.sh" l1 >"$out" 2>&1 || st=$?
echo "run-l1: podman.sh l1 exit $st" >>"$out"
echo "run-l1: exit $st, output in $out"
