#!/bin/sh
# S10 driver (throwaway): one transient `systemd-run --user` service per sandbox option, each running inner.py,
# (inner.py travels in the unit's environment, S10_CODE, so it needs no file under $HOME), with the unit's exit, the probe's output and the user journal's lines for that unit saved under
# results/<run>/. Transient units only, each with --collect, so nothing stays loaded; the script checks that no
# s10-* unit and no transient unit file is left at the end. It writes nothing outside results/ and one 0700
# ~/.cache/s10-probe.* dir that it removes again. No sudo. LimitCORE=1 (as in S11): a probe killed by SIGSYS leaves no
# apport report in /var/crash (the kernel skips a pipe core_pattern at RLIMIT_CORE 1).
#
#   sh spikes/S10/run.sh [CASE...]     all cases, or only the named ones
set -eu

here=$(cd "$(dirname "$0")" && pwd -P)
uid=$(id -u)
[ "$uid" -ne 0 ] || { echo "run as the desktop user" >&2; exit 2; }
run=$here/results/$(date +%Y%m%dT%H%M%S)
mkdir -p "$run"
probe=$(mktemp -d "$HOME/.cache/s10-probe.XXXXXXXX")
chmod 0700 "$probe"
trap 'rm -rf "$probe"' EXIT
touch "$probe/.stamp"
code=$(cat "$here/inner.py")

ref=""
for ns in user mnt net uts ipc pid cgroup; do ref="$ref${ref:+,}$ns=$(readlink /proc/self/ns/$ns)"; done
mgr=$(pgrep -u "$uid" -xo systemd)

wait_load() {
	while :; do
		l=$(cut -d' ' -f1 /proc/loadavg)
		[ "${l%%.*}" -lt 40 ] && return
		echo "load $l >= 40, waiting" >&2
		sleep 15
	done
}

leftovers() {
	{
		echo "units: $(systemctl --user list-units --all --no-legend --plain 's10-*' | wc -l)"
		echo "transient files: $(find "/run/user/$uid/systemd/transient" -name 's10-*' 2>/dev/null | wc -l)"
		echo "failed s10 units: $(systemctl --user list-units --failed --no-legend --plain 's10-*' | wc -l)"
		echo "new crash reports: $(find /var/crash -newer "$probe/.stamp" -name '*.crash' 2>/dev/null | wc -l)"
	}
}

{
	echo "kernel=$(uname -r)"
	echo "systemd=$(systemctl --user show -p Version --value)"
	echo "sysctl.apparmor_restrict_unprivileged_userns=$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns)"
	echo "sysctl.unprivileged_userns_clone=$(cat /proc/sys/kernel/unprivileged_userns_clone)"
	echo "lsm=$(cat /sys/kernel/security/lsm)"
	echo "manager.pid=$mgr"
	echo "manager.label=$(cat /proc/$mgr/attr/current)"
	grep -E '^(NoNewPrivs|Seccomp|CapEff)' /proc/$mgr/status | sed 's/:\t*/=/; s/^/manager./'
	for ns in user mnt net; do
		m=$(readlink /proc/$mgr/ns/$ns 2>/dev/null) || m=unreadable
		echo "manager.ns.$ns=$m driver=$(readlink /proc/self/ns/$ns)"
	done
	echo "before: $(leftovers | tr '\n' ' ')"
} >"$run/host.txt"

