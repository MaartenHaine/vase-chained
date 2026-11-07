#!/bin/sh
# The M0a exit check (docs/linux-tiler/wayfinder.md §5, "M0a exit"): proof that a dev shell (tools/dev-shell.sh)
# or a podman run left the real desktop alone. It only reads: no setting is written, no method that changes
# state is called, nothing is started.
#
#   tools/before-after.sh snapshot FILE   write the desktop's state to FILE (new, mode 0600) and print the summary
#   tools/before-after.sh compare A B     compare two snapshots: exit 0 if no watched item changed, 1 otherwise
#
# Run both snapshots from a normal terminal on the desktop, with the dev shell closed. FILE holds the whole
# `dconf dump /org/gnome/`, so keep it out of anything shared (~/.cache/tiler-dev-snapshots/ is a good place).
#
# What a snapshot records:
#   dconf          sha256 of `dconf dump /org/gnome/`, plus every key as a line, for the diff
#   locked_hint    logind's LockedHint of the graphical session (a nested shell on the host system bus calls
#                  SetLockedHint, F18)
#   crash_marker   whether $XDG_RUNTIME_DIR/gnome-shell-disable-extensions exists (any gnome-shell creates its
#                  own for 60 s after start, extensionSystem.js:54-72; a settled desktop has none)
#   tree           a hash over ~/.local/share/gnome-shell (path, type, mode, link target, content), leaving out
#                  the three files the running host shell writes itself (application_state, its app-usage record,
#                  session-active-history.json and session.gvdb), plus a line per entry, all of them, for the diff
#   application_state_mtime  when the host shell last wrote its app-usage record (shown, never judged)
#   monitors_xml   mtime and sha256 of ~/.config/monitors.xml
#   owner.*        "pid process" owning org.gnome.Shell and org.gnome.Mutter.Devkit on the session bus, and
#                  org.freedesktop.PolicyKit1, org.freedesktop.NetworkManager and org.freedesktop.login1 on
#                  the system bus
#   conns.*        connections held by gnome-shell and mutter-devkit processes on each host bus, as
#                  pid:process:count. Polkit and NetworkManager agents are registered over a connection, not as
#                  a bus name, and neither service lists them read-only, so a second shell's agent shows up here
#                  as a connection from a new pid.
#   shell_pids     gnome-shell and mutter-devkit processes of this user
#   leftovers      processes still carrying a dev shell's TILER_DEV_RUN marker
#   crash          a line per entry of /var/crash and ~/.local/share/apport (if present): type, size, mtime,
#                  path. Names, sizes and times only: other users' reports aren't readable. A crash of anything
#                  the dev shell starts would land here as a .crash report (apport, through kernel.core_pattern),
#                  which Ubuntu offers to report at the next login
#
# Watched vs not: the running host shell itself writes application_state, session-active-history.json and
# session.gvdb in ~/.local/share/gnome-shell, and the owner's apps write dconf keys all the time. Those changes are
# listed but don't fail the check, and tree_sha256 leaves them out. Every other difference does (tree_sha256, the
# locked hint, the crash marker, monitors.xml, the bus owners and connections, the shell pids, the leftovers, any other
# tree line), and so does any change to these dconf keys:
# [shell] enabled-extensions, disabled-extensions, disable-user-extensions, allow-extension-installation,
# development-tools, disable-extension-version-validation; anything under [shell/extensions/...], [mutter...]
# (the devkit's launchers live in /org/gnome/mutter/), [desktop/screensaver], [desktop/lockdown],
# [desktop/session] and [settings-daemon/...]. A crash entry that appears or changes is watched too; one that goes
# away (the owner deleting a report) is listed but isn't.
set -eu

die() { printf 'before-after: %s\n' "$*" >&2; exit 2; }

uid=$(id -u)
real_home=$(getent passwd "$uid" | cut -d: -f6)
runtime=/run/user/$uid
gs_dir=$real_home/.local/share/gnome-shell
monitors=$real_home/.config/monitors.xml
apport_dir=$real_home/.local/share/apport

on_desktop() {
	[ "$uid" -ne 0 ] || die "run this as the desktop user, not root"
	[ -z "${TILER_DEV_RUN:-}" ] || die "this is a dev shell's environment; run it from a normal terminal"
	[ "${HOME:-}" = "$real_home" ] || die "HOME is ${HOME:-unset}, not $real_home"
	[ "${XDG_RUNTIME_DIR:-$runtime}" = "$runtime" ] || die "XDG_RUNTIME_DIR is $XDG_RUNTIME_DIR, not $runtime"
	[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || [ -S "$runtime/bus" ] || die "no session bus"
	case ${DBUS_SESSION_BUS_ADDRESS:-unix:path=$runtime/bus} in
	"unix:path=$runtime/bus" | "unix:path=$runtime/bus,"*) ;;
	*) die "DBUS_SESSION_BUS_ADDRESS is $DBUS_SESSION_BUS_ADDRESS, not the desktop's $runtime/bus" ;;
	esac
	[ -z "${DBUS_SYSTEM_BUS_ADDRESS:-}" ] || die "DBUS_SYSTEM_BUS_ADDRESS is set ($DBUS_SYSTEM_BUS_ADDRESS)"
	[ "${GSETTINGS_BACKEND:-dconf}" = dconf ] || die "GSETTINGS_BACKEND is $GSETTINGS_BACKEND"
}

