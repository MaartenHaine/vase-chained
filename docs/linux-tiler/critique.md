# Critique of the Linux Tiler wayfinder (skeptical review)

Reviewed 2026-11-07: `wayfinder.md` and the five notes (`vase-model.md`, `gnome-api.md`, `prior-art.md`,
`architecture.md`, `devtest-security.md`). Research only. I changed nothing on the desktop, in the fork or in Chained.
Everything I read on the desktop was read-only (`gsettings get`, `/proc`, `loginctl show-*`, `apt-get -s`), plus one
`unshare` probe that changes nothing. I fetched fresh copies of the primary sources into `scratch/critique/`:
gnome-shell and mutter at tag `50.1` from gitlab.gnome.org, and `man systemd.exec` (systemd 259).

Labels: **VERIFIED** means checked in source or on the desktop. **INFERENCE** means reasoned from verified facts.
**UNVERIFIED** means it needs a spike.

The overall direction holds up: a thin GJS bridge plus a Rust daemon, with the prefix armed inside the shell and a stage
grab after it. The problems are in the details the plan treats as solved: the lifecycle, the input contract, the
sandbox, the dev isolation and the milestone order.

---

## Issues, ranked by importance

### 1. The daemon has no "extension gone" state, so lock and unlock are not free (HIGH)

**Claim** (wayfinder §3, point 5): "Lock and unlock are free. The daemon outlives the extension's `disable()` and
`enable()`."

**What's wrong**
- At every lock, `disable()` drops the socket (F7, VERIFIED: `_sessionUpdated` → `_onEnabledExtensionsChanged`,
  `extensionSystem.js:782-786` @50.1).
- The daemon still runs vase's 100 ms `reconcile()` against the mirror. That reconcile forgets closed windows and adopts
  new ones (`daemon/reconcile.rs:128-225`). If the mirror is emptied or goes stale while disconnected, the layout
  collapses, and the next snapshot re-adopts every window as a new tab.
- Locks are not the only trigger. Disabling or enabling any extension earlier in the load order also disables and
  re-enables every later one (VERIFIED, `extensionSystem.js:195-240`), so the switch-over itself cycles the Tiler.
- A clean stop runs `restore()`, which moves every window back to its pre-adoption frame (`daemon/effects.rs:112-124`).
  That happens on `systemctl --user restart` at an upgrade and on SIGTERM at log-out. The wayfinder drops the "detach"
  exit that `architecture.md` §1(b) asked for.
- A daemon that outlives the session (the user manager lingers while another login exists) would meet a new
  gnome-shell with a stale model. Window ids then start from a fresh random base (`meta_display_generate_window_id`,
  `display.c:3182-3191`).

