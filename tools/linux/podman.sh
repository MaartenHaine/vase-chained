#!/usr/bin/env bash
# The Linux Tiler's container steps. The just recipes (build-linux, test-linux, l1) and the Linux CI workflow both
# call this script, so a local run and a CI run do the same thing.
#
#   tools/linux/podman.sh build        cargo build of the Linux workspace, fetched with the lock, built offline
#   tools/linux/podman.sh test         cargo test of the same packages
#   tools/linux/podman.sh check        clippy (warnings denied) and rustfmt --check on the same packages
#
# Rules (docs/linux-tiler/devtest-security.md §3.5, §4.5): images, containers and volumes are all named chained-tiler-*;
# every container is --rm; only the checkout is mounted, read-only; nothing from the host's /run/user, D-Bus or
# Wayland is ever mounted; nothing is pruned. Builds and runs are niced, because the host is shared.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
HERE="$ROOT/tools/linux"

# The Linux build skips the two platform crates: vase-macos needs AppKit and CoreGraphics, vase-windows needs Win32
# (`podman.sh build` with LINUX_PACKAGES=--workspace shows the failures). New Linux crates are picked up by --workspace.
LINUX_PACKAGES=${LINUX_PACKAGES:---workspace --exclude vase-macos --exclude vase-windows}

die() { printf 'podman.sh: %s\n' "$*" >&2; exit 2; }

# The image for one Containerfile, tagged by the hash of that file, built when missing. Prints the reference.
image() { # build|test
	local file="$HERE/$1.Containerfile" ref
	ref="localhost/chained-tiler-$1:$(sha256sum "$file" | cut -c1-16)"
	if ! podman image exists "$ref"; then
		printf 'podman.sh: building %s\n' "$ref" >&2
		nice -n 19 podman build --quiet -t "$ref" -f "$file" "$HERE" >&2
	fi
	printf '%s\n' "$ref"
}

# One --rm container from an image, with the checkout read-only at /src and no capabilities.
run() { # name image podman-args... -- command...
	local name=$1 ref=$2
	shift 2
	local args=()
	while [ $# -gt 0 ] && [ "$1" != -- ]; do args+=("$1"); shift; done
	[ "${1:-}" = -- ] && shift
	nice -n 19 podman run --rm --name "chained-tiler-$name-$$" \
		--cap-drop=all --security-opt=no-new-privileges \
		-v "$ROOT:/src:ro" "${args[@]}" "$ref" "$@"
}

cargo_volumes=(-v chained-tiler-cargo:/home/ubuntu/cargo:U -v chained-tiler-target:/home/ubuntu/target:U)

# Fetch exactly what Cargo.lock names (online), then run cargo offline with no network at all.
cargo_offline() { # subcommand args...
	local ref sub=$1
	shift
	ref=$(image build)
	run fetch "$ref" "${cargo_volumes[@]}" -- cargo fetch --locked
	# shellcheck disable=SC2086 # LINUX_PACKAGES is a list of flags
	run cargo "$ref" --network=none "${cargo_volumes[@]}" -- cargo "$sub" --locked --offline $LINUX_PACKAGES "$@"
}

case "${1:-}" in
	build) cargo_offline build --all-targets ;;
	test) cargo_offline test ;;
	check)
		cargo_offline clippy --all-targets -- -D warnings
		run fmt "$(image build)" --network=none -- cargo fmt --all --check
		;;
	*) die "usage: podman.sh build | test | check" ;;
esac
