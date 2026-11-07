#!/usr/bin/env bash
# Spike S4 (throwaway): builds chained-tiler-s4 on top of the current chained-tiler-test image and runs
# spikes/S4/run.sh in it, under the same rules as tools/linux/podman.sh: --rm, no capabilities, no network, only the
# checkout mounted (read-only), niced, nothing pruned. Waits while the host's 1-minute load is over 40.
#
#   S4_LAYOUT=dbus S4_MON0=3840x2160 S4_SCALE=1.5 spikes/S4/l1.sh
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
HERE="$ROOT/spikes/S4"

base="localhost/chained-tiler-test:$(sha256sum "$ROOT/tools/linux/test.Containerfile" | cut -c1-16)"
podman image exists "$base" || { echo "s4: $base missing; run tools/linux/podman.sh l1 once" >&2; exit 2; }
ref="localhost/chained-tiler-s4:$(cat "$HERE/Containerfile" <(printf %s "$base") | sha256sum | cut -c1-16)"
if ! podman image exists "$ref"; then
	nice -n 19 podman build --quiet --build-arg BASE="$base" -t "$ref" -f "$HERE/Containerfile" "$HERE" >&2
fi

while awk '{ exit !($1 > 40) }' /proc/loadavg; do
	echo "s4: load $(cut -d' ' -f1 /proc/loadavg) > 40, waiting" >&2
	sleep 30
done

exec nice -n 19 podman run --rm --name "chained-tiler-s4-$$" \
	--cap-drop=all --security-opt=no-new-privileges --network=none --ulimit core=1:1 \
	-v "$ROOT:/src:ro" \
	-e S4_LAYOUT -e S4_MON0 -e S4_MON1 -e S4_SCALE -e S4_CLIENTS -e S4_TIMEOUT -e S4_VERBOSE -e S4_DELAYS -e S4_QUIET_MS -e S4_WAYLAND_DEBUG -e S4_WOOP \
	"$ref" bash /src/spikes/S4/run.sh
