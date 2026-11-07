# Wayfinder: the Linux Tiler (vase-chained on GNOME Shell 50, Wayland)

The map for the Tiler track's Linux leg (spec Out of Scope, `.scratch/chained-v1/spec.md:1039`: "That track starts
with a `/wayfinder` and a prototype of the extension"). Written 2026-11-07 from five research notes in this folder
(`vase-model.md`, `gnome-api.md`, `prior-art.md`, `architecture.md`, `devtest-security.md`), then **revised the same
day against `critique.md`** (18 issues). The first version is kept at `scratch/wayfinder.v1.md`. Research only: nothing
was installed, and nothing in `~/src/vase-chained`, the Chained repo or the desktop's settings was changed.

Labels, as in the critique: **VERIFIED** means checked in source or read-only on the desktop. **INFERENCE** means
reasoned from verified facts. **UNVERIFIED** means a spike has to settle it; every UNVERIFIED item has a spike row (§5).

How to read it:
- §1 is the destination.
- §2 is where we stand: facts, corrections, and what is still unverified.
- §3 is the recommended architecture, with its lifecycle, its key-ownership state machine and its core seams.
- §4 is the decision tree. Round 1 is final. Questions are worded so that "yes" accepts the recommendation. Items that
  a spike settles are listed as engineering defaults, not owner questions.
- §5 is the prototype plan, with how each milestone is tested.
- §6 is what changes in Chained when the Linux Tiler lands.
- §7 is what stays out.
- The last section maps each critique issue to the place it was fixed.

---

## 1. Destination: what "done" means for a first usable Linux Tiler

On the owner's desktop (Ubuntu 26.04 LTS, GNOME Shell 50.1 / Mutter 50.1 (API 18) on Wayland, gjs 1.88), the reviewed
commit of `MaartenHaine/vase-chained` provides a Linux Tiler that Chained installs and checks. It must meet all of the
following.

1. **The same product as on macOS and Windows.** The shared vase-core model, keymap, `config.toml` schema and
   `state.json` format, with no second layout engine (ADR-0001: "one model and keymap"). vase-core gains a small,
   listed set of seams (§3.4); it is not "unchanged".
2. **Prefixes.** Alt+A and Alt+E work in every windowed app, with the Keymap's prefix and stack-prefix tables
   (`.claude/worktrees/14-retire/docs/keymap.md:66-131`). The exceptions are the Linux deviations written into the
   Keymap (Q2.1): for example the 3 s prefix timeout, and no prefix while a fullscreen window has focus.
3. **Layout.** Tabs, panes (split, focus, resize, move, zoom, break, close), stacks, the empty-pane picker, the window
   switcher, the command line, rename and the help sheet all work **on the owner's two monitors**: the primary
   1920x1080 at scale 1 placed at x=2560 (not at (0,0)), and a 4K monitor at scale 1.5 with fractional scaling and
   Xwayland native scaling (F11).
4. **Chrome.** The tab bar, stack bars, empty-pane placeholders, the focus border, the pickers and lists and the help
   sheet are drawn above windows, hidden over fullscreen windows, clickable, crisp at scale 1 and 1.5, and placed so
   they don't fight the Ubuntu Dock (Q1.5).
5. **Gaming.**
   - Pause stops all tiling and releases Alt+A and Alt+E to the game.
   - Fullscreen windows are not managed (a core seam, §3.4), and Alt+A and Alt+E aren't grabbed while one has focus.
   - Apps on a game ignore list are never managed (if Q4.1 is answered by then).
6. **Safety.**
   - A bug in the layout logic, which lives in the daemon, cannot end the GNOME session. The extension is the part
     that runs inside the compositor: it stays small, looks every window up again on each effect, and tears down step
     by step so one error can't skip the rest (§3.2).
   - While the shell's main loop runs, the keyboard can't stay grabbed: the prefix times out, Esc leaves, a revoked
     grab or a lost daemon releases it, and the kill chord (Ctrl+Super+Alt+Esc) always gives the keyboard back.
   - A hung shell is outside what the Tiler can fix. The recovery runbook has a crash branch (text console) and a hang
     branch (SysRq), and both have been rehearsed once (F15).
7. **Persistence.** The layout survives, with no window moving: a screen lock and unlock; enabling or disabling
   another extension; a daemon restart or upgrade; and a log-out and log-in (vase's re-matching by id, then by
   `(app, title)`, `vase-core/src/state.rs:49`).
8. **Chained integration.** It is a catalogue entry installed by Chained from the reviewed SHA through an install
   channel that actually loads under Chained's rules (§6). Doctor checks it. It has passed its own assessment
   (audit §24, rewritten), and it has replaced Tiling Assistant with a one-step way back.
9. **The evidence.**
   - The fork's Linux CI is green: Rust tests, JS lint and unit tests, headless gnome-shell integration tests, and the
     tripwires.
   - The manual checklist (spec story 119) passes.
   - The owner has used it daily for two weeks on both monitors without losing a session.

Not part of "done": publishing on extensions.gnome.org, animations, a native Adwaita chrome style (powerline only),
Dock badges, configurable keys (§7). GNOME 51 is not a target at all: 26.04 LTS stays on GNOME 50 (F21).

---

## 2. Where we stand

### 2.1 Facts that shape every option

Sources are in the notes and in `critique.md`. Copies of the 50.1 sources are in `scratch/src/` and `scratch/critique/`.

