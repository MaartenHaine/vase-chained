#!/bin/sh
# The L3 dev shell: gnome-shell 50 --devkit in a window on this desktop, cut off from the real session
# (docs/linux-tiler/wayfinder.md §5 "Test levels", L3; devtest-security.md §3.3). Spike S11 asks whether it runs.
#
#   tools/dev-shell.sh [--x11] [--debug]   start a dev shell that loads only vase-gnome/extension
#   tools/dev-shell.sh clean               delete finished runs (~/.cache/tiler-dev.*)
#
#   --x11    let mutter start Xwayland on demand (its sockets go in the shared /tmp/.X11-unix); default --no-x11
#   --debug  G_MESSAGES_DEBUG='GNOME Shell'
#
# What is isolated, and checked again just before gnome-shell starts (the run is refused otherwise):
#   environment   env -i and an allow-list; nothing of the login session (XDG_SESSION_ID, DISPLAY,
#                 SESSION_MANAGER, SSH_AUTH_SOCK, NOTIFY_SOCKET, ...) gets through
#   system bus    a plain dbus-daemon with no service files: no logind, GDM, polkit, NetworkManager,
#                 AccountsService or UPower. With no GDM, LoginManager.canLock() is false, so no ScreenShield
#                 (loginManager.js canLock, main.js:237); with no logind, no Lock signal and no SetLockedHint
#   session bus   dbus-run-session on a config with no service files, so nothing is D-Bus-activated
#   files         HOME, XDG_{CONFIG,DATA,STATE,CACHE}_HOME and XDG_RUNTIME_DIR in a fresh 0700
#                 ~/.cache/tiler-dev.XXXXXXXX, never /tmp: monitors.xml, the crash marker
#                 gnome-shell-disable-extensions and the nested Wayland socket stay in it
#   settings      GSETTINGS_BACKEND=keyfile: every write lands in that dir's keyfile, never in dconf
#   session mode  tiler-dev: no parent mode, no components, no unlock dialog; it enables only the extension,
#                 a symlink to vase-gnome/extension in this working tree. A mode file that doesn't load makes
#                 gnome-shell fall back to 'user' silently, so the mode is proven twice: before the start, by
#                 gnome-shell's own loader code run in plain gjs (tools/mode-check.js), and after it, on the private
#                 session bus, where the shell is killed at once unless it shows it runs tiler-dev
#   crash reports RLIMIT_CORE is 1 for the whole run: the kernel then skips the core_pattern pipe, so a crash
#                 writes no /var/crash report (apport writes one even at ulimit -c 0)
#
# Two host sockets are passed on purpose, both as absolute paths, because the devkit can't work without them:
#   WAYLAND_DISPLAY  mutter reads it once, at setup, and gives it only to the viewer it spawns,
#                    /usr/libexec/mutter-devkit (mutter 50.1 meta-mdk.c: meta_mdk_new, maybe_launch_devkit).
#                    Before any client starts, mutter replaces it in its own environment with the nested
#                    display (meta-wayland.c: set_gnome_env), so nothing else inherits the host's.
#   PIPEWIRE_REMOTE  the viewer gets the nested screen as a PipeWire screencast stream, and the host's
#                    WirePlumber links the two ends (mdk-stream.c: PW_STREAM_FLAG_AUTOCONNECT;
#                    meta-screen-cast-stream-src.c: pw_context_connect). Both ends would otherwise look for
#                    pipewire-0 in the private XDG_RUNTIME_DIR and fail.
#
# Closing the viewer window ends the dev shell (meta-mdk.c: on_devkit_died); so does Ctrl+C here.
# Logs stay in the run dir (printed at the end); `tools/dev-shell.sh clean` removes finished runs.
#
# Take tools/before-after.sh snapshots before and after, from a normal terminal.
set -eu

