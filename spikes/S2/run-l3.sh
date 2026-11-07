#!/bin/sh
# Spike S2 (throwaway): one L3 run of the S2 extension, between before-after snapshots of the real desktop.
#
#   sh spikes/S2/run-l3.sh
#
# tools/dev-shell.sh loads only vase-gnome/extension of its own checkout, so for the run that directory's contents are
# replaced by spikes/S2/extension (plus its compiled schema) and restored from git afterwards, whatever happens. The
# dev shell itself is unchanged: --x11 (xev runs through Xwayland), niced, and stopped with SIGTERM after 8 minutes if
# the extension hasn't ended it. Results are copied to spikes/S2/results/l3-<time>/.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd -P)
spike=$repo/spikes/S2
ext=$repo/vase-gnome/extension
snaps=$HOME/.cache/tiler-dev-snapshots
ts=$(date +%Y%m%dT%H%M%S)
out=$spike/results/l3-$ts

die() { printf 'run-l3: %s\n' "$*" >&2; exit 1; }

[ -z "$(git -C "$repo" status --porcelain -- vase-gnome/extension)" ] || die "vase-gnome/extension has local changes"
pgrep -u "$(id -u)" -x mutter-devkit >/dev/null && die "a devkit is already running (one L3 at a time)"

while [ "$(cut -d' ' -f1 /proc/loadavg | cut -d. -f1)" -ge 40 ]; do
	printf 'run-l3: load %s, waiting\n' "$(cut -d' ' -f1 /proc/loadavg)" >&2
	sleep 30
done

mkdir -p "$snaps" "$out"
chmod 700 "$snaps"
sh "$repo/tools/before-after.sh" snapshot "$snaps/s2-$ts-before" >"$out/before.txt"

restore() {
	git -C "$repo" checkout -- vase-gnome/extension
	git -C "$repo" clean -fdq -- vase-gnome/extension
}
trap restore EXIT
find "$ext" -mindepth 1 -delete
cp -r "$spike/extension/." "$ext/"
glib-compile-schemas "$ext/schemas"

st=0
timeout -s TERM 480 nice -n 19 sh "$repo/tools/dev-shell.sh" --x11 >"$out/dev-shell.out" 2>&1 || st=$?
restore
trap - EXIT
echo "dev-shell exit $st" >>"$out/dev-shell.out"

sleep 3
sh "$repo/tools/before-after.sh" snapshot "$snaps/s2-$ts-after" >"$out/after.txt"
cst=0
sh "$repo/tools/before-after.sh" compare "$snaps/s2-$ts-before" "$snaps/s2-$ts-after" >"$out/compare.txt" 2>&1 || cst=$?
echo "compare exit $cst" >>"$out/compare.txt"

run=$(sed -n 's/^dev-shell: run dir //p' "$out/dev-shell.out" | head -1)
if [ -n "$run" ] && [ -d "$run" ]; then
	echo "$run" >"$out/run-dir.txt"
	cp "$run/log/shell.log" "$out/" 2>/dev/null || true
	[ -d "$run/home/s2" ] && cp -r "$run/home/s2/." "$out/"
fi
printf 'run-l3: dev-shell exit %s, compare exit %s, results in %s\n' "$st" "$cst" "$out"
