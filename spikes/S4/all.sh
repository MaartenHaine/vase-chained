#!/usr/bin/env bash
# Spike S4 (throwaway): the three measured configurations, one shell at a time. Logs go to OUT (a directory).
#   spikes/S4/all.sh OUT [name...]
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=${1:?usage: all.sh OUT [name...]}
shift
mkdir -p "$out"
declare -A cfg=(
	[dbus15]='dbus 3840x2160 1.5'
	[xml15]='xml 3840x2160 1.5'
	[xml1]='xml 2560x1440 1'
)
names=("$@")
[ ${#names[@]} -gt 0 ] || names=(dbus15 xml15 xml1)
for name in "${names[@]}"; do
	read -r layout mon0 scale <<<"${cfg[$name]}"
	echo "== $name: layout=$layout mon0=$mon0 scale=$scale load $(cut -d' ' -f1-3 /proc/loadavg)"
	set +e
	S4_LAYOUT=$layout S4_MON0=$mon0 S4_SCALE=$scale S4_CLIENTS=${S4_CLIENTS:-gtk4,adw,ghostty,xterm,xterm2x} \
		S4_TIMEOUT=900 bash "$HERE/l1.sh" >"$out/$name.log" 2>&1
	status=$?
	set -e
	echo "exit $status steps $(grep -c '"what":"step"' "$out/$name.log") errors $(grep -c 'client-error\|apply-error' "$out/$name.log")"
done