| # | Fact | Status and source |
|---|---|---|
| F1 | Only the compositor can move other apps' windows on GNOME Wayland, so part of the Tiler must be a GNOME Shell extension. GNOME 50 has no X11 session; X11 survives only as Xwayland clients (games, Steam). | ADR-0001; `gnome-api.md` §0.5 |
| F2 | An extension runs inside the compositor. A crash ends the session (`org.gnome.Shell@.service`: `OnFailure=org.gnome.Shell-disable-extensions.service gnome-session-shutdown.target`, `Restart=no`), and a busy loop freezes the desktop. Code can't be reloaded: every iteration needs a fresh shell. | VERIFIED, `/usr/lib/systemd/user/org.gnome.Shell@.service` |
| F3 | Mutter has no layer-shell, so the chrome must be St actors inside the extension. | mutter issue #973; `architecture.md` C3 |
| F4 | A tmux-style prefix needs a stage grab (`Main.pushModal`). Keybindings run before Wayland delivery and still run under a grab (`events.c:249-258`), so Alt+A never reaches the app. Under a grab the Wayland focus surface is NULL (`meta-wayland-input.c:117-131`): **the focused client loses keyboard and pointer focus**. `ClutterGrab` has a `revoked` property (`clutter-grab.c:85-120`). | VERIFIED @50.1 |
| F5 | Off-screen parking is impossible (`constrain_partially_onscreen`, `constraints.c:2020-2098`). The options are raise-to-cover (vase's current behaviour), minimize, or Mutter 50's `inhibit_mapped()`, which is a bare reference count with no owner (`window.c:8713-8736`): one missed uninhibit leaves a window invisible for the session. | VERIFIED @50.1 |
| F6 | Resizes on Wayland are asynchronous and can settle at a different size. Every tiler re-checks the frame after `size-changed`. | `gnome-api.md` §0.2, §4.2; utwm `_settle` |
| F7 | **The extension is disabled and re-enabled often.** At every screen lock (session-mode change, `_sessionUpdated`, `extensionSystem.js:782-786`) unless it declares `unlock-dialog`; and whenever any extension earlier in the load order is enabled or disabled, which "rebases" every later one (`extensionSystem.js:195-240`). The switch-over itself cycles the Tiler. | VERIFIED @50.1 |
| F8 | vase-core (5,928 lines, 183 tests, pure-Rust deps) sits behind two synchronous traits, `Backend` and `Painter`. It is single-threaded and polling; the **platform loop** calls `reconcile()` (`daemon/reconcile.rs:128`) and the `tick_*` functions, so a host can freeze them by not calling them. | `vase-model.md` §1, §3; `vase-windows/src/bin/vase.rs` |
| F9 | **With today's settings no new extension can load.** `allow-extension-installation=false` stops the shell scanning `~/.local/share/gnome-shell/extensions` (`extensionSystem.js:711-712`, `fileUtils.js:19-22`), and `disable-user-extensions=true` makes it ignore `enabled-extensions` (`:545-556`). Chained's lint refuses to write either key. | VERIFIED twice (map and critique) |
| F10 | `gnome-shell-test-tool` (50.1, `--extension`, `--devkit`) is installed, but **it isolates only settings and the XDG config, data and cache dirs**: it copies the whole environment (`env = dict(os.environ)`), starts no private bus, and uses a `TemporaryDirectory()` in `/tmp` (`/usr/bin/gnome-shell-test-tool:45-59,103`). Run on the host, it joins the real session bus and fills the small `/tmp` quota. `mutter-dev-bin` (universe) is not installed; `apt-get -s` shows no new dependencies, and it holds only `/usr/libexec/mutter-devkit`, a `.desktop` file and icons. There is no Rust on the host; the archive has rustc/cargo 1.93.1. | VERIFIED (critique; test tool re-read for this revision) |
| F11 | **The owner's monitors.** `~/.config/monitors.xml`: the primary (HDMI-3, 1920x1080, scale 1) at x=2560, y=0; a 4K monitor (2560 logical wide) at scale 1.5 at (0,0). A second stored layout has the primary at (2560,360). `org.gnome.mutter experimental-features = ['scale-monitor-framebuffer', 'xwayland-native-scaling']`; `workspaces-only-on-primary=true`, so windows on the 4K monitor are on every workspace. vase picks the main screen as the display at (0,0) on every display change (`reconcile.rs:61`), so it would put the tab bar on the 4K monitor. | VERIFIED, read-only |
| F12 | **The systemd namespace sandbox doesn't apply to a user unit here.** In a user unit, `ProtectHome=`, `PrivateNetwork=` and the other file-system and namespace options need `PrivateUsers=`, which needs unprivileged user namespaces (`man systemd.exec` 259, SANDBOXING), and systemd drops unavailable sandboxing "gracefully". This desktop has `kernel.apparmor_restrict_unprivileged_userns=1`, and `unshare --user --map-root-user` fails with EPERM. Whether such a unit fails or silently runs unconfined is UNVERIFIED (S10). | VERIFIED (sysctl, man page); outcome UNVERIFIED |
| F13 | `SO_PEERCRED` gives the peer's credentials from the time of its `connect()` or `listen()` (`man 7 unix`). Under socket activation systemd `--user` called `listen()`, so the peer pid is the user manager's. | VERIFIED |
| F14 | **`NON_MASKABLE` has no effect on `grab_accelerator` chords.** Mutter tests the *handler's* flags (`keybindings.c:1452-1453`), and every external grab shares the `external-grab` handler, whose only flag is `TRIGGER_RELEASE` (`:2762-2764`). A binding added through `meta_display_add_keybinding` (`Main.wm.addKeybinding`, with a GSettings schema) gets the flags it is given (`:973-978`). Separately, gnome-shell's filter lets **no** binding through while `actionMode` is NONE (`windowManager.js:1589-1591`), so inside a NONE grab even a NON_MASKABLE binding never fires: the grab's own key handler has to check the kill chord. | VERIFIED @50.1 (re-read for this revision) |
| F15 | **A text console isn't always reachable.** Ctrl+Alt+F* is a mutter keybinding (`switch-to-session-N`, `keybindings.c:2673-2684`) that runs on the compositor's main loop: a hung shell can't switch VTs, and a NONE grab filters it too. `kernel.sysrq=176` (Ubuntu's default) allows only sync (16), remount read-only (32) and reboot (128). No SSH server is installed. | VERIFIED, read-only |
| F16 | **IBus sees keys before the prefix does.** `ibus-daemon` runs here. When the focused client has an active text-input, mutter offers each key to the input method (`meta_wayland_text_input_update`, `events.c:170`, and over D-Bus, `inputMethod.js:331-360`) before keybindings (`events.c:257`). How keys typed quickly across a grab change are ordered is UNVERIFIED (S2). | VERIFIED order; timing UNVERIFIED |
| F17 | **A grab has a cost.** `pushModal` calls `global.compositor.disable_unredirect()` (`main.js:706`), so a fullscreen game or video loses direct scan-out, and the client loses focus (F4). What each game does on focus loss is UNVERIFIED (S2, M6). | VERIFIED @50.1 |
| F18 | **A nested shell reaches the real login session.** With `XDG_SESSION_ID` unset it asks logind for `session/auto` (`loginManager.js:141-160`), which here is the owner's graphical session. With GDM answering, `canLock()` is true and the shell builds a ScreenShield (`main.js:237-238`), which follows the real session's `Lock` signal (`screenShield.js:147`) and calls `SetLockedHint` on it (`screenShield.js:174`). A `user`-parent session mode also starts the polkit, keyring and NetworkManager agents (`sessionMode.js:13-19`) on whatever system bus it has. | VERIFIED @50.1 (re-read for this revision) |
| F19 | **The Ubuntu Dock is at the bottom**, with autohide and intellihide (`dash-to-dock dock-position 'BOTTOM'`). vase's tab bar position is already a config option, `[tabbar] position = "top" \| "bottom"`; unset means "leave it to the platform" (`vase-core/src/config.rs:184-191`). | VERIFIED, read-only |
| F20 | Tiling Assistant overrides `edge-tiling` at runtime and restores the saved value when it is disabled (TA `extension.js:166-169`, SettingsOverrider `_clear`). | VERIFIED (critique) |
| F21 | Ubuntu 26.04 LTS stays on GNOME 50 for its life; stable updates bring 50.x point releases only (https://wiki.ubuntu.com/StableReleaseUpdates/GNOME). The spec puts other Ubuntu versions out of scope. | Per critique; not re-fetched |
| F22 | The only input source is `('xkb','us')`, with no xkb options. | VERIFIED, read-only |
| F23 | Window ids start from a random 32-bit base for each shell process (`display.c:3182-3191`), so vase's "match by id first" doesn't collide across logins. | VERIFIED (critique) |
| F24 | The fork carried the inherited release workflows: `release-finalize.yaml` ran on every closed PR to `main` with `permissions: contents: write`, guarded only by a `release/v` branch-name check. M0a's hardening deleted all three (`97369fd`); they stay live on GitHub's `main` until that lands there (`github-settings.md`). | VERIFIED, `.github/workflows/release-finalize.yaml:1-18` before `97369fd` |
| F25 | `/usr/local/share/` is on the running shell's `XDG_DATA_DIRS`. | VERIFIED (critique, `/proc/<gnome-shell>/environ`) |

### 2.2 Corrections

To the research notes (from the first version, still true):

1. `architecture.md` C13 and §3 and `prior-art.md` §4.10 are wrong that `allow-extension-installation=false` only blocks
   remote installs (F9). The install plan "copy to `~/.local/...`" doesn't work; §6 replaces it.
2. `vase-model.md` §6 (Keymap missing Alt+E and the kill chord) describes `main`. The ticket-13/14 branch has both.
3. **Pass-through is in the Keymap** ("After either prefix, any key with no row passes through to the app", and
   "Alt+A, then Alt+A: send one Alt+A", `14-retire/docs/keymap.md:70,82`). In vase that works only because an unbound
   second key is `PassThrough` (`input/mod.rs:166-172`); `InputCommand::SendPrefix` is a no-op
   (`daemon/input.rs:85`).
4. **The pane picker is modal whenever an empty pane has focus** (`daemon/input.rs:15-17`,
   `pane_picker/keys.rs:20-46`), and a split always makes one.
5. The fork CI the spec describes didn't exist (F24). M0a adds it: the hardening, then `.github/workflows/linux.yaml`.
6. The notes' counts differ (183 or 177 tests). Without the WebAssembly fallback this no longer matters.

To the first version of this map (from `critique.md`):

7. **"Lock and unlock are free" was wrong.** The daemon kept reconciling against a stale mirror while the extension was
   gone, and a clean stop ran `restore()` (`daemon/effects.rs:112-124`). §3.1 adds a lifecycle.
8. **"vase-core runs unchanged" was wrong.** The plan needs about nine seams (§3.4).
9. **"The grab holds back fast typing" was wrong.** Keys taken by the grab after the prefix has resolved are lost
   unless replayed. §3.3 sets the rule.
10. **"One monitor until M7" was wrong** for this desktop (F11). Primary display and mixed scale move to M1b.
11. **The dev setup wasn't isolated**: the L3 recipe kept the host system bus (F18), and the test tool was presented as
    safe on the host (F10). §5's test levels fix both.
12. **The sandbox claim was wrong** (F12), the **peer check couldn't work** with socket activation (F13), and
    **`NON_MASKABLE` on accelerators does nothing** (F14).
13. **"The extension spawns nothing" was wrong**: the `launch` effect starts apps through `Shell.AppSystem` inside
    the compositor (§3.6).
14. **A per-glyph width table won't match Pango** for titles (kerning, fallback, emoji, CJK). The measuring contract
    changes (§3, "Chrome layout").
15. **The 4-6 week estimate had no basis.** It is dropped; re-estimate after M1a.
16. Several owner questions were settled by facts (F21, F22, and that blur-my-shell and rounded-window-corners can't
    load, F9). They are removed from §4.

### 2.3 Still unverified (each has a spike row in §5)

| Unknown | Spike |
|---|---|
| Does the devkit run with private, empty system and session buses and a private `XDG_RUNTIME_DIR`? Does the viewer still reach the host compositor through an absolute `WAYLAND_DISPLAY`? | S11 |
| Fast Alt+A then `\` into a GTK4 entry with IBus active: does anything land in the entry? Press-to-effect time? | S2 |
| Can a host observe "any other key" without a grab (vase's sticky exit), or not at all? (INFERENCE: not, because unconsumed keys go to Wayland without the stage seeing them.) | S2 |
| Inside a NONE prefix grab, does Super+L reach the grab actor (and so vase) rather than lock the screen? (INFERENCE: yes, F14.) | S2 |
| What fullscreen Xwayland games do on focus loss. | S2, M6 |
| Do Clutter virtual-keyboard keys trigger `grab_accelerator` bindings, and can a key or a click be replayed to the app after the pop without re-arming? | S3 |
| Can GJS read the peer credentials of a unix socket (`Gio.Socket.get_credentials()`) and resolve `/proc/<pid>/exe`? | S9 |
| Does a user unit with namespace options fail, or run unconfined? Which seccomp options apply? Does an AppArmor profile attach to a binary started by systemd `--user`? | S10 |
| Do headless virtual monitors accept scale 1.5, or does the L1 runner need `ApplyMonitorsConfig`? | S4 |
| How a dev shell whose mode has no unlock dialog reacts to a `Lock` signal (only relevant if S11 fails and the private bus is dropped). | S11 |

---

## 3. Recommended architecture

**A thin GJS extension (the bridge and renderer) plus a Rust daemon (`vase-gnome`) that runs vase-core's `Daemon`
with a small set of seams, talking over one versioned protocol on a unix socket the daemon binds itself. The daemon is
a `systemd --user` service bound to the graphical session, confined by seccomp options and a root-owned AppArmor
profile, and it survives the extension's frequent disable/enable cycles by freezing, not by restoring.**

```
 gnome-shell process (compositor)                       vase-gnome daemon (systemd --user)
 ┌─────────────────────────────────────────┐            ┌──────────────────────────────────────────────┐
 │ extension (GJS, no deps)                 │  protocol  │ GnomeBackend: Backend  ← mirror (optimistic)  │
 │  • addKeybinding (NON_MASKABLE): kill,   │◄──────────►│ GnomePainter: Painter  → draw lists           │
 │    Pause                                 │  unix sock │ vase-core Daemon<B,C> + seams (§3.4)          │
 │  • grab_accelerator: Alt+A, Alt+E, app   │  JSON lines│ lifecycle: Connected / Disconnected (frozen)  │
 │    chords, temporary sticky/Esc keys     │            │ key-ownership state machine (§3.3)            │
 │  • pushModal per the state machine       │            │ config.toml / state.json (XDG dirs)           │
 │  • Meta signals → snapshot + deltas      │            │ 25 ms tick, 100 ms reconcile (only connected) │
 │  • effects → move/raise/activate/launch  │            └──────────────────────────────────────────────┘
 │  • St chrome; Pango advances for titles  │
 │  • safety: timeouts, release on EOF,     │
 │    try/finally teardown                  │
 └─────────────────────────────────────────┘
```

How it works:

- **Extension to daemon:** a hello (protocol version, compositor-instance id, build hash), a snapshot, then deltas
  from Meta signals (windows, stacking, focus, displays and the primary index, work areas, fullscreen, scale), keys
  while the state machine says the daemon owns them, clicks as global points, and text measurements.
- **Daemon to extension:** one batch of effects per dispatch (place, raise, focus, close, launch, warp), a draw list
  for the chrome, the key mode (§3.3), and the list of chords to grab.
- **Reads are local.** `list_windows`, `title` and the other queries read the daemon's mirror (`architecture.md`
  §1(b)), so vase's synchronous traits never wait on the socket. `GnomeBackend` updates the mirror optimistically on
  raise, focus and `set_frame`, and the next delta corrects it. This matters inside one dispatch:
  `tab_partly_covered` calls `list_windows()` straight after `focus` (`daemon/effects.rs:68-77`), and a stale mirror
  would re-front windows needlessly and flicker.
- **Chrome layout** runs in Rust (`chrome::powerline::layout`, as the Windows painter does,
  `vase-windows/src/chrome/mod.rs:87`). `Painter::measure(text, size)` (`chrome/paint.rs:50`) is called on titles and
  their prefixes while ellipsizing (`chrome/bar.rs:75-125`), and Windows answers with a real DirectWrite layout
  (`vase-windows/src/chrome/gpu.rs:116-123`). So with each title delta the extension sends that title's cumulative
  Pango advance array (`Pango.Layout.index_to_pos`) at each chrome size the daemon declared in its hello, plus a small
  table for digits, marks and separators. A cut between two glyphs that kern, and the join with "…", are
  approximations; golden tests at L1 compare the daemon's widths with Pango's within a tolerance.
- **The extension only paints** and reports clicks to `Daemon::click` (`daemon/bar.rs:75`).

### 3.1 Lifecycle

The extension is disabled at every lock and at every rebase (F7), and the daemon is stopped at log-out and restarted at
every upgrade. The daemon therefore has an explicit "extension gone" state.

| Daemon state | Entered by | What runs | Leaves to |
|---|---|---|---|
| **Starting** | the unit starts | nothing; waits for a hello | Connected (builds the `Daemon` from `state::load` + `state::restore` against the snapshot, as the other platforms do, `vase-windows/src/bin/vase.rs:74-78`) |
| **Connected(instance)** | a valid hello and snapshot | effects, ticks, the 100 ms reconcile | Disconnected on EOF or a missed heartbeat |
| **Disconnected(instance)** | EOF | **nothing**: no reconcile, no ticks, no effects; the model is kept and saved once | same instance: the new snapshot is diffed into the mirror, then reconcile resumes and forgets only windows that really closed. New instance (a new shell pid and start time, for example after a re-login while the user manager lingered): save, drop the `Daemon`, rebuild it through the Starting path, which re-matches by `(app, title)` because window ids are new (F23) |
| **Stopping** | SIGTERM (log-out, `systemctl --user restart`, upgrade) | save and **detach**: hide chrome, never `restore()` | exit |
| **Quitting** | `q`, `:quit`, the kill chord | vase's `restore()` (`daemon/effects.rs:112-124`): windows back to their pre-adoption frames | exit |

- The unit: `PartOf=graphical-session.target`, `WantedBy=graphical-session.target`, `After=graphical-session.target`
  (the target is active on this desktop, VERIFIED read-only), `Restart=on-failure` with a start limit. A stop at
  log-out is a SIGTERM, which detaches.
- The extension's `enable()` connects with backoff (100 ms doubling to 2 s); it never spawns the daemon.
- The extension's `disable()` runs in `try`/`finally` steps, in this order: pop the modal, uninhibit every window it
  inhibited (tracked in its own set, because the ref count has no owner, F5), ungrab every chord, destroy the chrome,
  close the socket. An exception in `disable()` is only logged and skips the rest of that function
  (`extensionSystem.js:216-218`), hence the separate steps. A throw in `enable()` leaves the extension in ERROR and
  `disable()` is never called, so `enable()` also cleans up after itself on failure.
- **What a lock costs:** a teardown, a reconnect and a snapshot, with no window moving. That is the claim the L1 test
  in M1b checks (lock twice, toggle another extension, restart the unit; assert no frame changed). UNVERIFIED until then.

### 3.2 What the extension trusts

- It looks a window up by id again for every effect (`global.display.list_all_windows()` index kept current by
  `window-created` and `unmanaging`), drops references at `unmanaging`, and ignores effects on unknown ids.
- It accepts `launch` only for desktop-file ids present in `Shell.AppSystem.get_default().get_installed()` (§3.6).
- It enforces its own timeouts (3 s prefix, 200 ms missed reply, heartbeat), so a hung daemon can't hold a grab.

### 3.3 Key ownership state machine

vase assumes a hook that sees every key and decides on the spot (`vase-model.md` §3c). Under GNOME, a key is either
grabbed (and lost to the app) or not seen at all. So each state names exactly what the Tiler takes. The extension holds
the state; the daemon's key-mode reply moves it. This table becomes part of the protocol spec in M1a and is tested at
L0 with a fake extension.

| State | Entered by | Grab | `actionMode` | Ends on | Keys taken in flight |
|---|---|---|---|---|---|
| **Idle** | start | `grab_accelerator`: Alt+A, Alt+E, `[[app_focus]]` chords. `addKeybinding` with `NON_MASKABLE`: kill chord, Pause | – | – | – |
| **Prefix** (Alt+A or Alt+E armed, and multi-digit tab entry) | the accelerator callback calls `pushModal` **synchronously**, before any reply | stage grab | NONE | the daemon's reply leaving Prefix *and* the command key's release (PaperWM #70); Esc; **3 s with no key** (Linux deviation); `notify::revoked`; EOF or a 200 ms missed reply; the kill chord | every key goes to the daemon in order. A key the daemon answers PassThrough (an unbound key, or one typed after the command but before the pop) is **replayed** through a Clutter virtual keyboard after the pop if S3 shows that works without re-arming, otherwise **dropped** and documented |
| **Overlay** (picker on an empty pane, switcher, prompt, help) | the daemon's reply | stage grab | NORMAL, so GNOME's Alt+Tab, Super, lock and VT switching keep working | the daemon's reply; a click outside (forwarded to the daemon, which closes the overlay; the click itself is replayed only if S3 shows virtual-pointer replay works); a focus change; `revoked`; EOF | none: all keys go to the daemon. Alt+A firing its accelerator inside an overlay (NORMAL lets it through, `events.c:256-258`, `windowManager.js:1589-1601`) is **forwarded as a key**, not a new prefix, matching vase where `intercept_key` sees keys before the router (`daemon/input.rs:20-53`) |
| **Sticky** (after a prefix resize or move-tab) | the daemon's reply | **temporary `grab_accelerator`s** for exactly the repeat keys: Shift+Arrows, Shift+H/J/K/L, or Shift+, and Shift+. (`daemon/input.rs:99-130`) | – | **500 ms after the last repeat** (tmux's `repeat-time`; vase ends on "any other key", which the extension can't see without a grab, UNVERIFIED S2); a focus change; a prefix; the kill chord | none; other keys reach the app |
| **Pending launch** (up to `LAUNCH_ADOPT_TICKS = 100`, about 10 s, `pane_picker/keys.rs:16`) | the daemon's reply | **a temporary `grab_accelerator` for Esc only** | – | the window maps; the 10 s cap; Esc | none: other keys reach the apps (Linux deviation: vase swallows them, `daemon/input.rs:26-32`). The desktop stays usable, because a stage grab would also take the pointer (F4) |
| **Fullscreen focused** | the focus window is fullscreen (known locally, no round trip) | Alt+A and Alt+E **ungrabbed**; kill chord and Pause kept | – | focus leaves the fullscreen window | – |
| **Paused** | the Pause chord | only Pause and the kill chord | – | the Pause chord | – |
| **Disconnected** | EOF | every grab released except the kill chord and Pause | – | a hello | – |

Rules that apply in every state:
- **The kill chord is checked first in every stage-grab key handler**, because a NONE grab filters every binding,
  NON_MASKABLE ones included (F14). Outside a grab it is an `addKeybinding` binding with `NON_MASKABLE`, so a client
  that inhibits shortcuts can't mask it. The same goes for Pause.
- **Inside a NONE prefix, GNOME's own chords reach vase**, so Alt+A then Super+L moves the pane on Ubuntu instead of
  doing nothing (INFERENCE, S2). The Keymap row (`keymap.md:105`) gets an Ubuntu note.
- **A prefix ends before VT switching works again**: Ctrl+Alt+F3 is filtered while a NONE grab is up (F15), so the 3 s
  timeout is also what makes the console reachable during a stuck prefix.
- Keys are identified by **hardware keycode** mapped to US positions (decided: the only input source is `us`, F22;
  vase's `KeyCode` and the Keymap use physical positions). Chords are grabbed by keycode, as utwm does.

### 3.4 Core seams

"Shared core plus about nine seams", each offered upstream to CasperTeirlinck/vase one by one; a seam upstream
declines is carried as a fork patch and listed in the fork ADR (Q1.2).

| # | Seam | Why | Milestone | Upstream or fork |
|---|---|---|---|---|
| K1 | `KeyRouter::disarm()` and `Daemon::cancel_prefix()` (clears the armed router and a pending tab-number entry). The router has no timeout and no public disarm today (`input/mod.rs:149-185`). | the extension ends a prefix on timeout, `revoked` or EOF | M1a | upstream (no effect on other platforms) |
| K2 | `Daemon::key_mode()` returning Idle / Prefix / Overlay / Sticky(keys) / PendingLaunch. `modal()` (`daemon/input.rs:15-17`) leaves out the sticky modes; it stays as a wrapper. | the grab host must know what to grab | M1a | upstream |
| K3 | `Daemon::end_sticky()` | the 500 ms sticky timeout | M1a | upstream |
| K4 | `SendPrefix` and pass-through through a `Backend::send_key` hook, default no-op (`daemon/input.rs:85`) | "Alt+A, then Alt+A" and replayed keys | M1c | upstream if S3 works; otherwise only a Keymap deviation |
| D1 | The main screen comes from the backend (a `Backend::primary_display()` with today's "display at (0,0)" as the default) instead of `reconcile.rs:61`. `Daemon::new` already takes `main_screen` (`daemon/mod.rs:118`). | F11 | M1a | upstream |
| L1 | `Daemon::detach()`: save state and hide chrome without `restore()`. `save_state` is `pub(crate)` today (`daemon/mod.rs:223`). | §3.1 Stopping and Disconnected | M1a | upstream |
| P1 | Pause as a core state | Q3.4 | M6 | upstream (cross-platform) |
| P2 | Fullscreen windows unmanaged; today vase keeps them managed and only hides the chrome (`reconcile.rs:180-188`) | §1.5 | M6 | upstream, behind a config flag if needed |
| P3 | The ignore list, `[[ignore]] app = …` (no rules engine today) | Q4.1 | M6, if answered | upstream |

Not core changes: freezing (the platform loop stops calling `reconcile()` and the ticks, F8), text measuring
(`GnomePainter` answers `measure` from the Pango arrays), the optimistic mirror (`GnomeBackend`), and the lifecycle
states (`vase-gnome`).

### 3.5 Transport and the peer check

- **The daemon binds its own socket** at `$XDG_RUNTIME_DIR/vase/gnome.sock` (directory 0700), with no socket
  activation, so `SO_PEERCRED` names the daemon itself (F13). The extension reconnects with backoff.
- On connect, the extension checks that the peer's uid is the user's and that `/proc/<pid>/exe` is
  `/usr/local/libexec/vase/vase-gnome` (S9 checks GJS can read both), and the daemon's hello carries the build hash
  of the reviewed SHA, which the extension compares with the value the sudo step wrote next to it.
- **This is defence in depth only.** Any process of the same uid can already reach the compositor in other ways
  (`devtest-security.md` §5.5). The audit text says so.
- JSON lines, one connection, protocol version in the hello; a mismatch is refused on both sides.

### 3.6 Confinement and what the extension may do

- **The daemon unit claims only what works without namespaces** (F12): `NoNewPrivileges=yes`,
  `RestrictAddressFamilies=AF_UNIX`, `SystemCallFilter=@system-service` minus the network and process-spawning groups
  it doesn't need, `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`, `RestrictNamespaces=yes`,
  `RestrictRealtime=yes`. Each is checked in S10. `PrivateNetwork=`, `ProtectHome=` and the other namespace options are
  **not** claimed.
- **File and network confinement come from a root-owned AppArmor profile** for `/usr/local/libexec/vase/vase-gnome`,
  installed by Chained's sudo step: read the vase config, read and write the vase state dir, the runtime socket, no
  network beyond unix, no exec. Whether it attaches when systemd `--user` starts the binary is UNVERIFIED (S10).
- **Doctor reads the live confinement**, not the unit file: `/proc/<pid>/status` (`Seccomp: 2`, `NoNewPrivs: 1`) and
  `/proc/<pid>/attr/current` (the profile name, enforce mode).
- **The extension launches apps.** The `launch` effect runs `Shell.AppSystem` `launch`/`open_new_window` inside the
  compositor, so "the extension spawns nothing" was false. It launches only installed desktop-file ids (§3.2); it
  spawns no helper process; the CI tripwire flags any `.launch(`, `open_new_window(`, `spawn`, `Subprocess` outside
  the one reviewed function.

### 3.7 Why this one

1. **It isolates the layout logic.** A daemon panic or hang costs tiling, not the session; the extension releases every
   grab on EOF or a missed reply. The extension itself still runs in the compositor (F2), which is why it is small and
   defensive (§3.2) rather than "safe by construction".
2. **It shares the core.** vase-core with nine listed seams keeps behaviour, keymap, config and tests shared with
   macOS and Windows, and the monthly upstream merges keep helping Linux. A JS port would be a second product.
3. **The code in the compositor stays reviewable.** Plain JS with no dependencies, touching only `Meta`, `St`,
   `Shell.AppSystem`, `Main.wm` and `Main.pushModal`; the closest prior art is utwm's 516-line bridge
   (`prior-art.md` §1). Breakage across GNOME releases scales with how much shell internals an extension touches
   (`prior-art.md` §3).
4. **The socket hop is not where latency lives.** Each command is one local round trip; IBus already sends keys over
   D-Bus before mutter's keybindings (F16), and each resize waits for a Wayland configure/ack anyway. S9 measures press
   to effect under paint load, not an echo.
5. **Locks cost a reconnect, not a re-tile** (§3.1; checked at L1 in M1b).
6. **It fits the audit.** No helper process from the compositor; a daemon with seccomp options, an AppArmor profile and
   the fork's cargo-deny bans.

Rejected:
- **The JS port** (a second product: about 6 to 10 weeks, then a porting tax on every merge).
- **WebAssembly inside the shell** (option c). It gives no protection against hangs, needs clock and fs seams, and
  nobody plans to build it; its spike (S1) is dropped. The versioned protocol would still allow it.
- **A native library in the shell** (a panic aborts the compositor).
- **Socket activation** (breaks the peer check, F13).
- **Spawning the daemon from the extension** over stdio (Pop Shell's pattern): ties it to every `disable()` (F7).
- **Window-Calls-style D-Bus window control** open to any process, and any session-bus transport.
- **Keybinding-set swapping for the prefix's unbounded table** (leaky). Temporary accelerators for bounded key sets
  (sticky, Esc) are a different case and are used.
- **Changing GNOME settings at runtime** (shows up as Drift), **shadowing the session-mode file** (escapes GNOME's
  crash net), **`unlock-dialog` mode** (keeps compositor code running on the lock screen), and **leaving GNOME for
  niri** (an escape hatch only).

Costs to accept:
- A fork ADR that amends CONTEXT.md's "single daemon owns the hook" to "on Linux the hook half lives in the
  compositor", and records the seams and that Linux hides by raising, not by parking.
- A protocol, a mirror and two state machines to keep consistent, testable in Rust with a fake extension.
- Two artifacts and an AppArmor profile to install and match by version.
- A Rust build for Ubuntu.
- No estimate yet. The work covers two programs, a protocol, CI from nothing and a container harness; re-estimate
  after M1a.

---

## 4. Decision tree

### Round 1: before Milestone 0 (final)

- **Q1.1 Architecture.** Build the Linux Tiler as a thin GJS extension plus a Rust daemon (`vase-gnome`) that runs
  vase-core with the nine seams in §3.4, with the lifecycle of §3.1 and the key-ownership state machine of §3.3, and
  reject the JS port and WebAssembly in the shell?
  **Recommendation: yes.** It is the only option that keeps the layout logic out of the compositor and keeps one
  product across three platforms.
- **Q1.2 Where it lives.** Build it in the fork on a `linux-tiler` branch, as a new crate `vase-gnome/` (daemon,
  `GnomeBackend`, `GnomePainter`, protocol) and `vase-gnome/extension/` (GJS)? Write the fork ADR (process split,
  hiding by raising, the seams) before M1a merges to `main`? Offer each seam upstream on its own, and carry declined
  ones as listed fork patches?
  **Recommendation: yes.** It keeps the core shared and the fork's divergence visible.
- **Q1.3 Dev environment.**
  - Install exactly one package on the host, `mutter-dev-bin` (Ubuntu universe, OS channel; one package, no new
    dependencies, contents only `/usr/libexec/mutter-devkit`, a `.desktop` file and icons, F10), for the interactive
    devkit shell (L3)?
  - Run everything else in rootless podman images named `chained-tiler-*`, with only the checkout mounted read-only:
    the Rust toolchain from Ubuntu's archive, headless gnome-shell, `python3-dbusmock`, eslint and test clients. Never
    prune.
  - Run `gnome-shell-test-tool` only inside podman, never on the host (F10).
  - Run L3 only on private, empty system and session buses and a private `XDG_RUNTIME_DIR`, through a recipe that
    refuses to start otherwise (F18); never run the dev shell in a `user`-parent mode (no lock test at L3).
  - Use the real session only at assessment (L4).

  **Recommendation: yes.** It is the smallest host footprint that still allows a hands-on feel check, and it closes
  the two leaks the critique found.
- **Q1.4 Fork hardening and Linux CI first.** Do the spec's fork hardening (spec:866-871) in M0, before any Linux code
  is pushed: delete the three inherited `release*.yaml` workflows (F24), set `permissions: {}` at the top level,
  SHA-pin the allowed actions, turn on Dependabot alerts, and protect `main`? Then add a Linux CI job that grows with
  each milestone, with the L1 suite in it from M1b.
  **Recommendation: yes.** `release-finalize.yaml` already runs with `contents: write` on merged PRs, and agent-written
  compositor code needs CI from its first commit. Two steps are the owner's in GitHub's settings (Dependabot alerts,
  branch protection). Note: spec:868 still says "signed commits", which predates the 2026-09-24 decision to drop
  signing; the recommendation is protection without signing, and the spec line should be corrected.
- **Q1.5 Tab bar and the Ubuntu Dock.** The Dock autohides at the bottom (F19), where vase's tab bar would go. Put the
  tab bar at the **top** on Ubuntu, below GNOME's top panel, by making "top" the Linux platform default for vase's
  existing `[tabbar] position` option (no core change), and leave the Dock where it is? The alternative is to keep
  the bar at the bottom and move the Dock (to the left, say) as a Chained-owned dash-to-dock key.
  **Recommendation: yes, top.** It needs no change to the Dock or to Chained's GNOME data, and the shared
  `config.toml` keeps `position` unset so each platform picks.
- **Q1.6 Recovery from a hung shell.** Keep `kernel.sysrq=176` and install no SSH server, so the runbook's hang branch
  is Alt+SysRq+S, U, B (sync, remount read-only, reboot), rehearsed once? The alternatives are allowing more SysRq
  functions (for example 64, signalling processes, or 4, keyboard control including SAK; whether SAK kills the hung
  shell under logind is UNVERIFIED) or an SSH server to kill the shell from another device.
  **Recommendation: yes, keep 176 and no SSH.** The layout logic is out of the compositor, the extension has no loops,
  so a hang should be rare; a reboot loses unsaved work but nothing else; SSH and wider SysRq both add attack surface.
  Revisit if a hang ever happens in daily use.
- **Q1.7 Host probes for the sandbox spike.** Let spike S10 run on the desktop: transient `systemd-run --user` units
  that leave nothing behind, and, once, a test AppArmor profile loaded with sudo and removed (`apparmor_parser -R`) in
  the same sitting, with before and after checks? Neither can be tested in podman, because the result depends on the
  host's kernel settings (F12).
  **Recommendation: yes.** The sandbox claim goes into the audit, so it has to be measured where it will run.
- **Q1.8 Destination and scope.** Accept §1 as the destination and §7 as out of scope, with `"shell-version": ["50"]`
  and CI tracking 50.x point releases (F21)?
  **Recommendation: yes.**

### Round 2: owner questions after M0b (each names the spike it waits for)

- **Q2.1 Linux Keymap deviations** [S2, S3]. Accept these, each written into the Keymap as an Ubuntu note when M1c
  lands?
  - The prefix and the stack prefix end after 3 s with no key.
  - A key with no row after either prefix is swallowed (what tmux does), unless S3 shows replay works, in which case
    it passes through as today's Keymap says.
  - "Alt+A, then Alt+A" sends one Alt+A by replay if S3 shows that works; otherwise it is a deviation.
  - Sticky resize and move-tab repeat end 500 ms after the last repeat.
  - During a pending launch (up to 10 s), keys other than Esc reach the apps.
  - Alt+A and Alt+E reach a focused fullscreen app; there is no prefix over fullscreen.
  - Alt+A, then Super+L moves the pane on Ubuntu.
  - The kill chord on Ubuntu "stops tiling and releases the keyboard; the session keeps running".

  **Recommendation: yes.** Each follows from how GNOME delivers keys (F4, F14, F15, F17); S2 confirms them.
- **Q2.2 The picker on an empty pane** [S2, S3, S7]. Keep vase's auto-open picker on a focused empty pane, which on
  GNOME means a keyboard and pointer grab for as long as the empty pane has focus; a click outside closes it (and the
  click is replayed if S3 shows that works)? The alternative is a Linux deviation where the picker opens only on a key.
  **Recommendation: yes, keep auto-open**, with NORMAL `actionMode` so GNOME's own shortcuts keep working; switch to
  the alternative if the L3 feel check shows the grab gets in the way.

### Engineering defaults (not owner questions; the named spike confirms or the question comes back)

- **Transport and lifecycle** [S9]: §3.1 and §3.5 (own socket, `PartOf=graphical-session.target`, detach on SIGTERM).
- **Hiding** [S5]: raise-to-cover for hidden tabs and stack items. `inhibit_mapped()` for stack items only if S5 shows
  clean Alt+Tab and overview, sane focus, and restoration on every exit path including `disable()` and the kill chord.
- **Chrome placement** [S7]: the tab bar through `Main.layoutManager.addChrome` with `trackFullscreen: true` and
  `affectsStruts: false` (vase subtracts the strip itself); stack bars and the focus border in `global.window_group`,
  restacked on `restacked`.
- **New windows** [S6]: placed in the `configure` signal, with a move after the first frame for Xwayland or when
  `configure` doesn't fire.
- **Key identity**: keycode (decided, F22).

### Round 3: before landing in Chained (needs Milestones 1-8)

- **Q3.1 Install channel.** The extension as a root-owned copy in `/usr/local/share/gnome-shell/extensions/<uuid>`
  (on the shell's `XDG_DATA_DIRS`, F25), laid down by Chained's sudo step from `git archive <reviewed SHA>` with the
  tree hash and the daemon's build hash in the bootstrap lock; the daemon as a root-owned binary in
  `/usr/local/libexec/vase/`; its AppArmor profile in `/etc/apparmor.d/`; its user unit rendered by Chained;
  `allow-extension-installation` stays false; the UUID under a domain the owner controls?
  **Recommendation: yes.** It is the only channel that loads under F9 without loosening installation.
- **Q3.2 Loosening the lint.** Let ticket 13's GNOME lint allow exactly three extension writes: `disable-user-extensions
  =false`, the Tiler's UUID in `enabled-extensions`, and `tiling-assistant@ubuntu.com` in `disabled-extensions`? And
  mark `disable-user-extensions` **written once at the switch-over, never re-asserted** by bootstrap or `chained
  profile`, because re-asserting it would silently re-enable a Tiler that GNOME's crash unit just disabled?
  **Recommendation: yes.** With installation still off, the three old user extensions in `~/.local` stay unloaded.
- **Q3.3 After a crash.** When GNOME's crash net has flipped `disable-user-extensions` to true: doctor FAILs and names
  the recovery; `chained profile` refuses to flip it back without an explicit flag; nothing re-enables Tiling
  Assistant automatically? Note the net covers only the first 60 s (`extensionSystem.js:54-72`); a later crash logs the
  owner out and loads the Tiler again at the next login.
  **Recommendation: yes.** An automatic fallback would be a runtime settings write (Drift) and could hide a crash loop.
- **Q3.4 Rewriting audit §24** (`docs/research/security-audit.md:954-965`) and the spec:
  - condition 1: "installed by Chained's sudo step from the reviewed tree into a root-owned system directory" (not
    "by chezmoi", which can't load, F9); the compiled GSettings schema for the two NON_MASKABLE bindings is the only
    build step;
  - condition 3: "spawns no helper process; opens one unix-socket connection to a peer it checks; launches only
    installed apps the user picked, through `Shell.AppSystem`";
  - a new daemon condition: vase's Rust conditions from macOS plus the seccomp options and the AppArmor profile, as
    measured by S10, and checked live by doctor;
  - condition 5: Pause, as in Q3.5;
  - the platform limit: same-user code can reach the compositor anyway.

  **Recommendation: yes.**
- **Q3.5 Pause.** A vase-core state (P1), cross-platform, toggled by one `addKeybinding` chord with `NON_MASKABLE`
  (F14), proposed Ctrl+Super+Alt+P next to the kill chord, with a clash check? While paused the Tiler releases every
  grab except Pause and the kill chord, stops placing windows and hides the chrome. Pause is not
  `gnome-extensions disable` (that rewrites extension keys: Drift).
  **Recommendation: yes.**
- **Q3.6 Building the daemon.** On the desktop (the Upgrade Machine) inside a `chained-tiler-build` image pinned by
  digest with Ubuntu's rustc and cargo, fetching with the lock and then building locked and offline at the reviewed
  SHA, with the output hash in the bootstrap lock and no Rust on the host? An attested CI asset (the ThinkPad pattern)
  stays for later.
  **Recommendation: yes.**
- **Q3.7 The switch-over.** In one Profile switch, ordered: disable Tiling Assistant; let it restore `edge-tiling`
  (F20); then Chained writes the mutter keys it takes over (`edge-tiling` false, the Super+Arrow tile keys cleared);
  then enable the Tiler. Plus a documented one-step way back, and the "stock (Tiling Assistant)" Desktop rows dropped
  on Ubuntu?
  **Recommendation: yes.** Without the order, TA's restore races Chained's write and shows up as a doctor WARN.
- **Q3.8 Doctor.** Add: the extension is ACTIVE (not ERROR or OUT_OF_DATE); the installed tree, daemon and profile
  match their lock hashes; the unit is active; the daemon's live `Seccomp: 2`, `NoNewPrivs: 1` and AppArmor label; the
  hello's build hash matches the lock; Tiling Assistant disabled while the Tiler is enabled; FAIL on a crash flip
  (Q3.3); FAIL if `gnome-shell/{extensions,modes}` exists under a user-writable `XDG_DATA_DIRS` entry of the running
  shell, or if `~/.config/environment.d` sets `LD_PRELOAD`, `XDG_DATA_DIRS` or `GNOME_SHELL_*`.
  **Recommendation: yes.**
- **Q3.9 Debug tools.** Set `org.gnome.shell development-tools=false` on the real session once the Tiler ships, turning
  off Looking Glass (a JS prompt inside the compositor)?
  **Recommendation: yes.**

### Round 4: later features (after the first usable Tiler; mostly cross-platform)

- **Q4.1** The game ignore list as a vase-core rule (P3), matching the app id, or WM_CLASS on Linux
  (`steam_app_<id>`)?
- **Q4.2** Configurable keys in vase-core (Mac ticket 18 needs it anyway)?
- **Q4.3** A native GNOME chrome style (St, accent colour) behind vase's style seam (ADR 2026.08.21)?
- **Q4.4** Offer the Linux backend upstream once M4 works?
- **Q4.5** Bring blur-my-shell or rounded-window-corners back as system installs? They don't load today (F9), so the
  Tiler isn't tested with them. Recommendation: no.

---

## 5. Prototype plan

### Test levels

- **L0:** plain `cargo test` / `gjs -m` in the build image, with no shell. The fake extension drives `vase-gnome`
  through the protocol.
- **L1:** `gnome-shell-test-tool --headless --extension` **inside rootless podman only**, under `dbus-run-session`, with
  `TMPDIR` inside the container and a dbusmock system bus (gnome-shell's `tests/gnomeshell_dbusrunner.py`, trimmed).
  Locks, unlocks and extension toggles are tested here, and nowhere nearer the real session.
- **L3:** the devkit shell in a window on the desktop, through `just dev-shell`:
  - `env -i` with an allow-list; a private system bus (a plain `dbus-daemon` with an empty config, no services, so
    `canLock()` is false and nothing reaches logind, polkit or NetworkManager); a private session bus
    (`dbus-run-session`);
  - HOME, the XDG dirs and `XDG_RUNTIME_DIR` under a 0700 `~/.cache/tiler-dev.*`, never `/tmp` (so the crash marker
    `gnome-shell-disable-extensions` is private too); `WAYLAND_DISPLAY` as an absolute path to the host socket for the
    viewer only (UNVERIFIED, S11);
  - `GSETTINGS_BACKEND=keyfile`; a `tiler-dev` session mode with no components and only the Tiler; the extension
    symlinked from the working tree;
  - the recipe refuses to start if `DBUS_SYSTEM_BUS_ADDRESS` or `DBUS_SESSION_BUS_ADDRESS` is the host's, if the
    backend isn't keyfile, or if HOME isn't redirected.

  Clients started from a host terminal land on the real compositor; only clients the dev shell launches are inside it.
- **L4:** the real session, only at assessment.

Every L1 case ends with an enable, disable, enable cycle and asserts that nothing is left behind: no actors, signals,
sources, grabs or inhibited windows.

### M0a: environment, fork hardening and CI skeleton (no tiling)

- **Fork hardening (Q1.4)** first: delete the release workflows, `permissions: {}`, SHA-pinned actions, Dependabot
  alerts and protection on `main` (the owner's two GitHub steps).
- **S11 first:** does the devkit run under the L3 recipe with no system services? If not, L3 is dropped and the plan
  runs on L1 plus the owner's L4 assessment; that decision is written down before anything else is built.
- `chained-tiler-build` (from `ubuntu:26.04` by digest, plus Ubuntu's rustc and cargo) builds the fork and runs
  vase-core's tests on Linux.
- `chained-tiler-test` (gnome-shell, gjs, dbus, python3-dbusmock, mesa software rendering, GTK4 demo apps, `x11-apps`)
  runs `gnome-shell-test-tool --headless --extension hello.zip`.
- The Linux CI job: `cargo test`, cargo-deny and cargo audit, eslint (lockfile, `--ignore-scripts`), a metadata lint,
  and the first tripwires (`devtest-security.md` §4.4, plus `.launch(`, `open_new_window(`, `Subprocess`, `spawn`).
- A hello extension logs `enable` and `disable` and draws one St label.

**M0a exit, and how it's tested**
- The hello extension runs at L1 in CI (exit code 0) and at L3.
- Before and after, read-only on the real desktop, nothing changed: `dconf dump /org/gnome/`; `loginctl show-session
  <id> -p LockedHint`; `$XDG_RUNTIME_DIR/gnome-shell-disable-extensions` absent; `~/.local/share/gnome-shell` (tree
  hash); the mtime of `~/.config/monitors.xml`; no new owner of `org.gnome.Shell` or of a polkit or NetworkManager
  agent on the host buses.

### M0b: the spikes, in order of kill risk

One finding each in `docs/linux-spike/<S#>.md` in the fork (question, method, evidence, answer, and what it decides),
feeding round 2 and the engineering defaults. Spike code is throwaway JS, under `spikes/<S#>/`.

| Order | # | Question | Level | Feeds |
|---|---|---|---|---|
| 1 | S2 | The input contract: Alt+A via `grab_accelerator`, then synchronous `pushModal`, the next key, `popModal` on release; the Alt-release race (`global.get_pointer()` mods); NONE vs NORMAL; `notify::revoked`; the 3 s timeout; the kill chord inside the grab; Super+L inside the prefix; what the focused app sees (Ghostty, a GTK4 app, an Electron app); **fast Alt+A then `\` 5-10 ms apart into a GTK4 entry with IBus active** (nothing may land, measure press to effect); whether keys can be observed without a grab; temporary accelerators for the sticky keys and Esc; a fullscreen Xwayland test app's focus loss | L3, L1 | §3.3, Q2.1, Q2.2 |
| 2 | S3 | Do Clutter virtual-keyboard keys trigger `grab_accelerator` bindings (L1 test design)? Can one Alt+A, or a PassThrough key, be replayed to the app after the pop without re-arming? Can a click be replayed with a virtual pointer? | L1 | K4, Q2.1, Q2.2, tests |
| 3 | S9 | GJS `Gio.SocketClient` to a unix socket the daemon bound: async line reads, `Gio.Socket.get_credentials()` and `/proc/<pid>/exe`, EOF, reconnect with backoff; **press-to-effect time** of a prefix command through a stub Rust daemon under paint load | L1 | §3.5, latency claim |
| 4 | S10 | On the host (Q1.7): transient `systemd-run --user` units with each namespace option (fails or silent?) and each seccomp option (`Seccomp: 2` in `/proc/<pid>/status`?); a test AppArmor profile for a test binary started by systemd `--user` (does `attr/current` show it?) | host, read-mostly | §3.6, Q3.4 |
| 5 | S4 | `move_frame` then `move_resize_frame` on GTK4, libadwaita, Electron, Ghostty and Xwayland (`xterm`): settle time and the size reached (cell-sized terminals), **at scale 1 and 1.5, with `scale-monitor-framebuffer` and `xwayland-native-scaling`, on two monitors with the primary at x=2560**; whether headless virtual monitors accept scale 1.5 or the runner needs `ApplyMonitorsConfig` | L1 | settling, D1, M1b |
| 6 | S6 | `Meta.Window::configure` on a Wayland and an Xwayland window: order relative to `window-created`, and whether a rect set there lands the first frame | L1 | new-window default |
| later (M1c) | S7 | St chrome in `uiGroup` vs `window_group` plus re-stacking on `restacked`; click delivery; `trackFullscreen`; Pango advance arrays vs whole-string widths | L1 | chrome default |
| later (M3) | S5 | `inhibit_mapped()`: Alt+Tab, overview, focus, xdg `suspended`, restore on `disable()` and the kill chord | L1 | hiding default |
| later (M1b) | S8 | Lock and unlock, and a rebase by toggling another extension: what `disable()`/`enable()` cost, and that the daemon stays frozen | **L1 only** | §3.1 |

S1 (WebAssembly) is dropped.

**M0b exit:** rows 1-6 have findings, round 2 can be asked, and the engineering defaults are confirmed or reopened.

### M1a: the Rust half, at L0 only

**What gets built**
- The protocol v1 spec: hello (version, compositor-instance id, build hash, chrome sizes), snapshot, window, stacking,
  focus, display (with primary index and scale), work-area and fullscreen deltas, keys, clicks, measurements, effects
  batch, key mode, grab list, heartbeat, draw lists for empty-pane placeholders, the focus border and the list panel.
- The key-ownership state machine (§3.3) and the lifecycle (§3.1), written into the spec.
- `vase-gnome`: `GnomeBackend` on an optimistic mirror, `GnomePainter` emitting draw lists and measuring from Pango
  arrays, the lifecycle states, SIGTERM as detach.
- The core seams K1, K2, K3, D1 and L1, each a separate commit offered upstream.
- A fake extension in Rust.

**Tests (L0)**
- A snapshot, then Alt+A and `\`, gives the expected effects and key modes.
- The state machine: every state's entry and exit, the 3 s timeout calling K1, sticky ending through K3, pending
  launch grabbing Esc only, fullscreen focus ungrabbing the prefixes, Disconnected releasing grabs.
- The mirror converges after random effects and delayed or reordered deltas (property test).
- Disconnect, then the same instance reconnects with a changed snapshot: no effect is emitted while disconnected, and
  only closed windows are forgotten. A new instance id re-matches by `(app, title)`.
- SIGTERM writes `state.json` and emits no `set_frame`.
- The primary display at x=2560 gets the tab bar.
- A version mismatch is refused.

**Exit:** re-estimate the rest of the plan.

### M1b: the extension without keys or chrome, at L1

**What gets built:** connect with backoff and the peer check; hello, snapshot and deltas from Meta signals; effects
(look the window up again each time); settling, utwm-style, with the achieved frame fed back; the try/finally
teardown; two monitors with mixed scale; a test-only control hook so L1 can dispatch commands without keys.

**Tests (L1, in CI)**
- Two windows, then a split command, give the expected frames within a settle tolerance, on a scale-1 primary at
  x=2560 and a scale-1.5 secondary.
- Lock twice, toggle another extension, restart the unit: no frame changes, and the layout is unchanged.
- `kill -9` on the daemon leaves the windows in place; the restarted daemon re-adopts from `state.json`.
- A throw injected into each teardown step still leaves no grab, no inhibited window and no chrome.
- An effect on an unknown or unmanaged id is ignored.

### M1c: keys and the picker

**What gets built:** Alt+A through `grab_accelerator`; the kill chord and Pause placeholder through `addKeybinding`
with `NON_MASKABLE` and a compiled schema; the state machine's grabs; a small "PREFIX" indicator; the picker list
chrome with Pango measurements; seam K4 if S3 allows.

**Features:** prefix then Arrows (focus); `\` and `-` (split, with the picker on the new empty pane); pick an existing
window to fill the pane; Esc in the picker closes the pane; closing a window collapses its pane; windows already open
are adopted at start. `:close` waits for the prompt in M2.

**Tests**
- **L0 (gjs):** keycode translation and the protocol codec.
- **L1:** prefix+`\` with two windows gives the expected frames; Esc in the picker gives back one pane; the kill chord
  pops the modal and ungrabs; `kill -9` on the daemon while the prefix is armed releases the grab within 250 ms; the
  3 s timeout releases it; a shortcut-inhibiting client cannot mask the kill chord; Alt+A inside the picker is
  forwarded, not re-armed.
- **L3:** the owner's feel check of the prefix in Ghostty and a browser. The Keymap's Ubuntu notes (Q2.1) are written.

### M2: tabs

**Features:** `c`, `.`, `,`, `l`, `1-9` with multi-digit entry, Shift+, and Shift+. (move tab, with sticky repeat);
`t` rename and the `:` command line through the prompt overlay; `z` zoom; `x` break pane; new windows become tabs;
off-workspace windows get the `◇` mark (with `workspaces-only-on-primary=true`, windows on the 4K monitor are on every
workspace, F11).

**Tests**
- **L1:** after a tab switch every window of the current tab is above the hidden tab's (`sort_windows_by_stacking`);
  multi-digit entry within 120 ms; sticky move-tab ends after 500 ms; the prompt takes text and Esc cancels; `:close`
  on a window and on an empty pane; a re-login (a new compositor instance) restores the layout.
- **L3:** tab-switch flicker with the hiding default.

### M3: stacks and Alt+E

**Features:** `s` stackify, `[` and `]`; the Alt+E prefix with the nested table (`bindings_nested`,
`keymap.rs:96-110`); stack items hidden by the hiding default (S5 decides on `inhibit_mapped()`); transients stay above
their parents.

**Tests (L1):** the selected item is visible and the others covered or inhibited; every exit path (kill chord,
`disable()`, a throw in teardown, daemon death) brings every window back to visible and mapped.

### M4: the tab bar and the rest of the chrome

**Features:** the tab bar at the position from Q1.5 (powerline, all screens' tabs numbered flat, zoom `∗`, prefix
dot); stack bars, focus border, window switcher (`w`), help sheet (`?`); clicks through `Daemon::click`; the chrome
hidden over fullscreen windows; crisp at scale 1 and 1.5.

**Tests**
- **L0 (Rust):** draw lists from known models (golden JSON).
- **L1:** actor geometry matches the draw list; daemon widths match Pango within tolerance for Latin, emoji and CJK
  titles; virtual-pointer clicks on a tab select it; a fullscreen window hides the bar.
- **L3:** visual check in light and dark, at scale 1 and 1.5.

### M5: config shared with vase

**Features:** a Linux `paths.rs` (`$XDG_CONFIG_HOME/vase/config.toml`, `$XDG_STATE_HOME/vase/state.json`);
`Config::ensure` writes `docs/vase.example.toml` unchanged; `favorites` written back; `[[app_focus]]` chords grabbed
from config; `[theme]` and `[tabbar]` applied; reload.

**Tests:** **L0:** vase-core's config tests run on Linux unchanged; one fixture parses identically for the three
platforms. **L1:** an `app_focus` chord activates a launched test app; a change applies on reload; a `launch` effect
with an id that isn't installed is refused.

### M6: Pause, fullscreen and games

**Features:** Pause as in Q3.5 (seam P1); fullscreen windows unmanaged (seam P2: `is_fullscreen()`, plus
monitor-sized Xwayland windows); the game ignore list (P3) if Q4.1 is answered.

**Tests**
- **L1:** a shortcut-inhibiting client still gets Alt+A, and the Pause chord still works (it is NON_MASKABLE through
  `addKeybinding`); while paused Alt+A reaches a test client; a fullscreen Xwayland window is never moved and gets
  Alt+A; resume re-renders the saved layout.
- **L3, then L4:** a real game under Steam (Xwayland), including an accidental Alt+A while windowed.

### M7: monitor changes

**Features:** stable display ids from the connector name; `monitors-changed` and `workareas-changed` handling,
including a primary that moves; Shift+[ and Shift+] send a tab to another monitor.

**Tests:** **L1:** the track's own runner with two virtual monitors and hotplug (utwm's pattern; the test tool
hard-codes one): tabs stay on their screen, a hot-unplug moves them sensibly, the bar follows the primary.
**L4:** the owner's monitors, during the assessment.

### M8: assessment readiness (before round 3 is acted on)

- **CI complete:** the full L1 suite in `container: ubuntu:26.04@sha256:…`; shexli pinned to a commit; all
  tripwires (processes, network other than the one unix socket, D-Bus, file writes, clipboard, `eval`, patching
  polkit or the unlock dialog, launches outside the reviewed function).
- **Confinement:** the AppArmor profile and unit options from S10, with the doctor checks of Q3.8 drafted.
- **Reviews and docs:** the fork ADR merged with the seams list and each seam's upstream status; a full code review
  of the extension and the daemon at the candidate SHA.
- **The recovery runbook, both branches rehearsed once:**
  - crash or stuck grab, main loop alive: wait out the 3 s timeout or press the kill chord; Ctrl+Alt+F3, log in, then
    `gsettings set org.gnome.shell disable-user-extensions true` or `sudo mv` the extension directory;
  - hang, main loop dead: Alt+SysRq+S, U, B (F15; or whatever Q1.6 decides).

---

## 6. What changes in Chained when the Linux Tiler lands

Each item depends on round 3. None of it is Chained v1 work.

- **Catalogue entry.** The Tiler entry gains an Ubuntu installation made of three artifacts from one reviewed SHA: the
  extension tree, the daemon and its AppArmor profile. It also gets its unit, and doctor and ledger treatment like
  any tool (spec:639).
- **Install channel (Q3.1).**
  - The sudo step copies the extension into `/usr/local/share/gnome-shell/extensions/<uuid>`, the daemon into
    `/usr/local/libexec/vase/`, and the profile into `/etc/apparmor.d/`, and loads the profile.
  - Chained renders the systemd user unit (no socket unit).
  - The bootstrap lock records the reviewed SHA, the extension tree hash, the daemon hash, the profile hash and the
    build image digest.
  - The existing guided GNOME log-out step runs after each install or bump.
  - Upgrade moves the SHA and checks each 50.x update; an unbumped Tiler fails safe by not loading, and doctor FAILs.
- **Lint and data.** `data/gnome.toml`'s `[never]` list keeps refusing `disable-extension-version-validation`. It
  allows exactly the three writes in Q3.2, with `disable-user-extensions` marked write-once, plus the mutter keys
  Chained takes over at the switch-over (Q3.7). `docs/tools/gnome.md`'s "Chained writes no other extension key"
  paragraph is rewritten, and so is its claim that bootstrap and `chained profile` write every key.
- **Assessment.** The audit's §24 is rewritten (Q3.4), and an `[assessment.*]` entry goes in
  `data/assessments.toml`. Policy rule 3: it does not enter the Standard before this.
- **Tiler config.** The same modify target as on the Mac (spec:373), at `~/.config/vase/config.toml`, with
  `[tabbar] position` left unset (Q1.5). `state.json` is never committed (spec:668).
- **Keymap rows** (`docs/keymap.md`):
  - "Where it is set" for Ubuntu becomes "the Tiler (its GNOME Shell extension)";
  - the Ubuntu notes from Q2.1 (timeout, pass-through, sticky, pending launch, fullscreen, Super+L, the kill chord's
    meaning);
  - a Pause row (Q3.5);
  - the Desktop rows that say "stock (Tiling Assistant)" are dropped or changed (Q3.7);
  - the reservation lint keeps covering Alt+A and Alt+E and gains the Pause chord.
- **Dropping Tiling Assistant.** It goes into `disabled-extensions` in the ordered Profile switch (Q3.7), with a
  one-step way back. The text "keeps Tiling Assistant until…" in the spec (653, story 107), `docs/keymap.md:27,68` and
  `docs/tools/gnome.md:26` is updated. Ubuntu's `ubuntu.json` mode file is never edited.
- **Doctor checks** as in Q3.8. **Manual checklist** (story 119): log out and in, ACTIVE state, prefix feel, Pause with
  a game, lock, toggle another extension, suspend, hotplug, both monitors at their scales, and both recovery branches.
- **The spec's fork-hardening line** (spec:868, "signed commits") is corrected to match the 2026-09-24 decision.

---

## 7. What stays out

**Out of Chained v1** (spec:1039, unchanged by this map):
- Everything in §5 and §6.
- Chained v1 keeps Tiling Assistant and today's extension keys.
- The one doc-only fix that could land early is the wording contradiction in audit §24.1 (F9). It is optional and
  doesn't change behaviour.

**Out of the first usable Linux Tiler** (later Tiler-track work or never):
- **Versions:** any GNOME other than 50.x, and any Ubuntu other than 26.04 (F21); the X11 session (gone).
- **Distribution:** publishing on extensions.gnome.org; the Windows CI build and the Mac items, which have their own
  legs of the track (spec:1056: Ubuntu first).
- **Features:** configurable keys (Q4.2); a native GNOME chrome style (Q4.3); animations; Dock badges
  (`badged_apps`, which stays empty); managing GNOME workspaces or backing tabs with hidden workspaces; mouse-drag
  tiling; the prefix over fullscreen windows.
- **Mechanisms:** `ExternalConstraint` (read-only in 50.x); off-screen parking (F5); `unlock-dialog` mode; socket
  activation; WebAssembly in the shell; any D-Bus interface on the session bus; systemd namespace sandboxing on this
  kernel configuration (F12).
- **Coexistence:** running alongside Tiling Assistant (two tilers fight); testing with blur-my-shell or
  rounded-window-corners (they don't load, F9); desktops other than the owner's.

---

## Critique issues and where they were addressed

| # | Issue | Addressed in |
|---|---|---|
| 1 | Lock and unlock are not free | F7; §3.1; M1a and M1b tests; S8 at L1 only |
| 2 | vase-core is not unchanged | §3.4 seams table; §1.1; Q1.1, Q1.2 |
| 3 | Key ownership under a grab | §3.3; K1-K4; Q2.1, Q2.2; S2, S3; M1a tests |
| 4 | The owner's monitors | F11; §1.3; D1 in M1a; mixed scale in S4 and M1b; M7 only hotplug |
| 5 | Dev setup touches the real session | F10, F18; Q1.3; §5 test levels; S11 first; M0a exit check |
| 6 | Milestones too big, wrong order | S1 dropped; spikes by risk; M1a/b/c; hardening and CI in M0a (Q1.4); estimate dropped |
| 7 | The systemd sandbox | F12; §3.6; S10; Q1.7; Q3.4, Q3.8 |
| 8 | Peer check vs socket activation | F13; §3.5 (own socket, defence in depth) |
| 9 | `NON_MASKABLE` on accelerators | F14; §3.3; M1c; Q3.5; M6 test |
| 10 | Recovery assumes a VT | F15; §1.6; Q1.6; M8 runbook branches |
| 11 | Chained's writes undo the crash net | Q3.2 (write-once), Q3.3, Q3.8; §6 |
| 12 | Glyph table vs Pango | §3 chrome layout; M4 tests |
| 13 | Grab cost in fullscreen | F17; §3.3 "Fullscreen focused"; Q2.1; S2; M6 |
| 14 | IBus before the prefix | F16; S2 case; S9 press-to-effect |
| 15 | The extension spawns nothing | §3.2, §3.6; Q3.4 condition 3; tripwire in M0a; M5 test |
| 16 | Stale mirror inside a dispatch | §3 "Reads are local"; M1a property test |
| 17 | §1.6 overclaims; teardown | §1.6; §3.1, §3.2; M1b throw test; M3 exit paths |
| 18 | Questions facts settle | F21, F22, F9; Q1.3 facts; Q2.x moved to engineering defaults; Q1.5 (Dock); Q3.7 ordering; Q4.5 |

---

## Sources

The five notes in this folder and `critique.md` carry the full citations. Re-checked for this revision (copies in
`scratch/critique/`, from https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/50.1/ and
https://gitlab.gnome.org/GNOME/mutter/-/raw/50.1/):

- gnome-shell 50.1:
  - `js/ui/extensionSystem.js:190-242` (rebase), `:775-787` (`_sessionUpdated`), `:545-556,705-730`
  - `js/ui/main.js:234-238` (ScreenShield when `canLock()`), `:697-706` (`pushModal`: NONE default,
    `disable_unredirect`)
  - `js/ui/windowManager.js:1588-1601` (`_filterKeybinding`)
  - `js/ui/screenShield.js:145-148,172-175`
  - `js/misc/loginManager.js:141-160`
  - `js/misc/fileUtils.js:19-22`
- mutter 50.1:
  - `src/core/keybindings.c:970-980` (handler flags from `add_keybinding`), `:1448-1456` (NON_MASKABLE test),
    `:2670-2686` (VT switching), `:2758-2766` (`external-grab` handler)
  - `src/core/events.c:160-175,250-262`
  - `src/core/window.c:8713-8720`
- `/usr/bin/gnome-shell-test-tool:40-60,100-105` (desktop, read-only).
- vase-chained @ `022340f`:
  - `vase-core/src/input/mod.rs:140-185`
  - `vase-core/src/daemon/input.rs:10-62,80-90,95-130`
  - `vase-core/src/daemon/effects.rs:60-124`
  - `vase-core/src/daemon/pane_picker/keys.rs:10-20`
  - `vase-core/src/daemon/reconcile.rs:55-65,128,175-190`
  - `vase-core/src/daemon/mod.rs:118,160-172,223`
  - `vase-core/src/daemon/bar.rs:75`
  - `vase-core/src/chrome/paint.rs:50`
  - `vase-core/src/config.rs:178-195`
  - `vase-core/src/state.rs:31,38,49`
  - `vase-windows/src/bin/vase.rs:31,74-78,189`
  - `docs/vase.example.toml:25-40`
  - `.github/workflows/release-finalize.yaml:1-20`
- Chained:
  - `docs/adr/0001-tiler-is-a-fork-of-vase.md`
  - `.scratch/chained-v1/spec.md:107,119,373,638-654,862-880,1039,1056`
  - `docs/research/security-audit.md:954-965`
  - on the ticket-14 branch (`.claude/worktrees/14-retire`): `docs/keymap.md:27,35,66-131`,
    `docs/tools/gnome.md:18-26`, `data/gnome.toml:28-41`
  - memory note `no-commit-signing.md` (2026-09-24)
- Desktop, read-only: `~/.config/monitors.xml`; `gsettings get` for `org.gnome.mutter experimental-features`,
  `org.gnome.shell.extensions.dash-to-dock dock-position`, `org.gnome.desktop.input-sources sources`;
  `sysctl kernel.sysrq kernel.apparmor_restrict_unprivileged_userns`; `systemctl --user is-active
  graphical-session.target`; `command -v sshd` (none); `dpkg -l mutter-dev-bin` (not installed).
- Man pages: `man systemd.exec` (systemd 259, `scratch/critique/systemd.exec.txt`), `man 7 unix`.
- Not re-fetched for this revision: https://wiki.ubuntu.com/StableReleaseUpdates/GNOME (F21), Tiling Assistant's
  `extension.js:166-169` (F20), `meta-wayland-input.c:117-131`, `clutter-grab.c:85-120`, `display.c:3182-3191`
  (all VERIFIED by the critique).

## Owner answers, round 1 (2026-11-07)

- **Q1.1 Architecture:** yes, a thin GNOME Shell extension plus one Rust process running the fork's vase core. Added by the owner, the same day: keep it Linux-native and lean ("not a lot of layers in between"), it must also work across the owner's other devices, and it must cause no trouble with video games. So:
  - Exactly two pieces: the extension inside GNOME Shell and the `vase-gnome` process. No D-Bus broker, helper or extra service beyond the one `systemd --user` unit.
  - Keys are caught inside GNOME Shell, never round-tripped for the prefix decision, so input latency stays the compositor's own.
  - One shared Rust core keeps behaviour, keys and config the same on Linux, macOS and Windows. The core seams stay compositor-agnostic, so a later KDE or wlroots front end would be another thin layer over the same core (out of v1).
  - Games: the Tiler steps fully aside when it is off, paused or a window is fullscreen: the extension releases every grab and accelerator except the on/off and kill chords, and moves no window. The on/off switch moves forward to M1c (it was part of M6); the game ignore list stays in M6.
- **Q1.2 Where it lives:** yes, the fork, branch `linux-tiler`, `vase-gnome/` and `vase-gnome/extension/`; each core seam offered upstream.
- **Q1.3 Dev environment:** yes, `mutter-dev-bin` on the host (the owner installs it), everything else in `chained-tiler-*` podman images, the devkit only on private buses.
- **Q1.4 Fork hardening first:** yes, in M0a. The GitHub settings (Dependabot alerts, protecting `main` without signed commits) are the owner's clicks.
- **Q1.5 Tab bar:** top on Linux, through vase's existing `[tabbar] position`.
- **Q1.6 Recovery:** keep `kernel.sysrq=176`, no SSH server. The owner's emphasis: the Tiler must be easy to switch off and on (one chord, and the systemd unit), so a misbehaving Tiler is turned off rather than recovered from.
- **Q1.7 Host probes for S10:** yes, transient user units and one test AppArmor profile, removed in the same sitting.
- **Q1.8 Destination and scope:** yes, GNOME 50 only, §1 and §7 as written.
