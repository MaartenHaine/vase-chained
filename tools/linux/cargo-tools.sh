#!/usr/bin/env bash
# Puts cargo-deny and cargo-audit in DIR, from their release tarballs, each checked against the sha256 pinned below.
# Neither is installed from crates.io at run time; moving a version means moving its hash here, in review.
#
#   tools/linux/cargo-tools.sh DIR
#
# The binaries are static musl builds. They run only in a container (podman.sh supply) or on a CI runner, never on
# the desktop itself.
set -euo pipefail

CARGO_DENY_VERSION=0.20.2
CARGO_DENY_SHA256=9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f
CARGO_AUDIT_VERSION=0.22.2
CARGO_AUDIT_SHA256=7fb9497f8594b389e5fce5ef9b92db08432996895b2e0c5a0167a69ed445c428

dir=${1:?usage: cargo-tools.sh DIR}
mkdir -p "$dir"

fetch() { # url sha256 member name
	local url=$1 sha=$2 member=$3 name=$4 tarball
	tarball="$dir/$(basename "$url")"
	if ! { [ -f "$tarball" ] && printf '%s  %s\n' "$sha" "$tarball" | sha256sum -c --quiet 2>/dev/null; }; then
		curl -fsSL --proto '=https' --tlsv1.2 -o "$tarball" "$url"
		printf '%s  %s\n' "$sha" "$tarball" | sha256sum -c --quiet
	fi
	tar -xzf "$tarball" -C "$dir" --strip-components=1 "$member"
	chmod 0755 "$dir/$name"
	printf 'cargo-tools: %s from %s (sha256 %s)\n' "$name" "$(basename "$url")" "$sha"
}

fetch "https://github.com/EmbarkStudios/cargo-deny/releases/download/$CARGO_DENY_VERSION/cargo-deny-$CARGO_DENY_VERSION-x86_64-unknown-linux-musl.tar.gz" \
	"$CARGO_DENY_SHA256" "cargo-deny-$CARGO_DENY_VERSION-x86_64-unknown-linux-musl/cargo-deny" cargo-deny
fetch "https://github.com/rustsec/rustsec/releases/download/cargo-audit/v$CARGO_AUDIT_VERSION/cargo-audit-x86_64-unknown-linux-musl-v$CARGO_AUDIT_VERSION.tgz" \
	"$CARGO_AUDIT_SHA256" "cargo-audit-x86_64-unknown-linux-musl-v$CARGO_AUDIT_VERSION/cargo-audit" cargo-audit
