#!/usr/bin/env bash
# Spike S4 (throwaway): one headless GNOME Shell 50.1 with two virtual monitors, running spikes/S4/s4.js.
# Inside the chained-tiler-s4 container only (spikes/S4/l1.sh starts it). Prints the [s4] lines and the shell's
# warnings to stdout.
#
#   S4_LAYOUT  xml | dbus | none   (see s4.js)
#   S4_MON0    the first virtual monitor (Meta-0), WxH, e.g. 3840x2160 (the 4K at 1.5) or 2560x1440 (scale 1)
#   S4_MON1    the second (Meta-1), default 1920x1080 (the owner's primary)
#   S4_SCALE   Meta-0's scale in monitors.xml (S4_LAYOUT=xml) or through ApplyMonitorsConfig (dbus)
#   S4_CLIENTS for s4.js
set -euo pipefail

if [ ! -e /run/.containerenv ]; then
	echo 's4: refusing to run outside a podman container' >&2
	exit 2
fi

layout=${S4_LAYOUT:-xml}
mon0=${S4_MON0:-3840x2160}
mon1=${S4_MON1:-1920x1080}
scale=${S4_SCALE:-1.5}
export S4_LAYOUT=$layout S4_SCALE=$scale

export TMPDIR=/var/tmp/s4
install -d -m 0700 "$TMPDIR"
home=$(mktemp -d)
export XDG_CONFIG_HOME="$home/config" XDG_DATA_HOME="$home/data" XDG_CACHE_HOME="$home/cache"
export GSETTINGS_BACKEND=keyfile
# A fixed background, as gnome-shell-test-tool sets it. (It doesn't stop the startup stalls; see the watchdog below.)
export SHELL_BACKGROUND_IMAGE=${SHELL_BACKGROUND_IMAGE:-/usr/share/gnome-shell/perf-background.xml}
mkdir -p "$XDG_CONFIG_HOME/glib-2.0/settings" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"

# The owner's mutter settings (wayfinder F11), so the shell parses the same list the desktop has.
cat >"$XDG_CONFIG_HOME/glib-2.0/settings/keyfile" <<EOF
[org/gnome/desktop/interface]
enable-animations=false

[org/gnome/mutter]
experimental-features=['scale-monitor-framebuffer', 'xwayland-native-scaling']
workspaces-only-on-primary=${S4_WOOP:-true}
EOF

w0=${mon0%x*} h0=${mon0#*x} w1=${mon1%x*} h1=${mon1#*x}
# Meta-0's logical width; the primary sits right after it (x=2560 for 3840@1.5 and 2560@1).
x1=$(awk -v w="$w0" -v s="$scale" 'BEGIN { printf "%d", w / s }')
if [ "$layout" = xml ]; then
	# The connector, vendor, product and serial are what mutter gives --virtual-monitor monitors
	# (meta-context-main.c add_virtual_monitor_cb: "MetaVendor", "MetaVirtualMonitor", serial "0x%.2x").
	spec() { # connector serial w h
		printf '<monitor><monitorspec><connector>%s</connector><vendor>MetaVendor</vendor>' "$1"
		printf '<product>MetaVirtualMonitor</product><serial>%s</serial></monitorspec>' "$2"
		printf '<mode><width>%s</width><height>%s</height><rate>60.000</rate></mode></monitor>' "$3" "$4"
	}
	cat >"$XDG_CONFIG_HOME/monitors.xml" <<EOF
<monitors version="2">
  <configuration>
    <layoutmode>logical</layoutmode>
    <logicalmonitor><x>0</x><y>0</y><scale>$scale</scale>$(spec Meta-0 0x00 "$w0" "$h0")</logicalmonitor>
    <logicalmonitor><x>$x1</x><y>0</y><scale>1</scale><primary>yes</primary>$(spec Meta-1 0x01 "$w1" "$h1")</logicalmonitor>
  </configuration>
</monitors>
EOF
	echo "s4: monitors.xml:"
	cat "$XDG_CONFIG_HOME/monitors.xml"
fi

printf 's4: layout=%s mon0=%s mon1=%s scale=%s clients=%s\n' "$layout" "$mon0" "$mon1" "$scale" "${S4_CLIENTS:-default}"
log="$home/shell.log"
export L1_LOG_DIR="$home"
set +e
timeout --kill-after=10 "${S4_TIMEOUT:-600}" \
	dbus-run-session -- python3 -I /src/vase-gnome/tests/l1/dbusrunner.py -- \
	gnome-shell --headless --virtual-monitor "$mon0" --virtual-monitor "$mon1" \
	--wayland-display s4-display --automation-script /src/spikes/S4/s4.js \
	>"$log" 2>&1 &
shell=$!
# Startup watchdog: the script runs at the layout manager's startup-complete (main.js); some runs never get there.
for _ in $(seq 180); do
	grep -q '"what":"env"' "$log" && break
	kill -0 "$shell" 2>/dev/null || break
	sleep 1
done
if ! grep -q '"what":"env"' "$log" && kill -0 "$shell" 2>/dev/null; then
	echo 's4: STARTUP STALLED: no [s4] line 180 s after the start; killing the shell'
	kill "$shell"
fi
wait "$shell"
status=$?
set -e
grep -E '\[s4\]|monitor|Monitor|scale|Scale|experimental|JS ERROR|Script failed|CRITICAL|Xwayland|X11' "$log" | grep -v '^\s*$' || true
if [ -e "$XDG_CONFIG_HOME/monitors.xml" ] && [ "$layout" != xml ]; then
	echo 's4: the shell wrote monitors.xml:'
	cat "$XDG_CONFIG_HOME/monitors.xml"
fi
echo "s4: exit $status"
# The shell exits 1 at the end because scripting.js asks for a PerfHelper that isn't there; that is expected.
if [ -n "${S4_VERBOSE:-}" ] || ! grep -q '"what":"done"' "$log"; then
	echo 's4: the full shell log follows'
	cat "$log"
fi
