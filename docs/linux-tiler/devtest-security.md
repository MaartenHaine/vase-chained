# Linux Tiler: developing, testing and securing a GNOME 50 Wayland extension

Lens: development, testing and security. Research only, done 2026-11-07. Nothing on the desktop, in
`~/src/vase-chained` or in the Chained repo was changed. All reads of the live desktop were read-only
(`gsettings get`, `dpkg -l`, `/proc/<pid>/environ`, `systemctl --user cat`).

Labels: **VERIFIED** means checked in source at the 50.1 tags or on this desktop. **INFERENCE** means
reasoned from verified facts. **UNVERIFIED** means plausible but not checked.

Source files fetched for this note are in `/home/martin/.claude/jobs/91ccbb14/tmp/tiler/scratch/src/`
(gnome-shell and mutter at tag `50.1`, from the GitHub mirrors of gitlab.gnome.org; `js_ui_extensionSystem.js`
was diffed against the 50.1 raw file and matches it). The gjs.guide pages are in `scratch/web/`.

---

## 0. Summary

1. **Never develop on the live session.** On Wayland the Tiler runs inside the compositor. A segfault or
   a hung main loop ends the session: `org.gnome.Shell@.service` has
   `OnFailure=... gnome-session-shutdown.target`, so you are logged out and every unsaved window is lost.
   Code can't be reloaded in place either: JS modules can't be unloaded, and Wayland has no shell restart.
   So every iteration starts a fresh shell anyway, and that shell should be a separate one.
2. **The safest automated loop is already installed.** `/usr/bin/gnome-shell-test-tool` (part of the
   `gnome-shell` package) gives each run a temporary `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME`
   with `GSETTINGS_BACKEND=keyfile`, so the real dconf is never touched. Since 50, its `--extension x.zip`
   option installs and enables an extension in that throwaway home. It then runs a test module inside a
   `--headless --virtual-monitor` shell, and a failing test gives a non-zero exit. Run it inside rootless
   podman (own session and system bus) on the desktop and in GitHub Actions (an `ubuntu:26.04` container).
3. **For interactive work, use the devkit, not `--nested`.** In mutter 50, `--nested` is gone and
   `--devkit` exists. Ubuntu's libmutter is built with devkit support, but the viewer
   (`/usr/libexec/mutter-devkit`, package `mutter-dev-bin`, universe) is **not installed**, so this needs
   an owner-approved apt install. Run the devkit under `dbus-run-session` with the XDG dirs and HOME
   redirected, `GSETTINGS_BACKEND=keyfile`, and a custom dev session mode, so that none of
   `allow-extension-installation`, `disable-user-extensions` or `enabled-extensions` changes on the real
   desktop. Some isolation stays incomplete: the host system bus, logind and the runtime dir are shared.
4. **The audit has a contradiction to fix before anything ships.** Section 24 condition 1 says "installed
   by chezmoi" (that is, into the home directory), and condition 2 says `allow-extension-installation`
   false. With that key false, gnome-shell 50.1 **does not scan `~/.local/share/gnome-shell/extensions` at
   all** (`extensionSystem.js:711-712`). Also, this desktop has `disable-user-extensions` **true**, which
   makes gnome-shell ignore `enabled-extensions` completely and enable only the session mode's list
   (`extensionSystem.js:545-556`). Chained's GNOME lint forbids writing either key. So the Tiler can't be
   enabled at all under today's rules; one of the rules has to change.
