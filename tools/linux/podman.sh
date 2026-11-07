#!/usr/bin/env bash
# The Linux Tiler's container steps. The just recipes (build-linux, test-linux, l1) and the Linux CI workflow both
# call this script, so a local run and a CI run do the same thing.
#
#   tools/linux/podman.sh build        cargo build of the Linux workspace, fetched with the lock, built offline
#   tools/linux/podman.sh test         cargo test of the same packages
#   tools/linux/podman.sh check        clippy (warnings denied) and rustfmt --check on the same packages
#   tools/linux/podman.sh supply BIN   cargo-deny and cargo-audit, with the binaries tools/linux/cargo-tools.sh put in BIN
#   tools/linux/podman.sh lock         Cargo.lock after a manifest change, printed to stdout (the checkout stays read-only)
#   tools/linux/podman.sh lint NODE    tools/linux/lint.sh in the build image, with a Node.js install mounted from NODE
#   tools/linux/podman.sh l1           the L1 hello test: headless GNOME Shell in chained-tiler-test
#   tools/linux/podman.sh spike S#     a throwaway M0b spike at L1: spikes/S#/l1.sh, or else spikes/S#/run.sh, like l1
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

# CARGO_HOME is one volume for every checkout: it holds only what Cargo.lock names, checked by checksum when it is
# downloaded. Only steps that run no crate code (cargo fetch, cargo-deny, cargo-audit) may write it; the offline step,
# which runs build scripts, proc macros and test binaries, gets it read-only, so nothing it does outlives the run.
# The target dir is one volume per checkout: every checkout is mounted at /src, so with one shared target dir cargo's
# mtime check could call another checkout's build fresh and run its test binaries.
target_volume="chained-tiler-target-$(printf %s "$ROOT" | sha256sum | cut -c1-12)"
cargo_volumes=(-v chained-tiler-cargo:/home/ubuntu/cargo:U -v "$target_volume:/home/ubuntu/target:U")
offline_volumes=(-v chained-tiler-cargo:/home/ubuntu/cargo:ro -v "$target_volume:/home/ubuntu/target:U")

# cargo fetch --locked, online. It refuses to run when the cargo volume holds a config or a bin/: nothing here writes
# one, and a planted config (a rustc-wrapper, say) would run code in this container, which has the network.
fetch() { # image
	# shellcheck disable=SC2016 # the script expands $CARGO_HOME in the container
	run fetch "$1" "${cargo_volumes[@]}" -- bash -c '
		for f in config.toml config bin; do
			if [ -e "$CARGO_HOME/$f" ]; then
				echo "podman.sh: the chained-tiler-cargo volume holds $f, which nothing here writes; not fetching" >&2
				exit 2
			fi
		done
		exec cargo fetch --locked'
}

# Fetch exactly what Cargo.lock names (online), then run cargo offline with no network at all.
cargo_offline() { # subcommand args...
	local ref sub=$1
	shift
	ref=$(image build)
	fetch "$ref"
	# shellcheck disable=SC2086 # LINUX_PACKAGES is a list of flags
	run cargo "$ref" --network=none "${offline_volumes[@]}" -- cargo "$sub" --locked --offline $LINUX_PACKAGES "$@"
}

case "${1:-}" in
	build) cargo_offline build --all-targets ;;
	test) cargo_offline test ;;
	check)
		cargo_offline clippy --all-targets -- -D warnings
		run fmt "$(image build)" --network=none -- cargo fmt --all --check
		;;
	supply)
		bin=${2:?usage: podman.sh supply BIN_DIR}
		[ -x "$bin/cargo-deny" ] && [ -x "$bin/cargo-audit" ] || die "$bin must hold cargo-deny and cargo-audit"
		ref=$(image build)
		# Both read the advisory database from GitHub, so this step is online. cargo-deny reads the whole
		# workspace's metadata (every platform), so it is fetched first.
		fetch "$ref"
		run supply "$ref" "${cargo_volumes[@]}" -v "$(cd "$bin" && pwd):/tools:ro" -- \
			bash -c '/tools/cargo-deny --locked check && /tools/cargo-audit audit --deny warnings'
		;;
	lock)
		# A new dependency needs a Cargo.lock entry before the locked steps above can run. The resolution runs online
		# in the container, in a copy of the checkout (the mount stays read-only), and only adds what the manifests
		# now need: cargo metadata keeps every version already locked. Like fetch, it runs no crate code, so it may
		# write the cargo volume. Review the diff before replacing the lockfile:
		#   tools/linux/podman.sh lock > Cargo.lock.new && git diff --no-index Cargo.lock Cargo.lock.new
		# shellcheck disable=SC2016 # the script expands $CARGO_HOME in the container
		run lock "$(image build)" "${cargo_volumes[@]}" -- bash -c '
			for f in config.toml config bin; do
				if [ -e "$CARGO_HOME/$f" ]; then
					echo "podman.sh: the chained-tiler-cargo volume holds $f, which nothing here writes; not resolving" >&2
					exit 2
				fi
			done
			mkdir /tmp/ws
			tar -C /src --exclude=./.git --exclude=./target -cf - . | tar -C /tmp/ws -xf -
			cd /tmp/ws && cargo metadata --format-version 1 >/dev/null && cat Cargo.lock'
		;;
	lint)
		node=${2:?usage: podman.sh lint NODE_DIR (a Node.js install with bin/node and bin/npm)}
		[ -x "$node/bin/node" ] || die "$node/bin/node not found"
		run lint "$(image build)" -v "$(cd "$node" && pwd):/opt/node:ro" -e PATH=/opt/node/bin:/usr/bin:/bin -- \
			bash /src/tools/linux/lint.sh
		;;
	l1)
		# The shell gets no network: everything it talks to is a bus inside the container.
		run l1 "$(image test)" --network=none -e L1_VERBOSE="${L1_VERBOSE:-}" -e L1_TIMEOUT="${L1_TIMEOUT:-120}" -- \
			bash /src/vase-gnome/tests/l1/run.sh
		;;
	spike)
		# A throwaway M0b spike at L1 (wayfinder.md §5) in chained-tiler-test, under the same rules as l1. S3 named its
		# entry point l1.sh and S6 named it run.sh; l1.sh wins when a spike has both.
		s=${2:?usage: podman.sh spike S#}
		[[ $s =~ ^S[0-9]+$ ]] || die "a spike is named S<number>, not $s"
		entry=
		for f in l1.sh run.sh; do [ -f "$ROOT/spikes/$s/$f" ] && { entry=$f; break; }; done
		[ -n "$entry" ] || die "spikes/$s/l1.sh or spikes/$s/run.sh not found"
		envs=(-e L1_VERBOSE="${L1_VERBOSE:-}" -e L1_TIMEOUT="${L1_TIMEOUT:-300}")
		while IFS='=' read -r k _; do envs+=(-e "$k"); done < <(env | grep -E '^S[0-9]+_[A-Z_]+=' || true)
		run "spike-${s,,}" "$(image test)" --network=none "${envs[@]}" -- bash "/src/spikes/$s/$entry"
		;;
	*) die "usage: podman.sh build | test | check | supply BIN_DIR | lock | lint NODE_DIR | l1 | spike S#" ;;
esac
