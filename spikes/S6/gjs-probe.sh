#!/usr/bin/env bash
# Spike S6 (throwaway): which Meta.WindowConfig calls work from GJS 1.88 with mutter 50.1's typelib, in plain gjs
# (no shell, no display), inside chained-tiler-test only:
#   podman run --rm --network=none --cap-drop=all -v "$PWD:/src:ro" localhost/chained-tiler-test:<tag> \
#       bash /src/spikes/S6/gjs-probe.sh
# Each case runs in its own gjs process, so a segfault ends only that case.
set -u
if [ ! -e /run/.containerenv ]; then
	echo 'gjs-probe: refusing to run outside a podman container' >&2
	exit 2
fi
H='import GIRepository from "gi://GIRepository?version=3.0";
GIRepository.Repository.dup_default().prepend_search_path("/usr/lib/x86_64-linux-gnu/mutter-18");
const Meta = (await import("gi://Meta?version=18")).default;
const Mtk = (await import("gi://Mtk?version=18")).default;
const c = new Meta.WindowConfig();
const show = r => r === null ? "null" : [r.x, r.y, r.width, r.height].join(",");'
dir=$(mktemp -d)
t() { # name body
	printf '%s\n%s\n' "$H" "$2" >"$dir/t.js"
	gjs -m "$dir/t.js" >"$dir/out" 2>&1
	s=$?
	out=$(tr '\n' ' ' <"$dir/out" | cut -c1-200)
	printf '%-26s exit %-4s %s\n' "$1" "$s" "$out"
}
for i in 1 2 3; do
	echo "round $i"
	t set_position 'c.set_position(120, 102); print("ok");'
	t set_size 'c.set_size(600, 400); print("ok");'
	t get_rect-fresh 'print(show(c.get_rect()));'
	t get_rect-after-size 'c.set_size(600, 400); print(show(c.get_rect()));'
	t get_rect-after-position 'c.set_position(120, 102); print(show(c.get_rect()));'
	t set_rect-then-get_rect 'c.set_rect(new Mtk.Rectangle({x: 1, y: 2, width: 3, height: 4})); print(show(c.get_rect()));'
	t get_position 'try { print(c.get_position()); } catch (e) { print("throws:", e.message); }'
	t get_size 'try { print(c.get_size()); } catch (e) { print("throws:", e.message); }'
	t fullscreen 'c.set_is_fullscreen(true); print(c.get_is_fullscreen(), c.get_is_initial());'
done
rm -rf "$dir"
