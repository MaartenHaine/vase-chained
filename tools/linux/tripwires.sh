#!/usr/bin/env bash
# The tripwires (tools/linux/tripwires.txt) over the extension's JavaScript.
#
#   tools/linux/tripwires.sh [DIR]      fail on any match in DIR (default vase-gnome/extension) not in tripwires.allow
#   tools/linux/tripwires.sh --self-test   check that every wire matches its sample line and that the allow-list works
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
WIRES="$ROOT/tools/linux/tripwires.txt"
ALLOW="$ROOT/tools/linux/tripwires.allow"

# Every match in a directory, one per line: path, wire id, trimmed line, line number (tab-separated).
scan() { # dir
	local dir=$1 id regex _sample hit file rest lineno text
	while IFS=$'\t' read -r id regex _sample; do
		[ -z "$id" ] || [ "${id:0:1}" = '#' ] && continue
		while IFS= read -r hit; do
			file=${hit%%:*}
			rest=${hit#*:}
			lineno=${rest%%:*}
			text=${rest#*:}
			text=$(sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' <<<"$text")
			printf '%s\t%s\t%s\t%s\n' "${file#"$ROOT"/}" "$id" "$text" "$lineno"
		done < <(grep -rnP --include='*.js' -- "$regex" "$dir" || true)
	done <"$WIRES"
}

# The values go to awk through the environment: awk -v would turn backslash escapes in the line into other characters.
allowed() { # path id text [allow-file]
	P="$1" I="$2" T="$3" awk -F'\t' '$1 == ENVIRON["P"] && $2 == ENVIRON["I"] && $3 == ENVIRON["T"] { found = 1 }
		END { exit !found }' "${4:-$ALLOW}"
}

check() { # dir
	local dir=$1 path id text lineno bad=0 hits=0
	while IFS=$'\t' read -r path id text lineno; do
		hits=$((hits + 1))
		if allowed "$path" "$id" "$text"; then
			printf 'tripwires: allowed: %s:%s [%s]\n' "$path" "$lineno" "$id"
		else
			printf 'tripwires: %s:%s [%s] %s\n' "$path" "$lineno" "$id" "$text" >&2
			bad=$((bad + 1))
		fi
	done < <(scan "$dir")
	if [ "$bad" -gt 0 ]; then
		printf 'tripwires: %s unreviewed match(es); review them, then list them in %s\n' "$bad" "${ALLOW#"$ROOT"/}" >&2
		return 1
	fi
	printf 'tripwires: %s clean (%s allowed match(es))\n' "${dir#"$ROOT"/}" "$hits"
}

self_test() {
	local tmp id regex sample failed=0 count=0
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' RETURN
	while IFS=$'\t' read -r id regex sample; do
		[ -z "$id" ] || [ "${id:0:1}" = '#' ] && continue
		count=$((count + 1))
		printf '%s\n' "$sample" >"$tmp/$id.js"
	done <"$WIRES"
	local found
	found=$(scan "$tmp")
	while IFS=$'\t' read -r id regex sample; do
		[ -z "$id" ] || [ "${id:0:1}" = '#' ] && continue
		if awk -F'\t' -v i="$id" -v f="${tmp#"$ROOT"/}/$id.js" '$2 == i && $1 == f { ok = 1 } END { exit !ok }' <<<"$found"; then
			printf 'tripwires self-test: ok: %s matches its sample\n' "$id"
		else
			printf 'tripwires self-test: FAIL: %s does not match its sample\n' "$id" >&2
			failed=1
		fi
	done <"$WIRES"
	# An allow entry matches its line exactly, backslashes included, and no other line.
	printf '%s\t%s\t%s\t%s\n' a.js proc-spawn 'x(/\s+/, "a\\b")' 'self-test' >"$tmp/allow"
	if allowed a.js proc-spawn 'x(/\s+/, "a\\b")' "$tmp/allow" && ! allowed a.js proc-spawn 'x(/\s+/, "a\b")' "$tmp/allow" \
		&& ! allowed a.js proc-spawn 'x(/s+/, "a\\b")' "$tmp/allow"; then
		printf 'tripwires self-test: ok: the allow-list compares lines with backslashes exactly\n'
	else
		printf 'tripwires self-test: FAIL: the allow-list does not compare lines with backslashes exactly\n' >&2
		failed=1
	fi
	if check "$tmp" >/dev/null 2>&1; then
		printf 'tripwires self-test: FAIL: a directory full of samples passed\n' >&2
		failed=1
	else
		printf 'tripwires self-test: ok: %s samples fail the check\n' "$count"
	fi
	return "$failed"
}

case "${1:-}" in
	--self-test) self_test ;;
	*) check "${1:-$ROOT/vase-gnome/extension}" ;;
esac