sha() { sha256sum | cut -d' ' -f1; }

# "pid process" of a well-known name's owner, or "none". busctl only reads; it activates nothing.
owner() { # user|system name
	line=$(busctl "--$1" list --acquired --no-legend --no-pager 2>/dev/null | awk -v n="$2" '$1 == n { print $2, $3 }')
	printf '%s\n' "${line:-none}"
}

conns() { # user|system
	busctl "--$1" list --unique --no-legend --no-pager 2>/dev/null |
		awk '$3 == "gnome-shell" || $3 == "mutter-devkit" { n[$2 ":" $3]++ } END { for (k in n) print k ":" n[k] }' |
		LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//'
}

leftovers() {
	n=0
	for e in /proc/[0-9]*/environ; do
		if tr '\0' '\n' 2>/dev/null <"$e" | grep -q '^TILER_DEV_RUN='; then n=$((n + 1)); fi
	done
	printf '%s\n' "$n"
}

# The files in ~/.local/share/gnome-shell that the running host shell rewrites as the owner uses the desktop: the
# app-usage record, the session history and the session's GVDB. Their changes are listed but never judged.
CHURN='application_state|session-active-history\.json|session\.gvdb'

# One line per entry of ~/.local/share/gnome-shell: "tree TYPE MODE SHA256|- PATH -> TARGET".
tree_lines() {
	[ -d "$gs_dir" ] || { echo 'tree absent'; return; }
	(
		cd "$gs_dir"
		LC_ALL=C find -P . -mindepth 1 -printf '%y %m %P\t%l\n' | LC_ALL=C sort | while IFS="$(printf '\t')" read -r head target; do
			type=${head%% *}
			path=${head#* * }
			if [ "$type" = f ]; then h=$(sha <"$path"); else h=-; fi
			printf 'tree %s %s %s -> %s\n' "${head% "$path"}" "$h" "$path" "$target"
		done
	)
}

# One line per entry of the crash report dirs: "crash TYPE SIZE MTIME PATH".
crash_lines() {
	for d in /var/crash "$apport_dir"; do
		[ -d "$d" ] || continue
		LC_ALL=C find -P "$d" -mindepth 1 -printf 'crash %y %s %Ts %p\n' 2>/dev/null || true
	done | LC_ALL=C sort
}

snapshot() {
	file=$1
	on_desktop
	[ ! -e "$file" ] || die "$file exists; give a new file"
	case $(cd "$(dirname "$file")" 2>/dev/null && pwd -P)/ in
	"$gs_dir"/*) die "don't write the snapshot inside $gs_dir: it would change the tree it records" ;;
	esac
	umask 077

	dump=$(dconf dump /org/gnome/)
	session=$(loginctl show-user "$(id -un)" -p Display --value 2>/dev/null || true)
	tree=$(tree_lines)
	crashes=$(crash_lines)
	{
		printf '# before-after snapshot v3\n'
		printf 'taken=%s\n' "$(date -Is)"
		printf 'dconf_sha256=%s\n' "$(printf '%s\n' "$dump" | sha)"
		printf 'session=%s\n' "${session:-none}"
		if [ -n "$session" ]; then
			printf 'locked_hint=%s\n' "$(loginctl show-session "$session" -p LockedHint --value)"
		else
			printf 'locked_hint=no-graphical-session\n'
		fi
		if [ -e "$runtime/gnome-shell-disable-extensions" ]; then
			printf 'crash_marker=present\n'
		else
			printf 'crash_marker=absent\n'
		fi
		printf 'tree_sha256=%s\n' "$(printf '%s\n' "$tree" | grep -v -E "^tree [^ ]+ [^ ]+ [^ ]+ ($CHURN) -> " | sha)"
		printf 'application_state_mtime=%s\n' "$(stat -c %Y "$gs_dir/application_state" 2>/dev/null || echo absent)"
		if [ -e "$monitors" ]; then
			printf 'monitors_xml=%s %s\n' "$(stat -c %Y "$monitors")" "$(sha <"$monitors")"
		else
			printf 'monitors_xml=absent\n'
		fi
		printf 'owner.session.org.gnome.Shell=%s\n' "$(owner user org.gnome.Shell)"
		printf 'owner.session.org.gnome.Mutter.Devkit=%s\n' "$(owner user org.gnome.Mutter.Devkit)"
		printf 'owner.system.org.freedesktop.PolicyKit1=%s\n' "$(owner system org.freedesktop.PolicyKit1)"
		printf 'owner.system.org.freedesktop.NetworkManager=%s\n' "$(owner system org.freedesktop.NetworkManager)"
		printf 'owner.system.org.freedesktop.login1=%s\n' "$(owner system org.freedesktop.login1)"
		printf 'conns.session=%s\n' "$(conns user)"
		printf 'conns.system=%s\n' "$(conns system)"
		printf 'shell_pids=%s\n' "$(pgrep -u "$uid" -x 'gnome-shell|mutter-devkit' | sort -n | tr '\n' ' ' | sed 's/ $//')"
		printf 'leftovers=%s\n' "$(leftovers)"
		printf '%s\n' "$dump" | awk '/^\[.*\]$/ { s = substr($0, 2, length($0) - 2); next } NF { print "dconf", s, $0 }'
		printf '%s\n' "$tree"
		[ -z "$crashes" ] || printf '%s\n' "$crashes"
	} >"$file"
	grep -v -e '^dconf ' -e '^tree ' -e '^crash ' "$file"
	printf 'crash reports: %s\n' "$(printf '%s' "$crashes" | grep -c '^crash f .*\.crash$' || true)"
}

# Whether a changed line is watched (fails the check) or the expected churn of normal use.
watched() { # line
	case $1 in
	'dconf shell '*)
		key=${1#dconf shell }
		key=${key%%=*}
		case $key in
		enabled-extensions | disabled-extensions | disable-user-extensions | allow-extension-installation | \
			development-tools | disable-extension-version-validation) return 0 ;;
		*) return 1 ;;
		esac
		;;
	'dconf shell/extensions '* | 'dconf shell/extensions/'* | 'dconf mutter '* | 'dconf mutter/'*) return 0 ;;
	'dconf desktop/screensaver '* | 'dconf desktop/lockdown '* | 'dconf desktop/session '*) return 0 ;;
	'dconf settings-daemon/'*) return 0 ;;
	'dconf '*) return 1 ;;
	'tree '*)
		printf '%s\n' "$1" | grep -q -E "^tree [^ ]+ [^ ]+ [^ ]+ ($CHURN) -> " && return 1
		return 0
		;;
	application_state_mtime=*) return 1 ;; # the host shell's own churn
	'#'* | taken=* | dconf_sha256=*) return 1 ;; # covered by the dconf line diffs
	*) return 0 ;;
	esac
}

compare() {
	a=$1
	b=$2
	[ -f "$a" ] && [ -f "$b" ] || die "both arguments must be snapshot files"
	# shellcheck disable=SC2015 # die runs when either file isn't a snapshot
	head -n1 "$a" | grep -qx '# before-after snapshot v3' && head -n1 "$b" | grep -qx '# before-after snapshot v3' ||
		die "not v3 before-after snapshots (v2 hashed the shell's own churn files into tree_sha256: take a new pair)"
	tmp=$(mktemp -d "$real_home/.cache/before-after.XXXXXXXX")
	trap 'rm -rf "$tmp"' EXIT
	LC_ALL=C sort "$a" >"$tmp/a"
	LC_ALL=C sort "$b" >"$tmp/b"
	LC_ALL=C comm -23 "$tmp/a" "$tmp/b" | sed 's/^/- /' >"$tmp/changes"
	LC_ALL=C comm -13 "$tmp/a" "$tmp/b" | sed 's/^/+ /' >>"$tmp/changes"
	: >"$tmp/watched"
	: >"$tmp/other"
	while IFS= read -r line; do
		case $line in
		'- crash '*) printf '%s\n' "$line" >>"$tmp/other"; continue ;; # a report went away
		esac
		if watched "${line#? }"; then
			printf '%s\n' "$line" >>"$tmp/watched"
		else
			printf '%s\n' "$line" >>"$tmp/other"
		fi
	done <"$tmp/changes"
	printf 'before: %s\nafter:  %s\n' "$(grep '^taken=' "$a" | cut -d= -f2)" "$(grep '^taken=' "$b" | cut -d= -f2)"
	if [ -s "$tmp/other" ] && [ "$(grep -c -v -e ' taken=' "$tmp/other")" -gt 0 ]; then
		printf '\nNot watched (normal use of the desktop; read them anyway):\n'
		grep -v -e '^[-+] taken=' "$tmp/other" | LC_ALL=C sort -k2 || true
	fi
	if [ -s "$tmp/watched" ]; then
		printf '\nWATCHED, changed:\n'
		LC_ALL=C sort -k2 "$tmp/watched"
		printf '\nbefore-after: the desktop changed\n' >&2
		return 1
	fi
	printf '\nbefore-after: no watched item changed\n'
}

case ${1:-} in
snapshot) [ $# -eq 2 ] || die "usage: $0 snapshot FILE"; snapshot "$2" ;;
compare) [ $# -eq 3 ] || die "usage: $0 compare BEFORE AFTER"; compare "$2" "$3" ;;
*) die "usage: $0 snapshot FILE | compare BEFORE AFTER" ;;
esac
