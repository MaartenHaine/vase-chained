#!/usr/bin/env bash
# The M0a exit check (docs/linux-tiler/wayfinder.md §5, "M0a exit"): proof that a Linux Tiler run in podman left the
# real desktop alone. It only reads. It never writes a setting, starts a shell or calls a method that changes state.
#
#   tools/before-after.sh snapshot DIR   record the desktop's state in DIR (created 0700, must not exist yet)
#   tools/before-after.sh compare A B    compare two snapshots; exit 0 when nothing changed, 1 otherwise
#
# What a snapshot holds, and why:
#   dconf          sha256 of `dconf dump /org/gnome/` (the dump itself is kept in DIR for a diff)
#   locked_hint    logind's LockedHint of the graphical session (a nested shell can call SetLockedHint, F18)
#   crash_marker   whether $XDG_RUNTIME_DIR/gnome-shell-disable-extensions exists (a shell started on the host
#                  creates it; the real shell removes its own 60 s after start, so it is "absent" on a settled desktop)
#   ext_tree       a hash over ~/.local/share/gnome-shell: every path, type, mode, link target and file content
#   monitors_xml   mtime and sha256 of ~/.config/monitors.xml
#   owner.*        pid and process of the owner of org.gnome.Shell (session bus), org.freedesktop.PolicyKit1 and
#                  org.freedesktop.NetworkManager (system bus)
#   shell_conns.*  how many connections gnome-shell processes hold on each host bus. polkit and NetworkManager have
#                  no read-only way to list their agents, so a new agent shows up here as a new connection.
#   shell_pids     the gnome-shell processes running on the host
set -euo pipefail

die() { printf 'before-after: %s\n' "$*" >&2; exit 2; }

# Owner of a well-known name as "pid process", or "none". Reads busctl's table; activates nothing.
owner() { # bus name
	local line
	line=$(busctl "--$1" list --acquired --no-legend --no-pager 2>/dev/null | awk -v n="$2" '$1 == n { print $2, $3 }')
	printf '%s\n' "${line:-none}"
}

shell_conns() { # bus
	busctl "--$1" list --unique --no-legend --no-pager 2>/dev/null | awk '$3 == "gnome-shell"' | wc -l
}

tree_hash() { # dir
	local dir=$1
	[ -d "$dir" ] || { echo absent; return; }
	(
		cd "$dir"
		LC_ALL=C find -P . -printf '%y %m %p -> %l\n' | LC_ALL=C sort
		LC_ALL=C find -P . -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum
	) | sha256sum | cut -d' ' -f1
}

snapshot() {
	local dir=$1 sid
	[ -e "$dir" ] && die "$dir exists; give a new directory"
	(umask 077 && mkdir -p "$dir")
	: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is unset: run this from the desktop session}"

	(umask 077 && dconf dump /org/gnome/ >"$dir/dconf.ini")
	sid=$(loginctl show-user "$(id -un)" -p Display --value 2>/dev/null || true)
	{
		printf 'dconf=%s\n' "$(sha256sum <"$dir/dconf.ini" | cut -d' ' -f1)"
		printf 'session=%s\n' "${sid:-none}"
		if [ -n "$sid" ]; then
			printf 'locked_hint=%s\n' "$(loginctl show-session "$sid" -p LockedHint --value)"
		else
			printf 'locked_hint=no-graphical-session\n'
		fi
		if [ -e "$XDG_RUNTIME_DIR/gnome-shell-disable-extensions" ]; then
			printf 'crash_marker=present\n'
		else
			printf 'crash_marker=absent\n'
		fi
		printf 'ext_tree=%s\n' "$(tree_hash "$HOME/.local/share/gnome-shell")"
		if [ -e "$HOME/.config/monitors.xml" ]; then
			printf 'monitors_xml=%s %s\n' "$(stat -c %Y "$HOME/.config/monitors.xml")" \
				"$(sha256sum <"$HOME/.config/monitors.xml" | cut -d' ' -f1)"
		else
			printf 'monitors_xml=absent\n'
		fi
		printf 'owner.session.org.gnome.Shell=%s\n' "$(owner user org.gnome.Shell)"
		printf 'owner.system.org.freedesktop.PolicyKit1=%s\n' "$(owner system org.freedesktop.PolicyKit1)"
		printf 'owner.system.org.freedesktop.NetworkManager=%s\n' "$(owner system org.freedesktop.NetworkManager)"
		printf 'shell_conns.session=%s\n' "$(shell_conns user)"
		printf 'shell_conns.system=%s\n' "$(shell_conns system)"
		printf 'shell_pids=%s\n' "$(pgrep -x gnome-shell | sort -n | tr '\n' ' ' | sed 's/ $//')"
	} >"$dir/summary"
	printf 'taken=%s\n' "$(date -Is)" >"$dir/taken"
	cat "$dir/summary"
}

compare() {
	local a=$1 b=$2
	[ -f "$a/summary" ] && [ -f "$b/summary" ] || die "both arguments must be snapshot directories"
	if diff -u "$a/summary" "$b/summary"; then
		printf 'before-after: nothing changed (%s -> %s)\n' "$(cut -d= -f2 "$a/taken")" "$(cut -d= -f2 "$b/taken")"
		return 0
	fi
	diff -u "$a/dconf.ini" "$b/dconf.ini" || true
	printf 'before-after: the desktop changed\n' >&2
	return 1
}

case "${1:-}" in
	snapshot) [ $# -eq 2 ] || die "usage: $0 snapshot DIR"; snapshot "$2" ;;
	compare) [ $# -eq 3 ] || die "usage: $0 compare BEFORE AFTER"; compare "$2" "$3" ;;
	*) die "usage: $0 snapshot DIR | compare BEFORE AFTER" ;;
esac
