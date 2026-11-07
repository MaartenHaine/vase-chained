#!/usr/bin/env bash
# Spike S9 (docs/linux-spike/S9.md): build the stub daemon in chained-tiler-build, then run the L1 case in
# chained-tiler-test with it. Throwaway.
#
#   spikes/S9/run.sh build     cargo build --release of spikes/S9/stub, offline, into the chained-tiler-s9-target volume
#   spikes/S9/run.sh l1        the L1 case: headless GNOME Shell, the S9 extension, the stub daemon
#
# Same rules as tools/linux/podman.sh: chained-tiler-* names only, --rm, the checkout read-only, no network, no host
# /run/user, D-Bus or Wayland, nothing pruned, niced, and nothing starts while the 1-minute load is over 40.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
VOLUME=chained-tiler-s9-target

image() { # build|test: the image tools/linux/podman.sh would use; it must already exist (this script builds none)
	local ref
	ref="localhost/chained-tiler-$1:$(sha256sum "$ROOT/tools/linux/$1.Containerfile" | cut -c1-16)"
	podman image exists "$ref" || { echo "run.sh: $ref is missing; build it with tools/linux/podman.sh" >&2; exit 2; }
	printf '%s\n' "$ref"
}

wait_load() {
	local l
	while :; do
		l=$(cut -d' ' -f1 /proc/loadavg)
		[ "${l%.*}" -le 40 ] && break
		echo "run.sh: 1-minute load $l > 40, waiting 60 s" >&2
		sleep 60
	done
	echo "run.sh: load $(cut -d' ' -f1-3 /proc/loadavg)" >&2
}

case "${1:-}" in
	build)
		wait_load
		nice -n 19 podman run --rm --name "chained-tiler-s9-build-$$" --cap-drop=all \
			--security-opt=no-new-privileges --network=none \
			-v "$ROOT:/src:ro" -v "$VOLUME:/home/ubuntu/target:U" "$(image build)" \
			cargo build --release --locked --offline --manifest-path /src/spikes/S9/stub/Cargo.toml
		;;
	l1)
		wait_load
		nice -n 19 podman run --rm --name "chained-tiler-s9-l1-$$" --cap-drop=all \
			--security-opt=no-new-privileges --network=none \
			-v "$ROOT:/src:ro" -v "$VOLUME:/opt/s9:ro" -e S9_CASES="${S9_CASES:-}" "$(image test)" \
			bash /src/spikes/S9/l1/run.sh
		;;
	*) echo 'usage: spikes/S9/run.sh build | l1' >&2; exit 2 ;;
esac