5. **Recommended install channel:** a root-owned system directory (`/usr/local/share/gnome-shell/extensions/<uuid>`,
   which is already in the session's `XDG_DATA_DIRS`), copied by Chained's existing sudo step from the
   reviewed commit. Chained's lint would then allow exactly two extension writes:
   `disable-user-extensions=false` and the Tiler UUID in `enabled-extensions` (plus Tiling Assistant in
   `disabled-extensions` at the switch-over). Don't use a session-mode file. A mode extension escapes
   GNOME's crash safety net, which only flips `disable-user-extensions`, so a crashing Tiler would
   crash-loop the login.
6. **Be honest about the threat model.** `allow-extension-installation=false` is a supply-chain and update
   control (it stops extensions.gnome.org), not a local-integrity boundary. Any process running as the user
   can still put code into gnome-shell: the session's `XDG_DATA_DIRS` begins with the user-writable
   `~/.local/share/flatpak/exports/share`, and `~/.config/environment.d` feeds the shell's environment. A
   root-owned install protects the Tiler's own files, not the session. The risks that matter most are the
   code that gets reviewed and installed, and bugs.
7. **What a malicious or compromised Tiler could do (INFERENCE):** read every keystroke and the screen,
   and patch the polkit agent and unlock dialog that run in the same process to capture the login
   password. From there it reaches sudo and so root. Review it at least as strictly as vase on macOS.

---

## 1. Facts about this desktop (VERIFIED, read-only)

| Item | Value | How checked |
|---|---|---|
| gnome-shell / mutter / gjs | 50.1-0ubuntu1.5 / 50.1-0ubuntu2.5 (libmutter-18) / gjs 1.88.0 | `dpkg -l` |
| Session | `ubuntu:GNOME`, Wayland; the shell runs as `org.gnome.Shell@ubuntu.service` (`ExecStart=/usr/bin/gnome-shell --mode=%i`) | `systemctl --user`, `/usr/lib/systemd/user/org.gnome.Shell@.service:3,20` |
| Crash path | `OnFailure=org.gnome.Shell-disable-extensions.service gnome-session-shutdown.target`; the disable unit runs `gsettings set org.gnome.shell disable-user-extensions true` only if `%t/gnome-shell-disable-extensions` exists | `/usr/lib/systemd/user/org.gnome.Shell-disable-extensions.service` |
| `allow-extension-installation` | `false` (set by Chained ticket 13) | `gsettings get` |
| `disable-user-extensions` | **`true`** | `gsettings get` |
| `enabled-extensions` | 3 user UUIDs (blur-my-shell, rounded-window-corners, just-perfection) and 5 system ones | `gsettings get` |
| Session mode `ubuntu` | `/usr/share/gnome-shell/modes/ubuntu.json`: `parentMode: user`, `enabledExtensions` includes `tiling-assistant@ubuntu.com`, `ubuntu-dock`, `ding`, `snapd-prompting`, ... | file |
| Shell's `XDG_DATA_DIRS` | `~/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:/usr/local/share/:/usr/share/:/var/lib/snapd/desktop` (the first entry is user-writable; the directory doesn't exist yet) | `/proc/<gnome-shell pid>/environ`; `/usr/share/gnome-session/sessions/ubuntu.session`; `60-flatpak` env generator |
| Extensions app | neither `org.gnome.Extensions.desktop` nor `com.mattjakeman.ExtensionManager.desktop` installed | `ls /usr/share/applications` |
| Devkit support | libmutter has the `--devkit` option and points at `/usr/libexec/mutter-devkit`; the viewer is in `mutter-dev-bin` 50.1-0ubuntu2.5 (universe), **not installed** (its package list was read from the downloaded .deb in scratch, not installed) | `strings libmutter-18.so.0.0.0`, `apt-get download` into `scratch/deb`, `dpkg -c` |
| Test tool | `/usr/bin/gnome-shell-test-tool` (50.1), `/usr/libexec/gnome-shell-perf-helper`, `/usr/bin/gnome-extensions` installed | `ls`, `dpkg -S` |
| Archive extras | `python3-dbusmock` 0.38.1, `mutter-18-tests`, `libmutter-test-18`, `eslint` 6.4 (too old for GNOME's flat config) available, none installed; `jasmine-gjs` not in the archive | `apt-cache policy` |
| podman | 5.7.0 (the host is shared: only `chained-*` names, never prune, small `/tmp` quota) | `dpkg -l`, memory |

---

## 2. How gnome-shell 50.1 loads and enables extensions

All VERIFIED in `js/ui/extensionSystem.js`, `js/misc/fileUtils.js` and `js/ui/sessionMode.js` at 50.1.

- **Where it looks.** `_loadExtensions()` calls `collectFromDatadirs('extensions', includeUserDir)` with
  `includeUserDir = allow-extension-installation` (`extensionSystem.js:711-712`). `collectFromDatadirs`
  walks `GLib.get_system_data_dirs()` (that is, `XDG_DATA_DIRS`) and puts `XDG_DATA_HOME` first only when
  `includeUserDir` is set (`fileUtils.js:19-22`). An extension directory **not** under
  `$XDG_DATA_HOME/gnome-shell` counts as `SYSTEM`, wherever it is (`extensionSystem.js:724-727`).
  The first directory found for a UUID wins; later copies are skipped with a log line (`:717-721`).
- **What gets enabled.** `_getEnabledExtensions()` starts with the session mode's `enabledExtensions`. It
  adds `enabled-extensions` **only if `disable-user-extensions` is false**, whatever type the extension
  is, and then removes anything in `disabled-extensions` (`extensionSystem.js:545-556`).
- **Updates and installs from extensions.gnome.org.** `updatesSupported` requires
  `allow-extension-installation` **and** an Extensions app (`extensionSystem.js:86-92`). The schema text
  says installation off makes `InstallRemoteExtension` fail and loads only system dirs, and that the change
  takes effect at the next login (`org.gnome.shell.gschema.xml.in:43-53`).
- **Session modes** are JSON files from the system data dirs (`sessionMode.js:134-136`). The first file
  for a name wins (`:109`). A mode can set `components`, `enabledExtensions`, `hasWindows` and so on
  (`:121-127`; `unlockDialog` can't be set from JSON). A child mode inherits from `parentMode`
  (`:192-198`). The built-in `user` mode starts the components
  `polkitAgent, keyring, autorunManager, automountManager, networkAgent` (`:13-19`).
- **Enable and disable** through `gnome-extensions` or D-Bus rewrite `enabled-extensions` and
  `disabled-extensions` in the user's settings (`extensionSystem.js:274-312`).
- **Crash marker.** At init, gnome-shell creates `$XDG_RUNTIME_DIR/gnome-shell-disable-extensions` and
  deletes it after 60 s or at shutdown (`extensionSystem.js:54-72`). The systemd unit in section 1 reads it.
- **Eval over D-Bus** (`org.gnome.Shell.Eval`) works only in unsafe mode (`shellDBus.js:70-74`). Putting a
  running shell into unsafe mode shows a "System was put in unsafe mode" notification (`main.js:291-305`).
  The `--unsafe-mode` option is hidden in mutter (`meta-context-main.c:397-401`).

---

## 3. Developing safely

### 3.1 What can go wrong on the owner's session

| Risk | Mechanism | Severity |
|---|---|---|
| Session lost | A segfault or abort in gnome-shell kills the compositor, and with it every Wayland client; the unit's `OnFailure` then shuts the session down | high |
| Real settings changed | A nested shell started with the default backend writes the user's real dconf (`~/.config/dconf/user`): `enabled-extensions`, welcome-dialog state, `monitors.xml` in `~/.config` | high (Chained doctor would report drift, and this could even re-enable things) |
| A `gnome-extensions` or `busctl --user` call aimed at the wrong bus | Outside the nested bus these calls reach the **real** shell (for example `gnome-extensions enable`, `tools/shell-lg.sh`'s `busctl call --user`) | medium |
| Real logind session affected | A nested shell on the host's system bus looks up `session/auto` when `XDG_SESSION_ID` is unset (`loginManager.js:141-163`), so it follows the real session's lock signals, and it creates a ScreenShield whenever logind can lock (`main.js:237-238`). utwm's harness refuses to run inside a login session for this reason (`tools/e2e/src/linux.rs:481-486`) | low to medium |
| Shared runtime dir | The nested shell creates and deletes the crash marker and a `wayland-N` socket in `/run/user/1000` | low (the marker matters only if the real shell crashes within 60 s of a nested start) |
| Ubuntu's mode extensions in the dev shell | Running `--mode=ubuntu` nested also loads `snapd-prompting@canonical.com`, which talks to snapd's system API, and `ding`, which spawns a helper over `~/Desktop`. Mode components register a polkit agent and keyring prompts | medium (use a custom mode, section 3.3) |
| Debug tools left enabled | `development-tools` is `true` (the default), so Alt+F2 `lg` gives any person at an unlocked session a JS REPL in the compositor | low; not a Chained key today (open question 6.6) |

### 3.2 Isolation levels, from safest to least safe

| Level | What runs | Isolation | Use for |
|---|---|---|---|
| L0 | Pure layout and keymap logic under plain `gjs -m`, with no `gi://Meta/St/Shell` imports | full; no shell at all | most tests, TDD |
| L1 | `gnome-shell --headless` in **rootless podman** (`chained-tiler-*`), with its own session and system bus (dbusmock for logind) | strong: no host sockets mounted | automated integration tests, locally and in CI |
| L2 | Headless on the host: `systemd-run --user --scope` + `env -i` + `dbus-run-session`, with a throwaway HOME and XDG dirs (0700) | good, but shares the host system bus | when podman is inconvenient (utwm's pattern) |
| L3 | **Devkit** (`dbus-run-session gnome-shell --devkit --wayland`) shown in a window on the real desktop | partial: shares the system bus, logind and the runtime dir; gjs.guide warns it "is not fully isolated" | interactive feel: keys, chrome, animations |
| L4 | The real session | none | only after the assessment, per section 5 |

### 3.3 Recipe for an interactive devkit session (L3)

The owner has to approve installing `mutter-dev-bin` (apt, universe, OS channel) first. Without it,
`--devkit` runs headless and only warns that the viewer can't start (`meta-mdk.c:145-152`).

The sketch below follows mutter's `doc/building-and-running.md:77-105`, gjs.guide's debugging page, and
the isolation that `gnome-shell-test-tool` and utwm use:

```sh
# one throwaway root per run, mode 0700, never under /tmp (small per-user quota)
R=$(mktemp -d -p "$HOME/.cache" tiler-dev.XXXXXX); chmod 700 "$R"
mkdir -p "$R"/{home,config,data,state,cache} "$R/share/gnome-shell/"{extensions,modes}
ln -s ~/src/vase-chained/vase-linux/extension "$R/share/gnome-shell/extensions/<uuid>"  # path: whatever the track picks
cat > "$R/share/gnome-shell/modes/tiler-dev.json" <<'EOF'
{ "hasWindows": true, "hasWorkspaces": true, "hasOverview": true, "hasRunDialog": true,
  "hasNotifications": true, "hasWmMenus": true, "components": [],
  "enabledExtensions": ["<uuid>"],
  "panel": { "left": ["activities"], "center": ["dateMenu"], "right": ["quickSettings"] } }
EOF
env HOME="$R/home" XDG_CONFIG_HOME="$R/config" XDG_DATA_HOME="$R/data" \
    XDG_STATE_HOME="$R/state" XDG_CACHE_HOME="$R/cache" GSETTINGS_BACKEND=keyfile \
    XDG_DATA_DIRS="$R/share:/usr/local/share:/usr/share" \
    SHELL_DEBUG=backtrace-warnings,backtrace-segfaults G_MESSAGES_DEBUG='GNOME Shell' \
  dbus-run-session -- gnome-shell --devkit --wayland --mode=tiler-dev 2>&1 | tee "$R/shell.log"
```

Why each part is there:
- `GSETTINGS_BACKEND=keyfile` and a fresh `XDG_CONFIG_HOME`: every settings write goes to
  `$R/config/glib-2.0/settings/keyfile`. This is how `gnome-shell-test-tool` isolates itself
  (`/usr/bin/gnome-shell-test-tool:44-58`). The real `allow-extension-installation=false` and
  `disable-user-extensions=true` aren't read at all, so the dev shell has nothing to override.
- The extension sits in a **system** data dir that the run itself provides, and is enabled by a **mode**
  that lists it. That means no dconf key, no `gnome-extensions install`, no copy into `~/.local`, and
  loading straight from the working tree (the symlink). A dev mode built on `restrictive` (no
  `parentMode`) has **no components** (no polkit agent, keyring, automount or network agent) and no unlock
  dialog. Leaving `/usr/share` in `XDG_DATA_DIRS` keeps the stock schemas and themes. Leaving out the
  flatpak and snap entries keeps any stray user-writable modes or extensions out (section 5.5).
- `dbus-run-session`: the shell's services and Eval live on a private bus. To poke at it, attach a second
  shell to the same bus (mutter's `dbus-session.sh new|attach`, `building-and-running.md:107-145`) and run
  `gdbus`, `gnome-extensions info <uuid>` or `busctl --user` from there. Never run them from a normal terminal.
- Keep the host `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY` **for the devkit only**. Mutter launches the viewer
  with the host's `WAYLAND_DISPLAY` (`meta-mdk.c:119-143`), and the viewer receives frames through
  screencast and remote desktop (`meta-mdk.c:114-117`), so moving the runtime dir would break it
  (UNVERIFIED in detail). Add `--no-x11` only when X11 clients aren't being tested; games under
  Steam/Proton are Xwayland clients, so the Tiler has to be tested with Xwayland too.
- Keys: mutter's doc warns that "keyboard shortcuts usually [don't get] to the nested compositor"
  (`building-and-running.md:149`), because the host grabs them first. Alt+A and Alt+E aren't GNOME
  defaults, so they should get through. Super-based combinations and Tiling Assistant's keys will not.
  When a key test matters, use L1 or L2 and inject keys (section 4.3).
- Iterate by quitting and restarting the devkit shell. There is no in-place reload: "JavaScript engines
  can not 'unload' code", and Wayland can't restart the shell (gjs.guide debugging; the GNOME 50 porting
  guide notes the restart command is gone along with X11).

### 3.4 Logs and debugging

- Nested and headless shells log to their stderr (the `tee` above). `SHELL_DEBUG=backtrace-warnings`
  prints the JS stack for `console.warn` and `console.error`; `backtrace-segfaults` prints it on a crash
  (gjs.guide debugging). `G_DEBUG=fatal-criticals` together with
  `dbus-run-session -- gdb --args gnome-shell --devkit --wayland` breaks on the first critical, and
  `call (void)gjs_dumpstack()` prints the JS stack (gjs.guide debugging).
- Looking Glass: Alt+F2 `lg` **inside** the devkit window (the dev mode sets `hasRunDialog`). It has an
  evaluator, Windows, Extensions (with each extension's errors), Actors and Flags pages.
- Scripted inspection: run the dev shell with `--unsafe-mode`, then use Eval over the **nested** bus
  (`tools/shell-lg.sh` in gnome-shell does this with `busctl call --user ... Eval`). Never put the real
  shell into unsafe mode.
- On the real session, after shipping: `journalctl --user -b -u org.gnome.Shell@ubuntu.service`, and
  extension errors in `gnome-extensions info <uuid>` (state ERROR). Log under a fixed prefix and keep the
  volume low, as gjs.guide asks.

### 3.5 Never

- Never install into `~/.local/share/gnome-shell/extensions` or run `gnome-extensions enable` on the real
  session during development.
- Never run a second gnome-shell on the real session bus (always use `dbus-run-session`).
- Never mount `/run/user/1000`, the host D-Bus or the host Wayland socket into a test container.
- Never use a self-hosted GitHub runner on the desktop: it would run pull-request code in the owner's session.

---

## 4. Automated testing

### 4.1 The seam

Mirror vase's own boundary. vase-core is platform-agnostic, and a `Backend` trait "covers everything vase
asks of the OS" (`vase-chained/CONTEXT.md`, Architecture). In the extension, keep the layout tree,
key router and registry in modules that import nothing from `gi://Meta`, `gi://St`, `gi://Shell` or
`resource:///org/gnome/shell/...`. Those typelibs and resources exist only inside gnome-shell. The Meta
typelib lives in `/usr/lib/x86_64-linux-gnu/mutter-18/`, outside the default GI path. All OS calls go
through a JS `Backend` object, with a Meta-backed implementation in the shell and a fake in tests. Then
most behaviour can be tested at L0 with no compositor at all.

### 4.2 Unit tests (L0)

- `gjs -m tests/run.js` with a small TAP harness written in the repo, which adds no dependency. gnome-shell's
  own unit tests use jasmine-gjs as a meson subproject, with `--module --tap` (`tests/meson.build:25-74`).
  jasmine-gjs isn't in Ubuntu's archive, so vendor it at a pinned commit if it's wanted.
- These tests run anywhere `gjs` is installed: in the Chained-style podman container, and on a plain
  `ubuntu:26.04` CI container after `apt-get install gjs`.
- Optional: parity fixtures. vase-core's Rust tests emit JSON (a sequence of commands and the expected
  layout tree), and the JS model replays them. This keeps the two layout engines from drifting while the
  Linux side is a separate program (ADR-0001).

### 4.3 Shell integration tests (L1 and L2)

- **Vehicle:** `gnome-shell-test-tool --headless --extension tiler.zip tests/shell/<case>.js`. The GNOME 50
  porting guide documents exactly this command. The tool:
  - builds `$tmp/{config,data,cache}` and sets `GSETTINGS_BACKEND=keyfile` (`gnome-shell-test-tool:44-58`)
  - installs the zip with `gnome-extensions install --print-uuid` into that throwaway `XDG_DATA_HOME`,
    and enables it in the keyfile (`:29-38`, `:66-68`)
  - starts `gnome-shell --automation-script <case.js> --headless --virtual-monitor 1280x720 --wayland-display gnome-shell-test-display` (`:76-95`)

  In the throwaway keyfile `allow-extension-installation` keeps its default (true), so the user-dir install
  works there and the real desktop isn't affected.
- **A test is a module** with `export async function run()`. It runs inside the shell with `Main` and
  `global` (see `tests/shell/basic.js`). A thrown error calls `Meta.exit(Meta.ExitCode.ERROR)`, so the
  tool exits non-zero (`js/ui/scripting.js:291-297`). Test windows come from `Scripting.createTestWindow()`
  and `waitTestWindows()`, through `gnome-shell-perf-helper`, a GTK client (`scripting.js:102-141`).
- **Window filter.** The tool sets `MUTTER_WM_CLASS_FILTER` so that only the perf helper's windows appear
  (`:48-49`). Add `--extra-filter <class>` for any other client a test opens, such as an X11 test client
  through Xwayland.
- **Keys:** inject Alt+A and the other chords from inside the test with a Clutter virtual keyboard
  (`seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE)`) so they go through mutter's
  normal keybinding path. utwm instead drives keys from outside through Mutter's RemoteDesktop D-Bus API,
  by evdev keycode, so the result doesn't depend on the keyboard layout (`utwm tools/e2e/src/linux.rs:7-9`).
  That also suits tests where the Tiler's keymap is the thing under test (UNVERIFIED for virtual devices
  reaching `Main.wm.addKeybinding` handlers; check it in the prototype).
- **Several monitors:** the tool hardcodes one 1280x720 monitor. The track's own runner can pass
  `--virtual-monitor` twice, as utwm does (`linux.rs:508-516`), which vase's per-screen tabs need.
- **What to assert:** frames of managed windows after each command, tab and stack occlusion, that
  `disable()` leaves no actors, signals or sources behind (an e.g.o rule, and a cheap leak test: enable,
  disable and enable again in one run), Pause, ignoring fullscreen windows and games, and that the
  extension survives monitor hotplug (`--hotplug`, as in `tests/shell/headlessStart.js`).
- **The bus around the shell:** gnome-shell's own CI runs these tests in plain containers with
  `XDG_RUNTIME_DIR` inside the project dir and `dbus-run-session -- meson test`
  (`.gitlab-ci.yml:190-204`). Its runner mocks the system services (DisplayManager, logind-adjacent
  services, GeoClue, the portal PermissionStore and others) with python-dbusmock
  (`tests/gnomeshell_dbusrunner.py`). Borrow that file, or a trimmed copy, so the container has a system
  bus with mocks instead of the host's.
- **Mutter's test framework** (`libmutter-test-18`, `MetaContextTest`, C, `src/tests/`) is for testing
  mutter itself. It isn't a good fit for a JS extension. Only its `mutter_dbusrunner`, which gnome-shell's
  runner imports, is worth reusing.

### 4.4 Static checks

- ESLint with gnome-shell's config (`tools/eslint.config.js` at 50.1), in CI only. Install it with
  `npm ci --ignore-scripts` from a committed lockfile; nothing from npm on the desktop. Ubuntu's eslint 6.4
  is too old.
- `shexli`, the extensions.gnome.org static analyser. Tiling Assistant's CI installs it from an unpinned
  git branch (`run-ci.yml`, job `shexli`); pin it to a full commit instead.
- A metadata lint: `uuid` under the owner's domain, `shell-version` exactly the supported majors (no future
  versions), no `session-modes` key (the Tiler must not run on the lock screen), and a
  `settings-schema` under `org.gnome.shell.extensions.*` if GSettings is used (review guidelines).
  `getSettings()` loads `schemas/` with `SettingsSchemaSource.new_from_directory`, so
  `gschemas.compiled` must be built (`js/extensions/sharedInternals.js:92-111`).
- **Tripwires** like the vase fork's CI conditions (spec: "tripwires for build-script changes, process
  spawning and file writes"). A grep job fails on any new match outside a committed allow-list:
  - processes: `Gio.Subprocess`, `GLib.spawn`, `Util.spawn`, `Meta.WaylandClient.new_subprocess`
  - network: `Soup`, `Gio.SocketClient`, `Gio.Resolver`, `GLib.Uri` with http
  - D-Bus: `own_name`, `DBusExportedObject`, `Gio.DBus.*call`
  - file writes: `replace_contents`, `create`, `GLib.file_set_contents`
  - clipboard, screen and dynamic code: `St.Clipboard`, `Shell.Screenshot`, `eval`, `new Function`,
    dynamic `import(`
  - environment: `GLib.setenv`
  - monkey-patching (`InjectionManager`, `.prototype.` assignments) of
    `components/polkitAgent`, `unlockDialog`, `keyring`, `loginDialog`, `shellDBus`

### 4.5 Local podman (shared host)

- An image `chained-tiler-test` from `ubuntu:26.04` by digest, the same base image policy as Chained's
  container suite. Install `gnome-shell gjs dbus python3-dbusmock mesa-libgallium` with
  `--no-install-recommends` (package names UNVERIFIED beyond `gnome-shell`, `gjs`, `dbus` and
  `python3-dbusmock`).
- Headless needs no GPU: the headless backend uses software rendering, as gnome-shell's CI shows. Mount
  nothing from the host except the checkout, read-only. Keep scratch on a volume or `--tmpfs`, not the
  host `/tmp`. Name containers `chained-tiler-*`, remove them with `--rm`, and never prune.

### 4.6 GitHub Actions (in `vase-chained`)

- GitHub's `ubuntu-latest` runner ships an older GNOME, so run the job in
  `container: ubuntu:26.04@sha256:<digest>`. Optionally pin apt to a `snapshot.ubuntu.com` date
  (UNVERIFIED that apt's `--snapshot` works in that image) so CI tests the same 50.x the desktop runs.
  Bump both through Upgrade, like the other CI pins (spec story 58).
- Jobs: `unit` (gjs), `lint` (eslint, shexli, metadata lint, tripwires), `shell-headless` (test tool +
  dbusmock runner, two virtual monitors), and `pack` (`gnome-extensions pack`, and checking that the zip
  matches `git archive` of the extension directory at the commit).
- Hardening as the spec lists for the fork: SHA-pinned actions, top-level `permissions: {}` with
  `contents: read` per job, Dependabot alerts, the inherited release workflows deleted (they're still
  present: `release.yaml`, `release-build.yaml`, `release-finalize.yaml` in
  `~/src/vase-chained/.github/workflows`), and no secrets in test jobs. The extension needs **no release
  artifact**: it is source, so Chained installs it from the reviewed checkout (section 5.3).

### 4.7 What stays manual (spec story 119's checklist)

After the first real install and after every bump, by hand: log out and in, check that
`gnome-extensions info <uuid>` shows ACTIVE, that Tiling Assistant is disabled, the feel of the Alt+A and
Alt+E prefixes, that Pause works with a game running, that suspend, resume and lock behave, that monitor
hotplug works, and that the recovery runbook (section 5.7) works once from a TTY.

---

## 5. Security

### 5.1 What the extension can do

An enabled extension "effectively becomes a part of GNOME Shell" and can "access and modify any internal
GNOME Shell code" (gjs.guide architecture, cited in the audit as [577]). Concretely (INFERENCE from the
APIs available in-process):
- It sees every input event before any app does, and every window's contents. On Wayland the compositor
  is the thing that isolates apps from one another, so a compromised extension cancels that isolation.
- It shares the process with the **polkit authentication agent** (a `user` mode component,
  `sessionMode.js:13-16`), the keyring prompts and the unlock dialog. It can patch them and capture the
  login password, which is the sudo password on this desktop. So the Tiler's integrity is equivalent to root.
- It runs as the user with full session-bus, filesystem, process-spawning and network access. GNOME
  imposes no sandbox. The rules on extensions.gnome.org are review policy only, not enforcement.

This is more power than vase has on macOS (Accessibility and Input Monitoring, with TCC prompts) or
Windows (low-level hooks).

### 5.2 What Chained requires before the Linux Tiler ships

| Requirement | Source | How to meet it | Status |
|---|---|---|---|
| Its own assessment before it enters the Standard | audit §24 verdict; open decision 14; spec "Ubuntu: keeps Tiling Assistant until the Linux Tiler exists and passes its own assessment", story 107 | A new section 24 assessment over the prototype's code, with an `[assessment.*]` entry in `data/assessments.toml` | not started |
| Source in the owner's repo at a reviewed full SHA, under the source-build rules (detached checkout, ancestry check, full diff read, offline build, output hash recorded) | §24.1, policy rule 1 | The "build" is only `glib-compile-schemas` and, if used, `gnome-extensions pack`. Record the installed tree's hash in the bootstrap lock | needs the contradiction below resolved |
| Never from or updated by extensions.gnome.org; `allow-extension-installation` false; a UUID under a domain the owner controls | §24.2; audit table rows 38, 67 | Keep the key false (doctor already checks it). UUID like `tiler@<owner-domain>`. The real protection is the key, not the UUID (INFERENCE: e.g.o doesn't appear to verify domains; the guidelines allow `username.github.io`) | key done (ticket 13) |
| No network, no subprocesses except fixed documented ones, no telemetry, no minified code | §24.3, review guidelines | The tripwires (section 4.4). If vase-chained's config (TOML) is read, parse it in plain reviewed JS, not with an npm parser | – |
| Full code review before the first enable, and a diff review at every bump | §24.4 | Same as the vase baseline review. The e.g.o rule "must not be AI-generated ... developers should be able to justify and explain the code" is a good bar for agent-written code | – |
| Pause disables the extension (`gnome-extensions disable <uuid>`) | §24.5 | **Conflicts** with Chained's "never writes an extension key" stance and its doctor checks, because `disable` rewrites `enabled-extensions` and `disabled-extensions` in the real dconf (`extensionSystem.js:294-312`). See 5.7 | decision needed |
| The fork's hardening (signed protected `main`, Dependabot, SHA-pinned actions, release workflows deleted, read-only token) and fork CI (tripwires, lint, tests) | spec, vase section, ~line 865 | Extend fork CI with section 4.6. Agent instruction files are prompt-injection surface (spec) | release workflows still present |
| Rule 4: tools with powerful OS access carry a written justification and extra review | ADR-0005 rule 4 | The written justification is section 5.1 of the assessment | – |
| One update path | audit policy rule 4 | Only Chained's Upgrade moves the reviewed SHA; no Extensions app; installation off | – |
| Licence | review guidelines (gnome-shell is GPL-2.0-or-later); ADR-0001 (vase is GPL-3.0) | GPL-3.0-or-later is compatible | ok |

### 5.3 Install channel: the options

The contradiction first. §24.1 implies a home-directory install ("installed by chezmoi"). §24.2 keeps
`allow-extension-installation` false, which stops gnome-shell from even scanning the home directory
(section 2). On top of that, `disable-user-extensions=true` on this desktop means `enabled-extensions` is
ignored. Chained's GNOME document lint refuses `enabled-extensions`, `disabled-extensions`,
`disable-user-extensions` and `disable-extension-version-validation` (ticket 13: `docs/tools/gnome.md:26`,
`data/gnome.toml:38-40` on the stacked branch in `.claude/worktrees/14-retire`).

| Option | Loads with installation off? | How it gets enabled | Integrity of the Tiler's files | Notes |
|---|---|---|---|---|
| A. chezmoi into `~/.local/share/gnome-shell/extensions/<uuid>` | **no** | `enabled-extensions` + `disable-user-extensions=false` | user-writable | Needs installation **on**, which reopens `InstallRemoteExtension` and the e.g.o update check as soon as any Extensions app gets installed (`extensionSystem.js:86-92`). Against §24.2 |
| **B. Root-owned copy in `/usr/local/share/gnome-shell/extensions/<uuid>`** (already in `XDG_DATA_DIRS`) by Chained's sudo step, or a locally built `.deb` into `/usr/share` | yes (SYSTEM type) | `disable-user-extensions=false` + the UUID in `enabled-extensions`. With installation off, the three stale user UUIDs still won't load, because the user dir isn't scanned | root-owned; changes only through Chained's sudo step | Root at install time only; the extension runs as the user. Doctor compares the installed tree with the hash in the bootstrap lock. A `.deb` adds dpkg tracking and clean removal, at the cost of packaging work |
| C. A Chained-owned user dir added to `XDG_DATA_DIRS` through `~/.config/environment.d` | yes (SYSTEM type, since it isn't under `XDG_DATA_HOME`) | as in B | user-writable | No root, but it quietly sidesteps what `allow-extension-installation` means, and changes every user service's environment. UNVERIFIED that environment.d reaches the shell unit in the right order (the flatpak generator's entries do reach it, section 1) |
| D. A session-mode file shadowing `ubuntu.json` (`/usr/local/share/gnome-shell/modes/ubuntu.json` wins because it comes earlier) | yes | the mode's `enabledExtensions`, ignoring every key | root-owned | **Reject.** The crash safety net only sets `disable-user-extensions`, which doesn't affect mode extensions, so a crashing Tiler would crash-loop every login. It also forks Ubuntu's mode file across upgrades |
| E. extensions.gnome.org | – | – | – | Ruled out by the audit (row 67) |

**Recommendation: B.** Chained's lint then allows exactly:
- `disable-user-extensions = false`
- `enabled-extensions` containing the Tiler UUID (doctor asserts that no other UUID in the list is
  loadable from a non-system directory)
- at the switch-over, `tiling-assistant@ubuntu.com` in `disabled-extensions` (it's a mode extension, and
  `disabled-extensions` wins, `extensionSystem.js:553-555`)

Each install or bump needs a log-out on Wayland (also recorded in `docs/research/cross-platform-dotfiles.md:556`).
The Bootstrap and Sync guided step already exists for the GNOME log-out.

Bump safety: `shell-version` gates loading, so an Ubuntu upgrade to GNOME 51 makes the Tiler silently
fail to load until it's bumped. That fails safe, but it leaves no tiling once Tiling Assistant is off.
Doctor should FAIL when the Tiler is enabled but not ACTIVE, and the switch-over should keep a one-step
way back (remove Tiling Assistant from `disabled-extensions`).

### 5.4 How the two keys behave together

- `allow-extension-installation=false`: no user-dir scan, no remote install, no update check (section 2).
  It takes full effect at the next login.
- `disable-user-extensions=true`: only the mode's extensions run, whatever the type. The crash unit sets
  it to true whenever the shell dies within about 60 s of startup (section 1). **With option B the Tiler
  is a non-mode extension, so this safety net covers it**: if the Tiler crashes the shell at login, the
  next login comes up without it. Doctor should report `disable-user-extensions=true` while the Tiler is
  in the Standard as "the Tiler was disabled after a crash".

### 5.5 What local integrity really means here (INFERENCE, from verified facts)

- The shell's `XDG_DATA_DIRS` begins with `~/.local/share/flatpak/exports/share`, which is user-writable
  (it doesn't exist yet, but any user process can create it). gnome-shell treats it as a system data dir
  for both `extensions` and `modes`, and it is searched first. So any same-user process can drop
  `gnome-shell/modes/ubuntu.json` there, listing its own extension, and be loaded into the compositor at
  the next login, with `allow-extension-installation=false` and `disable-user-extensions=true` both set.
  `~/.config/environment.d` (for example `LD_PRELOAD` for user units) is a second route.
- So `allow-extension-installation=false` controls the e.g.o channel, and a root-owned install protects
  the Tiler's own files. Neither stops same-user malware from reaching the compositor. Write this down in
  the assessment as an accepted platform limit. (Whether a Flatpak sandbox with `--filesystem=home` can
  write `~/.local/share/flatpak` is UNVERIFIED.)
- A cheap doctor check: FAIL if `gnome-shell/extensions` or `gnome-shell/modes` exists under any
  user-writable `XDG_DATA_DIRS` entry of the running shell, or if `~/.config/environment.d` sets
  `LD_PRELOAD`, `XDG_DATA_DIRS` or `GNOME_SHELL_*`.

### 5.6 Supply chain

- **Runtime dependencies: none.** Only `gi://` and gnome-shell's own resources. No npm packages, no
  bundler, no TypeScript (transpiled output would have to be reviewed as well, and the transpiler is one
  more dependency). If types are wanted, use JSDoc with `// @ts-check` in CI only.
- **Development dependencies are CI-only and pinned:** eslint through a committed lockfile with
  `--ignore-scripts`, shexli at a commit, and jasmine-gjs (if used) vendored at a commit. Nothing from
  npm or pip on the desktop.
- **No binaries and no WASM in the extension** (review guidelines: "MUST NOT include binary executables or
  libraries"). If the track decides to keep vase-core in Rust behind a thin extension (utwm's design), that
  is a second program with its own conditions, and the extension's D-Bus interface becomes attack
  surface. Any session-bus client can call it. Gate it the way gnome-shell gates its own privileged
  methods, with a `DBusSenderChecker` allow-list of well-known names (`js/misc/util.js:344-390`,
  used throughout `shellDBus.js`), or utwm's single-client lock (`gnome-extension/extension.js:315-333`).
  Still, an allow-list of well-known names doesn't stop a same-user process from claiming the name first.
- **Upstream merges** from CasperTeirlinck/vase follow the fork's rules (a PR, a full diff, at most
  monthly). The extension is new code with no upstream yet, so the first-enable review covers all of it.
- **Install artifact = the reviewed tree.** Chained copies `git archive <sha> -- <ext-dir>` plus the
  compiled schema, with no CI-built zip. That makes the trust chain the same as the existing source-build
  rule, with nothing extra to attest.

### 5.7 Runtime safety: Pause, the kill chord and recovery

- **Pause.** Prefer an in-extension Pause: the chord drops every key grab and window signal except the
  resume chord, and restores the windows' pre-adoption frames from the registry. `gnome-extensions disable`
  (audit §24.5) also works, but it writes the real `enabled-extensions` and `disabled-extensions`, which
  breaks Chained's extension-key rule and shows up as doctor drift. Re-enabling then re-runs `enable()` on
  the same module (no code reload). Owner decision (open question 6.3).
- **The kill chord** (spec: "The kill chord is documented"). On Linux it can't kill a process, because the
  Tiler is the compositor. Define it as "unmanage everything now and go inert until the next login",
  handled first in the key path so it still works when the layout code is wedged in a bad state.
- **Recovery runbook** (test once by hand): Ctrl+Alt+F3, log in, then
  `systemctl --user` to confirm the user manager is up, then either
  `gsettings set org.gnome.shell disable-user-extensions true` (with `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$UID/bus`),
  or `sudo mv /usr/local/share/gnome-shell/extensions/<uuid> /root/`, then log in again on the GDM screen.
  On option B, the second command always works even when dconf is unreachable.
- **No `session-modes` key:** the Tiler doesn't run on the lock screen, so all its key handling stops while
  locked (the guidelines' rule for `unlock-dialog`).

---

## 6. Open questions for the owner

1. **The install channel and the extension-key rule** (section 5.3): accept option B and loosen ticket
   13's lint to the three named writes? Or keep the rule and choose option C (no root, user-writable)?
2. **Fixing §24.1:** change "installed by chezmoi" to "installed by Chained's sudo step from the reviewed
   tree" (option B), in the audit and the spec.
3. **Pause:** in-extension Pause (recommended) or `gnome-extensions disable` as §24.5 says?
4. **Install `mutter-dev-bin`** on the desktop for interactive devkit work (an apt universe package; it
   isn't needed for any automated test)? `python3-dbusmock` is needed only in containers, not on the host.
5. **CI apt pinning:** pin the CI image to an Ubuntu snapshot date, moved by Upgrade, or follow the live
   archive?
6. **`development-tools`:** set it to `false` on the real session once the Tiler ships (it isn't an
   extension key; it turns off `lg` on the real session)? Also, add the doctor check in section 5.5?
7. **Architecture** (other lenses cover this): a pure GJS port of vase-core, or a Rust daemon plus a thin
   extension? The second adds a D-Bus surface and a second program to assess (section 5.6).

---

## 7. Sources

gnome-shell 50.1 (https://gitlab.gnome.org/GNOME/gnome-shell/-/tree/50.1, fetched through the github.com/GNOME/gnome-shell mirror):
- `js/ui/extensionSystem.js`: 54-72 (crash marker), 86-92 (`updatesSupported`), 274-312 (enable and disable write settings), 520-556 (mode and enabled lists), 700-748 (loading, `includeUserDir`)
- `js/misc/fileUtils.js:19-22`
- `js/ui/sessionMode.js`: 13-19, 21-103, 104-136, 192-208
- `js/ui/shellDBus.js:70-90` (Eval needs unsafe mode)
- `js/ui/main.js`: 237-238 (ScreenShield), 291-305 (unsafe-mode notice), 333-379 (automation script)
- `js/ui/scripting.js`: 102-141, 291-310
- `js/misc/loginManager.js:141-168`
- `js/misc/util.js:344-390` (`DBusSenderChecker`)
- `js/extensions/sharedInternals.js:92-111`
- `data/org.gnome.shell.gschema.xml.in`: 4-62
- `src/main.c` (options `--mode`, `--automation-script`)
- `src/gnome-shell-test-tool.in` (installed as `/usr/bin/gnome-shell-test-tool`: 29-38, 44-95, 248-258)
- `tests/meson.build`: 25-128
- `tests/gnomeshell_dbusrunner.py`
- `tests/shell/basic.js`
- `tools/shell-lg.sh`
- `.gitlab-ci.yml:190-211`

mutter 50.1 (https://gitlab.gnome.org/GNOME/mutter/-/tree/50.1):
- `src/core/meta-context-main.c`: 85-95, 198-241, 263-276, 352-401 (no `--nested`; `--devkit`, `--headless`, `--virtual-monitor`, hidden `--unsafe-mode`)
- `src/core/meta-mdk.c:104-160`
- `doc/building-and-running.md`: 54-176
- `mdk/data/org.gnome.mutter.devkit.gschema.xml.in`

gjs.guide:
- https://gjs.guide/extensions/development/debugging.html (nested via devkit on 49+, logging, Looking Glass, gdb)
- https://gjs.guide/extensions/review-guidelines/review-guidelines.html
- https://gjs.guide/extensions/overview/architecture.html
- https://gjs.guide/extensions/upgrading/gnome-shell-50.html (test-tool `--extension`, `gnome-extensions install --print-uuid`, X11 removed)
- https://gjs.guide/extensions/upgrading/gnome-shell-49.html (`--nested` gone, use `--devkit`)

Prior art (clones in `scratch/repos/`):
- johncarmack1984/universal-tiling-window-manager @ `58fdeecd`: `tools/e2e/src/linux.rs` 1-17, 342-560 (systemd-run scope, `env -i`, dbus-run-session, keyfile settings, 0700 runtime dir, refusing to run inside a login session, two virtual monitors); `crates/utwm-linux/gnome-extension/extension.js` 202-211, 315-333
- Leleat/Tiling-Assistant `.github/workflows/run-ci.yml` (eslint, shellcheck, shexli)
- forge-ext/forge `.github/workflows/testing.yml`
- gTile `.github/workflows/pr-check.yml` (node unit tests only)

This desktop (read-only): `/usr/lib/systemd/user/org.gnome.Shell@.service`, `/usr/lib/systemd/user/org.gnome.Shell-disable-extensions.service`, `/usr/share/gnome-shell/modes/ubuntu.json`, `/usr/share/gnome-session/sessions/ubuntu.session`, `/usr/lib/systemd/user-environment-generators/60-flatpak`, `gsettings get org.gnome.shell *`, `dpkg -l`, `apt-cache policy`.

Chained:
- `docs/adr/0001-tiler-is-a-fork-of-vase.md`, `docs/adr/0005-tool-security-policy.md`
- `.scratch/chained-v1/spec.md`: 107, 119, 638-654, 865-878, 952, 1039, 1056
- `docs/research/security-audit.md`: §14 (666-696), §24 (954-965), open decision 14 (1016), policy rule 1 (79), rows 26, 38, 67
- ticket 13 on the stacked branch: `.claude/worktrees/14-retire/docs/tools/gnome.md:18-26`, `data/gnome.toml:28-40`, `docs/research/tickets/13-facts.md` G3-G6

vase-chained: `CLAUDE.md`, `CONTEXT.md` (Architecture: Backend seam, "Linux is deferred, and Wayland is the known hard case"), `.github/workflows/` (the release workflows are still present), HEAD `022340fe`.