die() { printf 'dev-shell: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'dev-shell: refusing to start: %s\n' "$*" >&2; exit 1; }
say() { printf 'dev-shell: %s\n' "$*" >&2; }

uid=$(id -u)
real_home=$(getent passwd "$uid" | cut -d: -f6)
[ -n "$real_home" ] && [ -d "$real_home" ] || die "can't find the home directory of uid $uid"
host_runtime=/run/user/$uid
repo=$(cd "$(dirname "$0")/.." && pwd -P)
self=$repo/tools/dev-shell.sh
mode=tiler-dev

# sha256 of the two gnome-shell 50.1 files whose loader the mode check runs (ui/sessionMode.js, misc/fileUtils.js in
# libshell-18.so). Another build may load modes differently, so it's refused until the files are read again and
# these are updated.
session_mode_sha=6e2af282fb64799026ac05692b541dd8523272902243985df94ed61b50b87c05
file_utils_sha=4a8cdecc365f178d7eefd4001b3a7d8a3587e4615b681522a7224e59c2defbab
libshell=/usr/lib/gnome-shell/libshell-18.so

# Every own process that still carries this run's marker (the shell, the viewer, Xwayland, ibus-daemon, apps).
run_pids() { # run dir
	# Skip this shell and the subshell running this function ($$ is the parent's pid inside $(...)).
	read -r me _ </proc/self/stat
	for e in /proc/[0-9]*/environ; do
		p=${e#/proc/}
		p=${p%/environ}
		{ [ "$p" = "$$" ] || [ "$p" = "$me" ]; } && continue
		if tr '\0' '\n' 2>/dev/null <"$e" | grep -qxF "TILER_DEV_RUN=$1"; then
			printf '%s\n' "$p"
		fi
	done
}

# The socket path of a unix:path= bus address, or nothing for any other kind of address.
bus_path() {
	case $1 in
	unix:path=*) p=${1#unix:path=}; printf '%s\n' "${p%%,*}" ;;
	esac
}

json_check() { # file python-expression-over-d
	python3 -I -c 'import json, sys
d = json.load(open(sys.argv[1]))
sys.exit(0 if eval(sys.argv[2]) else 1)' "$1" "$2"
}

# ---------------------------------------------------------------------------------------------------------------
# Stage A, in the caller's environment: check the host, build the run dir, then re-exec under env -i.
host_stage() {
	x11=0
	debug=0
	for a in "$@"; do
		case $a in
		--x11) x11=1 ;;
		--debug) debug=1 ;;
		-h | --help) sed -n '2,10p' "$self"; exit 0 ;;
		*) die "unknown argument: $a (see --help)" ;;
		esac
	done

	[ "$uid" -ne 0 ] || refuse "running as root"
	[ -z "${TILER_DEV_RUN:-}" ] || refuse "already inside a dev shell ($TILER_DEV_RUN)"
	[ "${XDG_RUNTIME_DIR:-$host_runtime}" = "$host_runtime" ] ||
		refuse "XDG_RUNTIME_DIR is $XDG_RUNTIME_DIR, not $host_runtime: run this from a terminal on the desktop"
	[ -d "$host_runtime" ] && [ "$(stat -c %u "$host_runtime")" = "$uid" ] || refuse "no runtime dir $host_runtime"

	for b in /usr/bin/gnome-shell /usr/libexec/mutter-devkit /usr/bin/dbus-daemon /usr/bin/dbus-run-session \
		/usr/bin/gdbus /usr/bin/gsettings /usr/bin/python3 /usr/bin/gjs /usr/bin/gresource /usr/bin/prlimit; do
		[ -x "$b" ] || refuse "$b is missing (the viewer is in mutter-dev-bin, gresource in libglib2.0-bin)"
	done
	[ -f "$libshell" ] || refuse "$libshell is missing"
	[ -f "$repo/tools/mode-check.js" ] || refuse "$repo/tools/mode-check.js is missing"
	shell_v=$(dpkg-query -W -f='${Version}' gnome-shell)
	case $shell_v in 50.*) ;; *) refuse "gnome-shell is $shell_v; this recipe was written against 50.1" ;; esac
	mutter_v=$(dpkg-query -W -f='${Version}' libmutter-18-0)
	viewer_v=$(dpkg-query -W -f='${Version}' mutter-dev-bin)
	[ "$mutter_v" = "$viewer_v" ] ||
		refuse "mutter-dev-bin $viewer_v doesn't match libmutter-18-0 $mutter_v (their D-Bus API must agree)"

	wd=${WAYLAND_DISPLAY:-}
	[ -n "$wd" ] || refuse "WAYLAND_DISPLAY is unset: run this from a terminal on the GNOME desktop"
	case $wd in /*) host_wayland=$wd ;; *) host_wayland=$host_runtime/$wd ;; esac
	[ -S "$host_wayland" ] || refuse "the host Wayland socket $host_wayland doesn't exist"
	host_pipewire=$host_runtime/pipewire-0
	[ -S "$host_pipewire" ] || refuse "the host PipeWire socket $host_pipewire doesn't exist (the viewer needs it)"

	host_session_bus=${DBUS_SESSION_BUS_ADDRESS:-unix:path=$host_runtime/bus}
	host_system_bus=${DBUS_SYSTEM_BUS_ADDRESS:-unix:path=/run/dbus/system_bus_socket}

	ext=$repo/vase-gnome/extension
	[ -f "$ext/metadata.json" ] && [ -f "$ext/extension.js" ] || refuse "no extension in $ext"
	uuid=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["uuid"])' "$ext/metadata.json")
	case $uuid in *[!A-Za-z0-9._@-]* | '') refuse "odd uuid in $ext/metadata.json: $uuid" ;; esac
	json_check "$ext/metadata.json" "'50' in d.get('shell-version', []) and '$mode' in d.get('session-modes', [])" ||
		refuse "$ext/metadata.json must list shell-version 50 and session-modes $mode, or the shell won't enable it"

	mkdir -p "$real_home/.cache"
	R=$(mktemp -d "$real_home/.cache/tiler-dev.XXXXXXXX")
	chmod 700 "$R"
	R=$(cd "$R" && pwd -P)
	case $R in /tmp/* | /var/tmp/* | /dev/shm/*) refuse "the run dir resolved into $R" ;; esac
	case $R in *[!A-Za-z0-9/._-]*) refuse "odd characters in the run dir $R" ;; esac
	[ "$(stat -c '%a %u' "$R")" = "700 $uid" ] || refuse "$R isn't 0700 and ours"

	(umask 077 && mkdir "$R/run" "$R/home" "$R/config" "$R/data" "$R/state" "$R/cache" "$R/log" "$R/dbus")
	mkdir -p "$R/share/gnome-shell/extensions" "$R/share/gnome-shell/modes" "$R/dbus/services" \
		"$R/config/glib-2.0/settings"
	ln -s "$ext" "$R/share/gnome-shell/extensions/$uuid"

	# No parentMode: the mode inherits only 'restrictive' (no components, unlockDialog null, sessionMode.js:21-48).
	# A mode file that fails to load would make gnome-shell fall back to 'user' (sessionMode.js:149-152), so stage C
	# proves that this one loads, before and after the start.
	cat >"$R/share/gnome-shell/modes/$mode.json" <<EOF
{
  "hasOverview": true,
  "hasRunDialog": true,
  "hasWorkspaces": true,
  "hasWindows": true,
  "hasNotifications": true,
  "hasWmMenus": true,
  "showCalendarEvents": false,
  "showWelcomeDialog": false,
  "allowSettings": false,
  "allowScreencast": false,
  "components": [],
  "enabledExtensions": ["$uuid"],
  "panel": { "left": ["activities"], "center": ["dateMenu"], "right": [] }
}
EOF

	# The keyfile starts with the owner's lockdown (F9), so the mode alone has to enable the extension. The welcome
	# key is a sentinel: stage C reads it back to prove the keyfile backend is the one in use.
	cat >"$R/config/glib-2.0/settings/keyfile" <<'EOF'
[org/gnome/shell]
allow-extension-installation=false
disable-user-extensions=true
welcome-dialog-last-shown-version='tiler-dev-sentinel'
EOF

	for bus in system session; do
		if [ $bus = system ]; then
			sock=$R/run/system_bus_socket
			extra=''
		else
			sock=$R/run/bus
			extra='  <type>session</type>
  <keep_umask/>'
		fi
		cat >"$R/dbus/$bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<!-- tiler-dev private $bus bus: no service directories, so nothing can be activated on it. -->
<busconfig>
$extra
  <listen>unix:path=$sock</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
	done

	set -- PATH=/usr/local/bin:/usr/bin:/bin \
		HOME="$R/home" USER="$(id -un)" LOGNAME="$(id -un)" SHELL=/bin/sh \
		XDG_CONFIG_HOME="$R/config" XDG_DATA_HOME="$R/data" XDG_STATE_HOME="$R/state" \
		XDG_CACHE_HOME="$R/cache" XDG_RUNTIME_DIR="$R/run" \
		XDG_DATA_DIRS="$R/share:/usr/share" XDG_CONFIG_DIRS=/etc/xdg \
		XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland \
		GSETTINGS_BACKEND=keyfile GIO_USE_VFS=local GIO_USE_VOLUME_MONITOR=unix NO_AT_BRIDGE=1 GTK_A11Y=none \
		SHELL_DEBUG=backtrace-warnings,backtrace-segfaults \
		WAYLAND_DISPLAY="$host_wayland" PIPEWIRE_REMOTE="$host_pipewire" \
		DBUS_SYSTEM_BUS_ADDRESS="unix:path=$R/run/system_bus_socket" \
		TILER_DEV_RUN="$R" TILER_DEV_UUID="$uuid" TILER_DEV_EXT="$ext" TILER_DEV_X11="$x11" \
		TILER_DEV_REAL_HOME="$real_home" TILER_DEV_HOST_RUNTIME="$host_runtime" \
		TILER_DEV_HOST_WAYLAND="$host_wayland" \
		TILER_DEV_HOST_SESSION_BUS="$host_session_bus" TILER_DEV_HOST_SYSTEM_BUS="$host_system_bus"
	[ "$debug" = 1 ] && set -- "$@" "G_MESSAGES_DEBUG=GNOME Shell"
	for v in LANG LANGUAGE LC_ALL LC_CTYPE LC_NUMERIC LC_TIME LC_COLLATE LC_MONETARY LC_MESSAGES LC_PAPER \
		LC_NAME LC_ADDRESS LC_TELEPHONE LC_MEASUREMENT LC_IDENTIFICATION TZ; do
		eval "val=\${$v-}"
		[ -n "$val" ] && set -- "$@" "$v=$val"
	done

	say "run dir $R"
	# RLIMIT_CORE 1 (soft and hard, so nothing in the run can raise it) is the kernel's "skip the core_pattern pipe"
	# value: a crash then reaches no apport and leaves no /var/crash report. Apport ignores ulimit -c 0 for its report.
	exec /usr/bin/prlimit --core=1 -- env -i "$@" /bin/sh "$self" __bus-stage
}

# ---------------------------------------------------------------------------------------------------------------
# Stage B, under env -i: start the private system bus, then the private session bus around stage C.
bus_stage() {
	R=$TILER_DEV_RUN
	sys_pid=''
	cleaned=0
	# shellcheck disable=SC2329 # called from the traps below
	cleanup() {
		[ "$cleaned" = 1 ] && return
		cleaned=1
		[ -n "$sys_pid" ] && kill "$sys_pid" 2>/dev/null || true
		left=$(run_pids "$R")
		if [ -n "$left" ]; then
			# shellcheck disable=SC2086 # one pid per word
			kill $left 2>/dev/null || true
			sleep 2
			left=$(run_pids "$R")
			# shellcheck disable=SC2086
			[ -z "$left" ] || kill -KILL $left 2>/dev/null || true
		fi
		rm -f "$R/share/gnome-shell/extensions/$TILER_DEV_UUID"
		find "$R/run" -mindepth 1 -maxdepth 1 -type s -delete 2>/dev/null || true
		say "logs in $R/log (shell.log, system-bus.log); 'tools/dev-shell.sh clean' removes finished runs"
	}
	trap cleanup EXIT
	trap 'cleanup; exit 130' INT
	trap 'cleanup; exit 143' TERM HUP

	/usr/bin/dbus-daemon --config-file="$R/dbus/system.conf" --nofork --nopidfile --nosyslog \
		>"$R/log/system-bus.log" 2>&1 &
	sys_pid=$!
	n=0
	until [ -S "$R/run/system_bus_socket" ]; do
		n=$((n + 1))
		[ "$n" -le 300 ] || die "the private system bus didn't start (see $R/log/system-bus.log)"
		kill -0 "$sys_pid" 2>/dev/null || die "the private system bus exited (see $R/log/system-bus.log)"
		sleep 0.1
	done

	status=0
	/usr/bin/dbus-run-session --dbus-daemon=/usr/bin/dbus-daemon --config-file="$R/dbus/session.conf" \
		-- /bin/sh "$self" __shell-stage || status=$?
	exit "$status"
}

# ---------------------------------------------------------------------------------------------------------------
# Stage C, on both private buses: refuse unless everything is isolated, then run the devkit shell.
shell_stage() {
	R=$TILER_DEV_RUN

	# The environment is exactly the allow-list (dash adds PWD; dbus-run-session adds DBUS_SESSION_BUS_ADDRESS).
	extra=$(env | sed 's/=.*//' | while IFS= read -r name; do
		case $name in
		PATH | HOME | USER | LOGNAME | SHELL | PWD | OLDPWD | SHLVL | _ | LANG | LANGUAGE | LC_* | TZ) ;;
		XDG_CONFIG_HOME | XDG_DATA_HOME | XDG_STATE_HOME | XDG_CACHE_HOME | XDG_RUNTIME_DIR) ;;
		XDG_DATA_DIRS | XDG_CONFIG_DIRS | XDG_CURRENT_DESKTOP | XDG_SESSION_TYPE) ;;
		GSETTINGS_BACKEND | GIO_USE_VFS | GIO_USE_VOLUME_MONITOR | NO_AT_BRIDGE | GTK_A11Y) ;;
		SHELL_DEBUG | G_MESSAGES_DEBUG | WAYLAND_DISPLAY | PIPEWIRE_REMOTE) ;;
		DBUS_SYSTEM_BUS_ADDRESS | DBUS_SESSION_BUS_ADDRESS | TILER_DEV_*) ;;
		*) printf '%s ' "$name" ;;
		esac
	done)
	[ -z "$extra" ] || refuse "variables outside the allow-list: $extra"

	# Settings: the keyfile backend, and our keyfile (the sentinel only exists there).
	[ "${GSETTINGS_BACKEND:-}" = keyfile ] || refuse "GSETTINGS_BACKEND is '${GSETTINGS_BACKEND:-}', not keyfile"
	[ "$(gsettings get org.gnome.shell welcome-dialog-last-shown-version)" = "'tiler-dev-sentinel'" ] ||
		refuse "GSettings doesn't read the run's keyfile"

	# HOME and the XDG dirs are redirected into the run dir.
	[ "$HOME" = "$R/home" ] || refuse "HOME is $HOME, not $R/home"
	[ "$(cd "$HOME" && pwd -P)" != "$(cd "$TILER_DEV_REAL_HOME" && pwd -P)" ] || refuse "HOME is the real home"
	for d in "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"; do
		case $d in "$R"/*) ;; *) refuse "$d is outside the run dir $R" ;; esac
	done
	[ "$XDG_RUNTIME_DIR" != "$TILER_DEV_HOST_RUNTIME" ] || refuse "XDG_RUNTIME_DIR is the host's"
	[ "$(stat -c '%a %u' "$XDG_RUNTIME_DIR")" = "700 $uid" ] || refuse "$XDG_RUNTIME_DIR isn't 0700 and ours"

	# Both buses are ours: the address is the run dir's socket, not the host's (by text and by inode), and the bus
	# can activate nothing.
	for bus in system session; do
		if [ $bus = system ]; then
			addr=${DBUS_SYSTEM_BUS_ADDRESS:-}
			want=$R/run/system_bus_socket
			host=$TILER_DEV_HOST_SYSTEM_BUS
		else
			addr=${DBUS_SESSION_BUS_ADDRESS:-}
			want=$R/run/bus
			host=$TILER_DEV_HOST_SESSION_BUS
		fi
		[ -n "$addr" ] || refuse "the $bus bus address is unset"
		[ "$addr" != "$host" ] || refuse "the $bus bus is the host's ($addr)"
		path=$(bus_path "$addr")
		[ "$path" = "$want" ] || refuse "the $bus bus address $addr isn't unix:path=$want"
		[ -S "$path" ] || refuse "no $bus bus socket at $path"
		hpath=$(bus_path "$host")
		if [ -n "$hpath" ] && [ -e "$hpath" ]; then
			[ "$(stat -L -c '%d:%i' "$path")" != "$(stat -L -c '%d:%i' "$hpath")" ] ||
				refuse "the $bus bus socket is the host's $hpath"
		fi
		names=$(gdbus call --"$bus" --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
			--method org.freedesktop.DBus.ListActivatableNames | tr -d ' ')
		[ "$names" = "(['org.freedesktop.DBus'],)" ] || refuse "the $bus bus can activate services: $names"
		owned=$(gdbus call --"$bus" --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
			--method org.freedesktop.DBus.ListNames | grep -o "'[^']*'" | tr -d "'" |
			grep -v -e '^:' -e '^org\.freedesktop\.DBus$' || true)
		[ -z "$owned" ] || refuse "the $bus bus already has names on it: $owned"
	done

	# Nothing points at the login session; the only host paths are the viewer's two sockets.
	case $WAYLAND_DISPLAY in /*) ;; *) refuse "WAYLAND_DISPLAY must be an absolute path to the host socket" ;; esac
	[ "$WAYLAND_DISPLAY" = "$TILER_DEV_HOST_WAYLAND" ] && [ -S "$WAYLAND_DISPLAY" ] ||
		refuse "WAYLAND_DISPLAY $WAYLAND_DISPLAY isn't the host compositor's socket"
	[ "$PIPEWIRE_REMOTE" = "$TILER_DEV_HOST_RUNTIME/pipewire-0" ] || refuse "PIPEWIRE_REMOTE is $PIPEWIRE_REMOTE"

	# A crash leaves no report on the host: RLIMIT_CORE is 1, soft and hard (stage A).
	awk '$1 == "Max" && $2 == "core" { ok = ($5 == "1" && $6 == "1") } END { exit !ok }' /proc/self/limits ||
		refuse "RLIMIT_CORE isn't 1: a crash would write a report to /var/crash"

	# The mode loads, and it is the one we wrote; the extension it enables is this working tree's.
	case $XDG_DATA_DIRS in "$R/share":*) ;; *) refuse "XDG_DATA_DIRS doesn't start with $R/share" ;; esac
	mode_check "$R/share/gnome-shell/modes/$mode.json"
	link=$R/share/gnome-shell/extensions/$TILER_DEV_UUID
	[ -L "$link" ] && [ "$(cd "$link" && pwd -P)" = "$TILER_DEV_EXT" ] || refuse "$link doesn't point at $TILER_DEV_EXT"

	set -- --wayland --devkit "--mode=$mode"
	[ "$TILER_DEV_X11" = 1 ] || set -- "$@" --no-x11
	say "isolation checks passed; starting gnome-shell $*"
	: >"$R/log/shell.log"
	env -u TILER_DEV_HOST_SESSION_BUS -u TILER_DEV_HOST_SYSTEM_BUS -u TILER_DEV_HOST_WAYLAND \
		-u TILER_DEV_HOST_RUNTIME -u TILER_DEV_REAL_HOME /usr/bin/gnome-shell "$@" >"$R/log/shell.log" 2>&1 &
	sp=$!
	# A script's background job ignores SIGINT, so Ctrl+C here is passed on to the shell as SIGTERM.
	trap 'kill -TERM "$sp" 2>/dev/null || true' INT TERM HUP
	tail -n +1 -f --pid="$sp" "$R/log/shell.log" &
	tp=$!

	if ! running_mode_ok "$sp"; then
		kill -KILL "$sp" 2>/dev/null || true
		wait "$sp" 2>/dev/null || true
		wait "$tp" 2>/dev/null || true
		die "killed gnome-shell: it didn't prove that it runs the $mode mode (log: $R/log/shell.log)"
	fi
	say "the viewer window is on this desktop; close it (or Ctrl+C here) to end the dev shell"

	# wait returns early when a trapped signal arrives; wait again until the shell itself has exited.
	st=0
	wait "$sp" || st=$?
	while kill -0 "$sp" 2>/dev/null; do
		st=0
		wait "$sp" || st=$?
	done
	wait "$tp" 2>/dev/null || true
	exit "$st"
}

# Before the start: gnome-shell's own mode loader, run in plain gjs, loads this mode file and lists the mode as
# primary, so SessionMode won't fall back to 'user' (tools/mode-check.js says what it checks and why that is
# enough). This replaces `gnome-shell --list-modes`, which segfaults in 50.1 while it parses the command line.
mode_check() { # mode-file
	mc=$R/modecheck
	rm -rf "$mc"
	(umask 077 && mkdir "$mc" "$mc/ui" "$mc/misc" "$mc/gdm")
	for f in ui/sessionMode.js misc/fileUtils.js; do
		/usr/bin/gresource extract "$libshell" "/org/gnome/shell/$f" >"$mc/$f" 2>/dev/null ||
			refuse "can't extract $f from $libshell"
	done
	[ "$(sha256sum <"$mc/ui/sessionMode.js" | cut -d' ' -f1)" = "$session_mode_sha" ] &&
		[ "$(sha256sum <"$mc/misc/fileUtils.js" | cut -d' ' -f1)" = "$file_utils_sha" ] ||
		refuse "gnome-shell's sessionMode.js or fileUtils.js isn't the 50.1 one the mode check was written against"
	# Stubs for what the two files import but the loader never reads: SessionMode's base class (SessionMode isn't
	# constructed here), the gdm and unlock dialog classes (stored as values; unlockDialog is filtered out of
	# loaded modes), HAVE_NETWORKMANAGER (only the 'user' component list) and loadInterfaceXML (re-exported only).
	echo 'export class EventEmitter {}' >"$mc/misc/signals.js"
	echo 'export function loadInterfaceXML() { throw new Error("mode-check stub"); }' >"$mc/misc/dbusUtils.js"
	echo 'export const HAVE_NETWORKMANAGER = 1;' >"$mc/misc/config.js"
	echo 'export class LoginDialog {}' >"$mc/gdm/loginDialog.js"
	echo 'export class UnlockDialog {}' >"$mc/ui/unlockDialog.js"
	cp "$repo/tools/mode-check.js" "$mc/mode-check.js"
	# The environment gnome-shell gets, so GLib.get_system_data_dirs() is the shell's.
	modes=$(env -u TILER_DEV_HOST_SESSION_BUS -u TILER_DEV_HOST_SYSTEM_BUS -u TILER_DEV_HOST_WAYLAND \
		-u TILER_DEV_HOST_RUNTIME -u TILER_DEV_REAL_HOME \
		/usr/bin/gjs -m "$mc/mode-check.js" "$mode" "$1" "$TILER_DEV_UUID") ||
		refuse "the $mode mode file fails the mode check; gnome-shell would fall back to the 'user' mode"
	printf '%s\n' "$modes" | grep -qx "$mode" ||
		refuse "gnome-shell's loader doesn't load the $mode mode; it would fall back to the 'user' mode"
}

# After the start: the running shell shows on the private session bus that it runs the tiler-dev mode, or the caller
# kills it at once. gdbus activates nothing (the bus has no service files). Three signs:
#   - org.gnome.keyring.SystemPrompter has no owner. The keyring component claims it (components/keyring.js
#     enable), and of the two possible modes only 'user' has components. It is polled from the start, before
#     org.gnome.Shell is exported, so a fallback is caught as soon as the components come up.
#   - org.gnome.Shell's Mode is 'tiler-dev'. Mode is global.session_mode (shellDBus.js `get Mode`): the --mode the
#     shell was given, not the mode SessionMode settled on, so it proves only that the option arrived.
#   - GetExtensionInfo(uuid) has enabled: true. `enabled` is whether the uuid is in the extension manager's enabled
#     list (extensionSystem.js createExtensionObject), which with disable-user-extensions=true is the running
#     mode's enabledExtensions alone; 'user' has none. It doesn't depend on the extension's own code running.
# Returns 0 on that proof, or when the shell has already exited; 1 on a wrong sign, or no proof within 90 s.
running_mode_ok() { # shell-pid
	start=$(date +%s)
	mode_seen=0
	while kill -0 "$1" 2>/dev/null; do
		r=$(gdbus call --session --timeout 5 --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
			--method org.freedesktop.DBus.NameHasOwner org.gnome.keyring.SystemPrompter 2>/dev/null) || r=''
		if [ "$r" = '(true,)' ]; then
			say "org.gnome.keyring.SystemPrompter is on the bus: the shell fell back to the 'user' mode"
			return 1
		fi
		if [ "$mode_seen" = 0 ] &&
			r=$(gdbus call --session --timeout 5 --dest org.gnome.Shell --object-path /org/gnome/Shell \
				--method org.freedesktop.DBus.Properties.Get org.gnome.Shell Mode 2>/dev/null); then
			if [ "$r" != "(<'$mode'>,)" ]; then
				say "org.gnome.Shell's Mode is $r, not '$mode'"
				return 1
			fi
			mode_seen=1
		fi
		if [ "$mode_seen" = 1 ] &&
			r=$(gdbus call --session --timeout 5 --dest org.gnome.Shell --object-path /org/gnome/Shell \
				--method org.gnome.Shell.Extensions.GetExtensionInfo "$TILER_DEV_UUID" 2>/dev/null); then
			case $r in
			*"'enabled': <true>"*)
				say "the shell runs the $mode mode (Mode '$mode', $TILER_DEV_UUID enabled, no keyring prompter)"
				return 0
				;;
			*"'enabled': <false>"*)
				say "$TILER_DEV_UUID isn't among the running mode's extensions: the mode isn't $mode"
				return 1
				;;
			esac
		fi
		if [ $(($(date +%s) - start)) -ge 90 ]; then
			say "no proof of the $mode mode 90 s after the start"
			return 1
		fi
		sleep 0.2
	done
	return 0
}

# ---------------------------------------------------------------------------------------------------------------
clean() {
	[ -z "${TILER_DEV_RUN:-}" ] || refuse "inside a dev shell"
	for d in "$real_home"/.cache/tiler-dev.*; do
		[ -d "$d" ] && [ ! -L "$d" ] && [ "$(stat -c %u "$d")" = "$uid" ] || continue
		if [ -n "$(run_pids "$d")" ]; then
			say "skipping $d: still running"
			continue
		fi
		rm -rf -- "$d"
		say "removed $d"
	done
}

case ${1:-} in
__bus-stage) bus_stage ;;
__shell-stage) shell_stage ;;
clean) clean ;;
*) host_stage "$@" ;;
esac