# name|property;property;...   (PROBE is replaced by the probe dir)
cases() {
	cat <<'EOF'
c00-baseline|
n01-PrivateUsers-yes|PrivateUsers=yes
n02-PrivateUsers-identity|PrivateUsers=identity
n03-ProtectSystem-strict|ProtectSystem=strict
n04-ProtectSystem-full|ProtectSystem=full
n05-ProtectHome-yes|ProtectHome=yes
n06-ProtectHome-read-only|ProtectHome=read-only
n07-ProtectHome-tmpfs|ProtectHome=tmpfs
n08-PrivateTmp|PrivateTmp=yes
n09-PrivateDevices|PrivateDevices=yes
n10-PrivateNetwork|PrivateNetwork=yes
n11-PrivateIPC|PrivateIPC=yes
n12-PrivatePIDs|PrivatePIDs=yes
n13-PrivateMounts|PrivateMounts=yes
n14-ProtectHostname|ProtectHostname=yes
n15-ProtectClock|ProtectClock=yes
n16-ProtectKernelTunables|ProtectKernelTunables=yes
n17-ProtectKernelModules|ProtectKernelModules=yes
n18-ProtectKernelLogs|ProtectKernelLogs=yes
n19-ProtectControlGroups|ProtectControlGroups=yes
n20-ProtectProc-invisible|ProtectProc=invisible
n21-ProcSubset-pid|ProcSubset=pid
n22-ReadOnlyPaths|ReadOnlyPaths=PROBE
n23-InaccessiblePaths|InaccessiblePaths=PROBE
n24-TemporaryFileSystem|TemporaryFileSystem=/var
n25-strict-plus-ReadWritePaths|ProtectSystem=strict;ReadWritePaths=PROBE
n26-NoExecPaths|NoExecPaths=/
n27-old-plan-namespaces|PrivateUsers=yes;ProtectHome=read-only;PrivateNetwork=yes;PrivateTmp=yes;ProtectSystem=strict
n28-RestrictFileSystems|RestrictFileSystems=ext4 tmpfs proc
n29-IPAddressDeny|IPAddressDeny=any
n30-CapabilityBoundingSet-empty|CapabilityBoundingSet=
s01-NoNewPrivileges|NoNewPrivileges=yes
s02-SystemCallFilter-system-service|SystemCallFilter=@system-service;SystemCallErrorNumber=EBADE
s03-SystemCallFilter-system-service-kill|SystemCallFilter=@system-service
s04-SystemCallFilter-denylist|SystemCallFilter=~@privileged @resources @debug @mount @swap @reboot @module @raw-io @cpu-emulation @obsolete @clock;SystemCallErrorNumber=EBADE
s05-SystemCallFilter-deny-execve|SystemCallFilter=~execve;SystemCallErrorNumber=EBADE
s06-SystemCallFilter-deny-process|SystemCallFilter=@system-service;SystemCallFilter=~@process;SystemCallErrorNumber=EBADE
s07-SystemCallArchitectures-native|SystemCallArchitectures=native
s08-RestrictAddressFamilies-unix|RestrictAddressFamilies=AF_UNIX
s09-LockPersonality|LockPersonality=yes
s10-MemoryDenyWriteExecute|MemoryDenyWriteExecute=yes
s11-RestrictNamespaces|RestrictNamespaces=yes
s12-RestrictRealtime|RestrictRealtime=yes
s13-RestrictSUIDSGID|RestrictSUIDSGID=yes
p00-plan-with-CapabilityBoundingSet|NoNewPrivileges=yes;RestrictAddressFamilies=AF_UNIX;SystemCallFilter=@system-service;SystemCallFilter=~@privileged @resources;SystemCallArchitectures=native;SystemCallErrorNumber=EPERM;LockPersonality=yes;MemoryDenyWriteExecute=yes;RestrictNamespaces=yes;RestrictRealtime=yes;RestrictSUIDSGID=yes;CapabilityBoundingSet=
p01-plan|NoNewPrivileges=yes;RestrictAddressFamilies=AF_UNIX;SystemCallFilter=@system-service;SystemCallFilter=~@privileged @resources;SystemCallArchitectures=native;SystemCallErrorNumber=EPERM;LockPersonality=yes;MemoryDenyWriteExecute=yes;RestrictNamespaces=yes;RestrictRealtime=yes;RestrictSUIDSGID=yes
p02-plan-plus-namespaces|NoNewPrivileges=yes;RestrictAddressFamilies=AF_UNIX;SystemCallFilter=@system-service;SystemCallFilter=~@privileged @resources;SystemCallArchitectures=native;SystemCallErrorNumber=EPERM;LockPersonality=yes;MemoryDenyWriteExecute=yes;RestrictNamespaces=yes;RestrictRealtime=yes;RestrictSUIDSGID=yes;ProtectHome=read-only;PrivateNetwork=yes
p03-plan-kill-on-deny|NoNewPrivileges=yes;RestrictAddressFamilies=AF_UNIX;SystemCallFilter=@system-service;SystemCallFilter=~@privileged @resources;SystemCallArchitectures=native;LockPersonality=yes;MemoryDenyWriteExecute=yes;RestrictNamespaces=yes;RestrictRealtime=yes;RestrictSUIDSGID=yes
EOF
}

want() { # name
	[ $# -eq 0 ] && return 0
	for w in $sel; do [ "$w" = "$1" ] && return 0; done
	return 1
}
sel="$*"

cases | while IFS='|' read -r name props; do
	[ -z "$sel" ] || want "$name" || continue
	wait_load
	unit="s10-$name-$$.service"
	set -- --user --unit="$unit" --wait --pipe --collect \
		-p Nice=19 -p IOSchedulingClass=idle -p RuntimeMaxSec=60 -p LimitCORE=1 \
		-E "S10_PROBE_DIR=$probe" -E "S10_REF_NS=$ref" -E "S10_CODE=$code"
	oldifs=$IFS
	IFS=';'
	for p in $props; do
		IFS=$oldifs
		[ -n "$p" ] && set -- "$@" -p "$(printf '%s' "$p" | sed "s|PROBE|$probe|g")"
	done
	IFS=$oldifs
	since=$(date '+%Y-%m-%d %H:%M:%S')
	rc=0
	nice -n 19 systemd-run "$@" /usr/bin/python3 -I -c 'import os; exec(os.environ["S10_CODE"])' </dev/null >"$run/$name.out" 2>"$run/$name.err" || rc=$?
	sleep 1
	journalctl --user --no-pager -o cat --since "$since" -u "$unit" >"$run/$name.journal" 2>&1 || true
	{
		printf 'props=%s\n' "$props"
		printf 'systemd-run.rc=%s\n' "$rc"
	} >"$run/$name.meta"
	rm -f "$probe"/*
	printf '%-42s rc=%s %s\n' "$name" "$rc" "$(grep -E '^Seccomp=|^NoNewPrivs=' "$run/$name.out" | tr '\n' ' ')"
done | tee "$run/summary.txt"

echo "after: $(leftovers | tr '\n' ' ')" | tee -a "$run/host.txt"
echo "results in $run"
