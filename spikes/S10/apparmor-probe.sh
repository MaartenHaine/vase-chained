#!/bin/sh
# S10, the AppArmor half (throwaway). No sudo inside: the owner loads the test profile and installs the two test
# binaries first (docs/linux-spike/S10.md, "Sudo steps for the AppArmor half"), then runs this, then removes both.
#
#   sh spikes/S10/apparmor-probe.sh              probe: does vase-s10-probe attach to a binary systemd --user starts?
#   sh spikes/S10/apparmor-probe.sh --check-gone after removal: the profile is unloaded and the binaries are gone
#   sh spikes/S10/apparmor-probe.sh --dry        the same units with /usr/bin/gnucat and gnuenv (no profile
#                                                needed): checks the script itself; every label is "unconfined"
#
# Transient units only (--collect), LimitCORE=1, results under results/aa-<time>/. The one file it writes outside
# results/ is a 0700 ~/.cache/s10-probe.* dir with a fifo and a "secret" file, removed at exit.
set -eu

here=$(cd "$(dirname "$0")" && pwd -P)
dir=/usr/local/libexec/vase-s10
uid=$(id -u)
[ "$uid" -ne 0 ] || { echo "run as the desktop user, not with sudo" >&2; exit 2; }

loaded() { # prints "name mode" of every loaded profile called vase-s10-probe
	for p in /sys/kernel/security/apparmor/policy/profiles/*; do
		[ "$(cat "$p/name" 2>/dev/null)" = vase-s10-probe ] && echo "vase-s10-probe $(cat "$p/mode")"
	done
	return 0
}

if [ "${1:-}" = --check-gone ]; then
	l=$(loaded)
	echo "profile loaded: ${l:-no}"
	if [ -e "$dir" ]; then echo "$dir: present"; else echo "$dir: absent"; fi
	echo "units left: $(systemctl --user list-units --all --no-legend --plain 's10-*' | wc -l)"
	[ -z "$l" ] && [ ! -e "$dir" ]
	exit
fi

cat_bin=$dir/probe-cat
env_bin=$dir/probe-env
tag=aa
if [ "${1:-}" = --dry ]; then
	cat_bin=/usr/bin/gnucat
	env_bin=/usr/bin/gnuenv
	tag=aa-dry
	l="(dry run, not checked)"
else
	l=$(loaded)
	[ -n "$l" ] || { echo "vase-s10-probe isn't loaded; run the sudo steps first" >&2; exit 2; }
	for b in "$cat_bin" "$env_bin"; do
		[ "$(stat -c '%U:%G %a' "$b" 2>/dev/null)" = "root:root 755" ] || { echo "$b isn't root:root 0755" >&2; exit 2; }
	done
fi

run=$here/results/$tag-$(date +%Y%m%dT%H%M%S)
mkdir -p "$run"
probe=$(mktemp -d "$HOME/.cache/s10-probe.XXXXXXXX")
chmod 0700 "$probe"
trap 'rm -rf "$probe"' EXIT
echo "s10 secret" >"$probe/secret"
since=$(date '+%Y-%m-%d %H:%M:%S')

plan='NoNewPrivileges=yes;RestrictAddressFamilies=AF_UNIX;SystemCallFilter=@system-service;SystemCallFilter=~@privileged @resources;SystemCallArchitectures=native;SystemCallErrorNumber=EPERM;LockPersonality=yes;MemoryDenyWriteExecute=yes;RestrictNamespaces=yes;RestrictRealtime=yes;RestrictSUIDSGID=yes'

one() { # name props command...
	name=$1
	props=$2
	shift 2
	cmd="$*"
	set -- --user --unit="s10-aa-$name-$$.service" --wait --pipe --collect \
		-p Nice=19 -p IOSchedulingClass=idle -p RuntimeMaxSec=60 -p LimitCORE=1
	oldifs=$IFS
	IFS=';'
	for p in $props; do
		IFS=$oldifs
		[ -n "$p" ] && set -- "$@" -p "$p"
	done
	IFS=$oldifs
	rc=0
	# shellcheck disable=SC2086 # cmd is a list of plain words
	nice -n 19 systemd-run "$@" $cmd </dev/null >"$run/$name.out" 2>"$run/$name.err" || rc=$?
	{ printf 'props=%s\ncmd=%s\nrc=%s\n' "$props" "$cmd" "$rc"; } >"$run/$name.meta"
	sleep 0.5
	journalctl --user --no-pager -o cat --since "$since" -u "s10-aa-$name-$$.service" >"$run/$name.journal" 2>&1 || true
	printf '%-28s rc=%-3s label=%s\n' "$name" "$rc" "$(head -n1 "$run/$name.out")"
}

{
	echo "profile: $l"
	ls -l "$dir" 2>&1 || true
	echo "kernel=$(uname -r) systemd=$(systemctl --user show -p Version --value)"
} >"$run/host.txt"

files="/proc/self/attr/current /usr/lib/os-release $probe/secret"
{
	one a0-control-cat '' /usr/bin/cat $files
	one a1-plain '' $cat_bin $files
	one a2-plan "$plan" $cat_bin $files
	one a3-plan-plus-userns "$plan;ProtectHome=read-only" $cat_bin $files
	one a4-control-env-exec '' /usr/bin/env /usr/bin/true
	one a5-env-exec '' $env_bin /usr/bin/true
	one a6-plan-env-exec "$plan" $env_bin /usr/bin/true
	# By name instead of by path: systemd's AppArmorProfile= (aa_change_onexec in the executor) on the stock binary.
	one a8-by-name AppArmorProfile=vase-s10-probe /usr/bin/gnucat $files
	one a9-by-name-plan "$plan;AppArmorProfile=vase-s10-probe" /usr/bin/gnucat $files
} | tee "$run/summary.txt"

# a7: doctor's view, from outside, of a running confined process (MainPID's attr/current and status).
mkfifo "$probe/fifo"
u="s10-aa-a7-live-$$.service"
oldifs=$IFS
IFS=';'
set --
for p in $plan; do set -- "$@" -p "$p"; done
IFS=$oldifs
nice -n 19 systemd-run --user --unit="$u" --collect -p Nice=19 -p RuntimeMaxSec=60 -p LimitCORE=1 "$@" \
	"$cat_bin" "$probe/fifo" >"$run/a7-live.err" 2>&1
pid=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
	pid=$(systemctl --user show -p MainPID --value "$u" 2>/dev/null || echo 0)
	[ "${pid:-0}" -gt 0 ] && [ "$(readlink "/proc/$pid/exe" 2>/dev/null)" = "$cat_bin" ] && break
	sleep 0.5
done
{
	echo "MainPID=$pid exe=$(readlink "/proc/$pid/exe" 2>/dev/null)"
	echo "attr/current=$(cat "/proc/$pid/attr/current" 2>&1)"
	grep -E '^(NoNewPrivs|Seccomp|Seccomp_filters):' "/proc/$pid/status" 2>&1
} >"$run/a7-live.out"
: >"$probe/fifo" # cat sees EOF and exits
sleep 1
printf '%-28s %s\n' a7-live "$(tr '\n' ' ' <"$run/a7-live.out")" | tee -a "$run/summary.txt"

sleep 1
journalctl -k --no-pager -o cat --since "$since" 2>/dev/null | grep -E 'apparmor=|type=1326' >"$run/kernel-audit.txt" || true
echo "kernel audit lines: $(wc -l <"$run/kernel-audit.txt")" | tee -a "$run/summary.txt"
echo "units left: $(systemctl --user list-units --all --no-legend --plain 's10-*' | wc -l)" | tee -a "$run/summary.txt"
echo "new crash reports: $(find /var/crash -newer "$probe/secret" -name '*.crash' 2>/dev/null | wc -l)" | tee -a "$run/summary.txt"
echo "results in $run"