**Fix**
- Add to `vase-gnome` a `Disconnected` state that freezes reconcile, ticks and effects.
- On reconnect, re-run `state::restore` against the new snapshot.
- Make the protocol hello carry a compositor-instance id (the shell's pid plus its start time). A new instance means a
  full re-init.
- SIGTERM means save and detach, never `restore()`. Only quit and the kill chord restore.
- Bind the unit to `graphical-session.target` (`PartOf=`/`BindsTo=`).
- M2's L1 test should lock twice, toggle another extension, and restart the unit. After each, assert the layout is
  unchanged and no window moved.

### 2. "vase-core runs unchanged" is false (HIGH, planning)

**Claim** (§3 and Costs): one planned core change, the primary-display flag (M7).

**What the plan itself needs in the core**
- **Pause as a core state** (Q3.4).
- **Fullscreen windows unmanaged.** Destination item 5 asks for this. Today vase keeps them managed and only hides the
  chrome (`vase-model.md` §2, `reconcile.rs:180-188`).
- **The game ignore list** (Q4.1). There is no rules engine today.
- **A primary display** instead of the display at (0,0) (`reconcile.rs:61`).
- **A way to disarm the router.** `KeyRouter` has no timeout and no public disarm (`input/mod.rs:149-185`), but the
  extension ends the prefix on a 3 s timeout, on a revoked grab or on EOF (Q2.3). The router stays armed unless the core
  gains `disarm()`.
- **A fuller `modal()`.** It leaves out the sticky resize and move-tab modes (`daemon/input.rs:15-17`), so a grab-based
  host can't know to keep the grab.
- **Re-injection or a decision for pass-through and `SendPrefix`**, which is a no-op today (`daemon/input.rs:85`).
- **The detach exit** (issue 1). This one can live in `vase-gnome`.

**Fix.** Add a "core seams" table to the wayfinder: each seam, the milestone that needs it, and whether it is offered
upstream or carried as a fork patch. Do the input-contract seams (disarm, the full `modal()`, a key-ownership query) in
M1, not later. Re-state the "unchanged core" benefit honestly as "shared core plus about six seams".

### 3. The key-ownership model under a grab is incomplete (HIGH)

vase assumes a hook that sees every key and passes or swallows it on the spot (`vase-model.md` §3c). Under a stage grab
the extension owns every key until it pops. These cases are missing or wrong:

- **Sticky repeat** (bare Shift+arrows after a resize): it isn't in `modal()`. Either it silently doesn't work on Linux,
  or a held grab eats every other key.
- **Pending launch** is modal with "only Esc" for up to `LAUNCH_ADOPT_TICKS = 100`, about 10 s
  (`pane_picker/keys.rs:16`, `daemon/input.rs:26-32`). A stage grab also takes the **pointer**: the focus surface is NULL
  under a grab (VERIFIED, `meta-wayland-input.c:117-131`). So the desktop is unusable for up to 10 s if the app never
  maps a window.
- **"The grab holds back fast typing, so nothing reaches the app"** (§3) means those keys are lost, not delayed. Keys
  queued between the command key and the daemon's modal reply need a rule: replay or drop.
- **The 3 s prefix timeout** is a behaviour vase doesn't have. It belongs in the Keymap's Linux deviations.
- **Alt+A inside a long-lived overlay** fires the prefix accelerator again. With `actionMode: NORMAL`,
  `meta_keybindings_process_event` still runs under a grab (`events.c:256-258`) and the filter lets NORMAL bindings
  through (`windowManager.js:1589-1601`).
- **NONE versus NORMAL** (VERIFIED, `windowManager.js:1590`): NONE filters every binding, including Ctrl+Alt+F*. NORMAL
  lets Super+L lock the screen mid-overlay. Inside the prefix on Ubuntu, Super+L reaches vase and moves the pane. The
  Keymap row says "nothing" happens, which is true only on Windows and the Mac.

**Fix**
- Write the key-ownership state machine into the protocol spec before M1. States: idle, prefix, overlay, sticky,
  pending-launch. For each, give the grab kind (stage grab or temporary accelerators), the actionMode, the timeout, the
  exit keys and what happens to queued keys. Test it at L0 with a fake extension.
- Use **temporary `grab_accelerator`s, not a stage grab**, for the bounded-key states: Esc only for a pending launch,
  Shift+arrows and Shift+,/. for sticky. The "set-swap is leaky" objection is about unbounded tables, not these.
- Queued keys after a non-modal reply are replayed through the virtual keyboard if S3 shows that works, otherwise
  dropped and documented.

### 4. The owner's monitors contradict "one monitor until M7" (HIGH, planning; facts the map missed)

**VERIFIED** (`~/.config/monitors.xml`, `gsettings`):
- The primary monitor (HDMI-3, 1920x1080, scale 1) sits at **x=2560, not (0,0)**.
- A 4K monitor at **scale 1.5** sits at (0,0).
- `experimental-features = ['scale-monitor-framebuffer', 'xwayland-native-scaling']`.
- `workspaces-only-on-primary = true`, so windows on the second monitor are on every workspace.

**What follows**
- vase would put the tab bar on the non-primary 4K screen.
- The L3 feel checks from M1 onward, and the two weeks of daily use, run on two mixed-scale monitors.
- Xwayland games under native scaling are their own coordinate case.

**Fix**
- Move the primary-display seam and mixed-scale placement into M1 or M2.
- Add fractional scale and Xwayland native scaling to S4.
- Leave only hotplug in M7.

### 5. The dev setup can still touch the real session (HIGH, the task's main safety concern)

- **L3 devkit binds the owner's logind session.** Unset `XDG_SESSION_ID` falls back to `session/auto`
  (`loginManager.js:141-160`), which here is the graphical session (`loginctl show-user`: `Display=2`). GDM answers on
  the host system bus, so `canLock()` is true (`loginManager.js:38-53`) and the dev shell builds a ScreenShield
  (`main.js:237`). That ScreenShield:
  - follows the real session's `Lock` signal (`screenShield.js:147`);
  - calls **`SetLockedHint` on the real session** (`screenShield.js:174`);
  - may take a suspend inhibitor (`:202-225`, gated on the mode's unlock dialog).

  The recipe in `devtest-security.md` §3.3, used by M0a, keeps the host system bus. `env -i` alone doesn't help.
- **S8 runs a `user`-parent mode in L3.** That mode starts `polkitAgent, keyring, networkAgent`
  (`sessionMode.js:13-19`) on the host system bus. The network agent would be a NetworkManager secret agent able to
  receive the owner's Wi-Fi prompts; the polkit agent would compete for the real session (INFERENCE).
- **`gnome-shell-test-tool` isolates nothing but settings.** It does `env = dict(os.environ)` and runs no
  `dbus-run-session`, and it uses `tempfile.TemporaryDirectory()` in `/tmp` (`/usr/bin/gnome-shell-test-tool:45-59,
  103`). Run on the host, it joins the real session bus and hits the small `/tmp` quota. F10 presents it as "the test
  vehicle on the desktop".
- **The crash marker is shared.** The dev shell creates and deletes the same `$XDG_RUNTIME_DIR/gnome-shell-disable-extensions`
  file that the real shell's `OnFailure` unit tests (`extensionSystem.js:54-72`;
  `org.gnome.Shell-disable-extensions.service`, `ConditionPathExists=%t/...`). This is low risk today and matters once
  the Tiler ships.
- **Clients started from a host terminal land on the real compositor.** Only clients the dev shell launches inherit its
  Wayland display.
- **M0a's exit check** diffs only `dconf dump /org/gnome/shell/`.

**Fix**
- L3 gets a **private system bus**: a plain `dbus-daemon` with a session-type config and no services, so `canLock()` is
  false and nothing reaches logind, polkit or NetworkManager.
- `just dev-shell` refuses to start when `DBUS_SYSTEM_BUS_ADDRESS` or `DBUS_SESSION_BUS_ADDRESS` is the host's.
- S8 (lock) runs **only at L1**.
- `gnome-shell-test-tool` runs only in podman, or through a wrapper that requires `env -i`, `dbus-run-session` and
  `TMPDIR` under `~/.cache`.
- M0a's exit check also compares `loginctl show-session 2 -p LockedHint`, the marker file, `~/.local/share/gnome-shell`
  and the mtime of `~/.config/monitors.xml`.
- Make M0a's first deliverable an S11 spike: does the devkit run with no system services?

### 6. Milestones are too big and in the wrong order (MEDIUM-HIGH)

- **M0** bundles the environment with nine spikes. S1 (wasm) feeds a fallback nobody plans to build: drop it. Order the
  rest by kill risk: S2, S9, a new S10 (sandbox, issue 7), S4, S6. Run S5, S7 and S8 later.
- **M1** holds three Round-2 decisions, the whole protocol, the mirror, the backend and painter, the extension's grabs,
  effects and settling, the picker's list chrome, adoption, and kill -9 and restart tests. Split it:
  - **M1a**: Rust only. The protocol, the mirror, `vase-gnome`, the state machine from issue 3 and a fake extension, at
    L0.
  - **M1b**: the extension with snapshot, deltas, effects and settling, but no keys and no chrome, at L1.
  - **M1c**: the prefix and the picker list.
- **M7**'s primary display moves to M1 or M2 (issue 4).
- **M8**'s fork hardening and Linux CI move to M0. Agent-written compositor code needs the L1 suite in CI from M1 on.
  The inherited `release*.yaml` files are still present: `release-finalize.yaml` runs with `contents: write` on every
  merged PR, guarded only by a `release/v` branch-name check.
- **The 4-6 week estimate** has no basis. It covers two programs, a protocol, CI from nothing and a container harness.
  Drop it, or re-estimate after M1a.

### 7. The daemon's systemd sandbox mostly won't apply on this desktop (MEDIUM-HIGH, security claim)

**Claim** (§3, point 6; Q2.1; Q3.3): the daemon is confined with `PrivateNetwork=yes`, `ProtectHome=read-only` and the
rest.

**What's wrong**
- In a **user** unit those options need `PrivateUsers=`, which needs unprivileged user namespaces (VERIFIED,
  `man systemd.exec` 259, SANDBOXING: "settings requiring file system namespacing ... are not available ... will work
  when used in conjunction with PrivateUsers=true").
- This Ubuntu has `kernel.apparmor_restrict_unprivileged_userns = 1` and the user manager runs unconfined.
  `unshare --user --map-root-user` fails here with "write failed /proc/self/uid_map: Operation not permitted"
  (VERIFIED, from an unsandboxed shell: `Seccomp: 0`).
- systemd turns unavailable sandboxing off "gracefully" (same section). So the unit would look confined and not be
  (INFERENCE; the exact failure mode, silent or failed unit, is UNVERIFIED).

**Fix**
- Claim only what works without namespaces: `NoNewPrivileges`, `RestrictAddressFamilies=AF_UNIX`, `SystemCallFilter`,
  `LockPersonality`, `MemoryDenyWriteExecute`, all seccomp-based. Verify each.
- For file and network confinement, have the sudo step install a root-owned **AppArmor profile** for
  `/usr/local/libexec/vase/vase-gnome`.
- Doctor reads `/proc/<pid>/status` (`Seccomp: 2`) and `/proc/<pid>/attr/current`. Add spike S10.

### 8. The peer check in Q2.1 can't work with socket activation (MEDIUM)

- `SO_PEERCRED` returns the credentials "in effect at the time of the call to connect(2), listen(2)" (VERIFIED,
  `man 7 unix`).
- With socket activation, systemd `--user` calls `listen()`, so the extension sees the user manager's pid, and
  `/proc/<pid>/exe` is `/usr/lib/systemd/systemd`.

**Fix.** Either the daemon binds its own socket (no socket activation; the extension reconnects with backoff), or the
exe check is dropped and only the uid is checked, plus a hello carrying the build hash that is compared with the lock.
Either way, write down that this is defence in depth only (same uid).

### 9. `NON_MASKABLE` is ignored for `grab_accelerator` chords (MEDIUM)

- `process_event` tests `binding->handler->flags & META_KEY_BINDING_NON_MASKABLE` (VERIFIED, `keybindings.c:1452-1453`).
- Every `grab_accelerator` binding shares the `external-grab` handler, whose flags are only `TRIGGER_RELEASE`
  (`keybindings.c:2762-2764`, `:1317`).
- So the kill chord and the Pause chord, grabbed by keycode as Q2.6 and Q3.4 plan, are masked by any client that
  inhibits shortcuts. M6's L1 test ("still responds to the Pause chord") would fail.

**Fix**
- Register the kill chord and Pause with `Main.wm.addKeybinding` and a GSettings schema. Its handler takes the flags
  given (`keybindings.c:973-978`). The extension ships `schemas/gschemas.compiled`; that is exactly audit condition 1's
  "only build step".
- Keep `grab_accelerator` for Alt+A, Alt+E and the app chords.

### 10. The recovery runbook assumes a VT is reachable (MEDIUM)

- Ctrl+Alt+F* is a mutter keybinding (`switch-to-session-N`, `keybindings.c:2673-2684`) running on the compositor's main
  loop. A **hung** shell can't switch VTs. During the prefix, actionMode NONE filters it too.
- `kernel.sysrq = 176` allows only sync, remount and reboot (VERIFIED). No SSH server is installed (VERIFIED).
- So "the keyboard can never stay grabbed" holds only while the main loop runs, and the VT runbook covers crashes and
  stuck grabs, not hangs.

**Fix**
- Write a hang branch into the runbook: Alt+SysRq+S, U, B.
- Ask the owner whether to allow more sysrq bits or to install SSH. These are real questions with the facts behind them.
- Rehearse both branches.

### 11. Chained's writes would undo GNOME's crash safety net (MEDIUM)

- Bootstrap and `chained profile` write **every** key (`14-retire/docs/tools/gnome.md`, "The entry").
- Once Q3.2 allows `disable-user-extensions=false`, those commands silently re-arm a Tiler that GNOME's `OnFailure` unit
  just disabled (`org.gnome.Shell-disable-extensions.service`).
- With Tiling Assistant already in `disabled-extensions`, a crash at login leaves no tiler at all.
- The net only covers the first 60 s (`extensionSystem.js:63-72`). A crash later logs the owner out and loads the Tiler
  again at the next login.

**Fix**
- Mark the key "written once at the switch-over, never re-asserted".
- Doctor FAILs on a crash flip and names the recovery.
- `chained profile` refuses to flip it back without an explicit flag.
- Consider re-enabling Tiling Assistant automatically when the Tiler isn't ACTIVE.

### 12. A per-glyph width table won't match Pango (MEDIUM)

- Powerline layout and title ellipsizing call `measure` on arbitrary strings and their prefixes (`chrome/bar.rs:75-125`,
  `powerline.rs:152`).
- Windows measures with a real DirectWrite layout (`vase-windows/src/chrome/gpu.rs:116-123`).
- A glyph table misses shaping, kerning, font fallback, emoji and CJK, so tabs mis-size and clip. This lands in M1 (the
  picker list) and M4.

**Fix.** With each title delta, the extension sends the title's cumulative advance array from Pango
(`index_to_pos`). It also sends a small table for numbers and marks. The daemon lays out from those, and golden tests
compare with Pango in L1.

### 13. The cost of a grab in fullscreen games and video is not on the map (MEDIUM)

- `pushModal` calls `global.compositor.disable_unredirect()` (VERIFIED, `main.js:706` @50.1).
- The focused client loses keyboard and pointer focus (issue 3).
- An accidental Alt+A in a fullscreen Xwayland game sends it FocusOut. Some games pause or minimize on focus loss
  (UNVERIFIED per game).

**Fix**
- While a fullscreen window has focus, ungrab Alt+A and Alt+E so the game gets them. This is consistent with "fullscreen
  windows are left alone".
- Add the case to S2, and to M6 with a real game.

### 14. Keys already cross D-Bus, through IBus, before the prefix arms (MEDIUM-LOW, the task's "latency" concern)

- On this desktop `ibus-daemon` runs.
- When the focused client has an active text-input, every key, Alt+A included, goes to IBus over D-Bus
  (`inputMethod.js:331-360`) **before** mutter's keybindings: `meta_wayland_text_input_update` at `events.c:170`, then
  keybindings at `:257`.
- The daemon's unix-socket hop is not the D-Bus cost; IBus's is. How keys typed fast across the grab change are ordered
  is UNVERIFIED.
- S9's echo test doesn't measure the path the user feels.

**Fix**
- S2 adds a case: focus in a GTK4 entry, scripted Alt+A then `\` 5-10 ms apart. Assert nothing lands in the entry, and
  measure the time from key press to effect.
- S9 measures end to end under paint load, not an echo.

### 15. "The extension spawns nothing" is false (MEDIUM-LOW)

- The effects batch includes `launch` (§3), carried out with `Shell.AppSystem` and `open_new_window`/`launch` inside the
  compositor. The shell spawns the app.
- The planned rewording of audit condition 3 (Q3.3) and the tripwire list miss this.

**Fix**
- Allow only desktop-file ids found in `Shell.AppSystem.get_installed()`.
- Reword condition 3: "launches only installed apps the user picked, through Shell.AppSystem".
- Add a tripwire for `.launch(` and `open_new_window(`.

### 16. The mirror is stale inside a dispatch (LOW-MEDIUM)

- `tab_partly_covered` calls `list_windows()` straight after `focus` (`daemon/effects.rs:68-77`).
- With the mirror, the raises just queued aren't visible yet, so the daemon can re-front windows more than it needs to,
  which shows as flicker.

**Fix.** `GnomeBackend` updates the mirror optimistically on raise, focus and `set_frame`, and deltas correct it. Cover
it with a property test.

### 17. Destination §1.6 overclaims, and teardown isn't safe against errors (LOW-MEDIUM)

- "A bug in the layout logic cannot end the session" holds for the daemon only. The extension still calls Meta on
  windows by id.
- An exception in `disable()` is logged and the rest of the teardown is skipped (`extensionSystem.js:216-218`).
- A throw in `enable()` leaves ERROR, and `disable()` is never called afterwards.
- `inhibit_mapped` is a bare ref-count with no owner (VERIFIED, `window.c:8713-8736`), so one missed uninhibit leaves a
  window invisible for the rest of the session.

**Fix**
- Look the window up again on every effect, and drop references at `unmanaging`.
- Tear down in `try`/`finally` steps, in this order: popModal, uninhibit, ungrab, destroy the chrome.
- Add an L1 test that injects a throw.

### 18. Questions put to the owner that facts already settle (MEDIUM, process)

- **Q1.4 and Q4.3 (port to GNOME 51).** Ubuntu 26.04 LTS stays on GNOME 50 for its life; stable updates bring only 50.x
  point releases (GNOME's micro-release exception, https://wiki.ubuntu.com/StableReleaseUpdates/GNOME). The spec puts
  every other Ubuntu version out of scope. Drop the question. Use `["50"]` and have CI track 50.x updates.
- **Q2.6 (keycode or keysym).** The desktop has one input source, `('xkb','us')`, with no xkb options, so there is no
  observable difference. Decide on keycode.
- **S5, M3 and Q2.4 with blur-my-shell and rounded-window-corners.** Neither loads on this desktop: the user dir isn't
  scanned and `disable-user-extensions` is true. `gnome.md` says they can't come back while installation is off. Drop
  them, unless the owner wants them back as system installs; that is the real question.
- **Q1.3 (install `mutter-dev-bin`).** State the facts: it is one package, `apt-get -s` shows no new dependencies, and it
  contains only `/usr/libexec/mutter-devkit`, a `.desktop` file and icons.
- **Q2.1, Q2.4, Q2.5 and Q2.7** are engineering defaults a spike confirms. They are not owner questions. Present them as
  "default unless S-n shows X".
- **A real question the facts raise:** the Ubuntu Dock sits at the **bottom** with autohide and intellihide
  (`dash-to-dock dock-position 'BOTTOM'`), which is the bottom tab bar's place. Bar at the top, or the dock moved?
- **Q3.6 (the switch-over).** `edge-tiling` is already false because Tiling Assistant overrides it at runtime and
  restores the saved value on disable (TA `extension.js:166-169`; SettingsOverrider `_clear`). The writes have to be
  ordered: disable TA, let it restore, then Chained writes the mutter keys. Otherwise TA's restore races Chained's write
  and shows up as a doctor WARN.

---

## Load-bearing claims I re-checked and found correct

- **F9:** `includeUserDir = allow-extension-installation` (`extensionSystem.js:711-712`), and `enabled-extensions` is
  read only while `disable-user-extensions` is false (`:545-556`), with `disabled-extensions` taking precedence. On this
  desktop `enabled-extensions` holds only mode extensions plus the three user ones that aren't scanned, so Q3.2's three
  writes enable nothing else.
- **F4:** keybindings run before Wayland delivery and run under a grab (`events.c:256-258`). Under a grab the focus
  surface is NULL (`meta-wayland-input.c:117-131`). `ClutterGrab` has a `revoked` property (`clutter-grab.c:85-120`).
- **pushModal's default actionMode is NONE** (`main.js:697-700`). NONE filters every binding, VT switching included
  (`windowManager.js:1589-1591`).
- **Mutter 50.1 has the APIs:** `inhibit_mapped`/`uninhibit_mapped`, `get_min_size`, `add_external_constraint` and
  `get_id` (`window.h:422-482`). `configure` is emitted before the first xdg configure (`meta-wayland-xdg-shell.c:984-993`).
- **Window ids** start from a random 32-bit base for each shell process (`display.c:3182-3191`), so vase's "match by id
  first" doesn't collide across logins in practice.
- **The crash path:** `OnFailure=org.gnome.Shell-disable-extensions.service gnome-session-shutdown.target`, `Restart=no`.
- **Paths and packages:** `/usr/local/share/` is on the running shell's `XDG_DATA_DIRS`. Rust 1.93.1 is the archive
  candidate, enough for edition 2021. The installed test tool has `--extension` and `--devkit`.

## Still unverified (make them spike rows)

- Whether the user unit's sandboxing fails or silently turns off (S10).
- Whether the devkit runs on a private system bus with no services (S11).
- How keys filtered by IBus are ordered across a grab change (S2).
- What happens to games on focus loss (S2, M6).
- How a dev shell whose mode has no unlock dialog reacts to the real `Lock` signal.

## Sources

- gnome-shell 50.1, https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/50.1/ (copies in `scratch/critique/gs_*`):
  - `js/ui/extensionSystem.js`
  - `js/ui/main.js`
  - `js/ui/windowManager.js`
  - `js/ui/screenShield.js`
  - `js/misc/loginManager.js`
  - `js/misc/inputMethod.js`
- mutter 50.1, https://gitlab.gnome.org/GNOME/mutter/-/raw/50.1/ (`scratch/critique/mu_*`):
  - `src/core/events.c`
  - `src/core/keybindings.c`
  - `src/core/window.c`
  - `src/core/display.c`
  - `src/meta/window.h`
  - `clutter/clutter/clutter-grab.c`
  - `src/wayland/meta-wayland-text-input.c`
  - `src/wayland/meta-wayland-input.c` (scratch copy)
- **Man pages:** `man systemd.exec` (systemd 259, saved as `scratch/critique/systemd.exec.txt`); `man 7 unix`.
- **Desktop, read-only:**
  - `/usr/bin/gnome-shell-test-tool`
  - `/usr/lib/systemd/user/org.gnome.Shell@.service` and `org.gnome.Shell-disable-extensions.service`
  - `/proc/<gnome-shell>/environ`
  - `~/.config/monitors.xml`
  - `gsettings get`: `org.gnome.shell`, `org.gnome.desktop.input-sources`, `org.gnome.mutter`, `dash-to-dock`
  - `sysctl kernel.apparmor_restrict_unprivileged_userns kernel.sysrq`
  - `loginctl show-user` and `show-session`
  - `apt-get -s install mutter-dev-bin`
  - `/usr/share/gnome-shell/extensions/tiling-assistant@ubuntu.com/extension.js`
- **vase-chained @022340f:**
  - `vase-core/src/daemon/input.rs:15-55`
  - `daemon/effects.rs:60-124`
  - `daemon/pane_picker/keys.rs:16`
  - `input/mod.rs:140-185`
  - `state.rs:66-99`
  - `chrome/bar.rs:75-125`
  - `vase-windows/src/chrome/gpu.rs:116-123`
  - `.github/workflows/release-finalize.yaml:1-18`
- **Chained:**
  - `.scratch/chained-v1/spec.md:865-878,1039`
  - `docs/research/security-audit.md:954-965`
  - `.claude/worktrees/14-retire/docs/tools/gnome.md`
  - `.claude/worktrees/14-retire/data/gnome.toml`
  - `.claude/worktrees/14-retire/docs/keymap.md:66-131`
