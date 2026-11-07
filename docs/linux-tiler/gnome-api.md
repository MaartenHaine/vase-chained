# GNOME Shell 50 / Mutter 50 APIs for the Linux Tiler (vase-chained as a GNOME Shell extension)

Lens: what a keyboard-driven, tmux-style manual tiler (vase's model: tabs, panes, stacks, the
Alt+A prefix and the Alt+E stack prefix) can and cannot do from inside GNOME Shell 50.1 on Wayland,
the version on the owner's desktop (Ubuntu 26.04, `gnome-shell 50.1-0ubuntu1.5`,
`libmutter-18-0 50.1-0ubuntu2.5`; the Mutter API version is **18**).

Research only. Nothing was installed or changed, and no shell or nested session was run.

How the facts were checked:
- **Typelib dumps.** `/usr/lib/x86_64-linux-gnu/mutter-18/Meta-18.typelib`, `Clutter-18`,
  `/usr/lib/gnome-shell/Shell-18.typelib` and `St-18` were read with GIRepository from `gjs 1.88`
  (`scratch/dump.js`, `scratch/fields.js`). These are the exact APIs the desktop exposes.
- **Shell JS.** gnome-shell's JS was extracted from the installed `libshell-18.so` gresource into
  `scratch/shelljs/org/gnome/shell/...`. Paths below written as `shell:ui/x.js:N` point there. They
  were checked against upstream 50.1 where it matters.
- **Mutter C.** Mutter 50.1 C sources were fetched from gitlab.gnome.org into `scratch/mutter/`.
  Paths written as `mutter:src/...:N` refer to tag 50.1.
- **Prior art.** Ubuntu's Tiling Assistant came from the system install (v54, shell 48-50).
  PaperWM, Forge, Pop Shell and Tiling Shell were read from GitHub tarballs in `scratch/prior/`.

---

## 0. Headline findings (the things that change the plan)

1. **Hiding windows by moving them off-screen does not work on GNOME.** Mutter's
   `constrain_partially_onscreen` always keeps 10–75 px of a window on screen, whether or not the
   move is a user op. vase's macOS ADR depends on off-screen parking (it is specified but not
   implemented). On GNOME the Tiler needs another way to hide a window:
   - `Meta.Window.raise()` to occlude, which is vase's current behaviour and what Forge does;
   - `minimize()` with `Main.wm.skipNextEffect()`;
   - the **new in Mutter 50** `Meta.Window.inhibit_mapped()` / `uninhibit_mapped()` (§4.4).
   Sources: `mutter:src/core/constraints.c:2020-2098`, and `:1879-1900` for fully-onscreen being
   skipped only for user ops.
2. **Mutter 50 added external window constraints, but GNOME 50 extensions can't use them.**
   `Meta.ExternalConstraint` and `Meta.Window.add_external_constraint()` let a Tiler stop apps
   from resizing out of their tile. They were added for GNOME Kiosk (mutter
   [!4692](https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/4692), commit `17b6a69868`,
   2025-12-02). In 50.x a JS implementation can read `info.new_rect` but **cannot replace it**:
   GObject-Introspection ignores `(inout)` on struct fields. The fix,
   `meta_external_constraint_info_set_rect()` (commit `f40380f6cf`, MR !5187), is in **51.0** but
   in no 50.x tag up to 50.6. For a GNOME 50 extension this is read-only; plan to use it on 51+.
3. **The prefix mode means `Main.pushModal`, which takes Wayland keyboard focus away from the
   app.** Without a stage grab, keys go to the Wayland client and the shell's actors never see
   them. A grab sets the Wayland keyboard and pointer focus surface to NULL, so the focused app
   gets `wl_keyboard.leave`, then `enter` when the grab is dismissed. Alt+Tab works the same way,
   so apps cope, but it is a visible difference from the macOS event tap. Sources:
   `mutter:src/core/events.c:70-85,257,282`; `mutter:src/wayland/meta-wayland-input.c:104,118-131`.
4. **With Chained's current settings, the extension won't load.**
   - `allow-extension-installation=false` makes GNOME Shell skip `~/.local/share/gnome-shell/extensions`
     entirely. This is upstream behaviour (upstream `js/ui/extensionSystem.js:711-712` at 50.1).
   - The desktop also has `disable-user-extensions=true`. That limits enabling to **session-mode**
     extensions only (`shell:ui/extensionSystem.js:550-554`), so `enabled-extensions` is ignored,
     even for an extension installed system-wide.
   - Chained's lint refuses to write `disable-user-extensions`.
   - Audit condition 1 ("installed by chezmoi") assumes a per-user install, which can't load while
     installation is off.

   This is a policy decision the Tiler track has to make (§10).
5. **GNOME 50 is Wayland-only.** The X11 session and backend were removed in Mutter 50 (!4505).
   gjs.guide says "GNOME Shell 50 removed X11 support". The only X11 windows left are
   **Xwayland clients** (`get_client_type() === Meta.WindowClientType.X11`): many games, Steam,
   older apps. "X11 vs Wayland" is now a per-window question, not a per-session one.
6. **Useful new 49/50 APIs for a tiler:**
   - `Meta.Window.get_min_size()` / `get_max_size()` (50, mutter !4917; its commit message names
     tiling extensions as the use case);
   - `inhibit_mapped()` (50);
   - `set_maximize_flags()`, `set_unmaximize_flags()`, `hide_from_window_list()`, `set_type()` (49);
   - the `Meta.Window::configure` signal carrying a `Meta.WindowConfig`, emitted before a Wayland
     toplevel's first configure, which allows placement with no flicker (§4.3);
   - `Clutter.KeyController` (present in 50.1; the documented way from 51 on).

---

## 1. Extension structure (ESM, GNOME 45+)

- **The class.** `extension.js` default-exports a subclass of `Extension` from
  `resource:///org/gnome/shell/extensions/extension.js`. It has `enable()`, `disable()`,
  `getSettings()`, `openPreferences()`, `this.uuid`, `this.dir`, `this.metadata`, and an
  `InjectionManager` for overriding shell methods (`shell:extensions/extension.js:1-120`).
- **Imports.**
  - GI modules: `import Meta from 'gi://Meta'`, likewise `Clutter`, `St`, `Shell`, `Gio`, `GLib`.
  - Shell modules: `import * as Main from 'resource:///org/gnome/shell/ui/main.js'`.
  - Never `Gtk`, `Gdk` or `Adw` in the shell process, and never `Meta`, `Clutter`, `St` or `Shell`
    in `prefs.js` (review guidelines).
- **Lifecycle rules** (gjs.guide review guidelines):
  - Create nothing in the constructor or at module scope (no GObjects).
  - Everything is created in `enable()` and destroyed or disconnected in `disable()`: actors,
    signals, GLib sources, keybindings (`Main.wm.removeKeybinding`), grabs, injected overrides.
  - From GNOME 51 an async `disable()` throws.
- **metadata.json.**
  - Required: `uuid`, `name`, `description`, `shell-version`, plus `settings-schema` if
    `getSettings()` is used.
  - `session-modes` defaults to `['user']` (`shell:ui/extensionSystem.js:394`). Leave out
    `unlock-dialog`: a tiler has no business on the lock screen, and the guidelines require
    keyboard signals to be disconnected there.
- **The version check.** `_isOutOfDate()` compares only the **major** version: any
  `shell-version` entry that starts with `"50"` matches (`shell:ui/extensionSystem.js:409-411`).
  - With `disable-extension-version-validation=false` (the default), a mismatch sets the
    extension to `OUT_OF_DATE` and it does not load.
  - Tiling Assistant ships `["48","49","50"]`. PaperWM and Forge ship `"45"…"51"`.
  - For the owner's desktop `["50"]` is enough. Add `"51"` only after testing on 51; the
    guidelines forbid claiming future versions.
- **Crash protection.** For 60 s after extensions are enabled, the shell keeps
  `$XDG_RUNTIME_DIR/gnome-shell-disable-extensions` (`shell:ui/extensionSystem.js:53-73`). If
  gnome-shell fails in that window, `org.gnome.Shell@.service`'s `OnFailure=` runs
  `org.gnome.Shell-disable-extensions.service`, which runs
  `gsettings set org.gnome.shell disable-user-extensions true`
  (`/usr/lib/systemd/user/org.gnome.Shell-disable-extensions.service:14`). That is one possible
  reason the desktop has `disable-user-extensions=true` today.
  - On Wayland a gnome-shell crash ends the whole session and all its apps; there is no Alt+F2
    `r` restart any more (the `restart` signals were removed in 50, per gjs.guide).
  - **An extension bug is therefore session-fatal.** The "kill chord" has no in-process equivalent
    (§9).
- **The dev loop** (for later; not run here):
  - From 49 on, nested sessions use `dbus-run-session gnome-shell --devkit --wayland`
    (gjs.guide 49 page).
  - From 50, `gnome-shell-test-tool --extension <zip>` installs and enables an extension zip in a
    headless shell (gjs.guide 50 page; shell NEWS !3986). That is a CI path for smoke tests.
  - `gnome-extensions install --print-uuid` was added in 50.

## 2. GSettings schemas

- **Lookup.** `getSettings()` reads `<extdir>/schemas/` if it exists and otherwise the default
  system source, which is what system-installed extensions use (`shell:extensions/sharedInternals.js:92-107`).
  The directory needs a compiled `gschemas.compiled`.
  - Since GNOME 44, `gnome-extensions install` and extensions.gnome.org compile it automatically
    (gjs.guide preferences page).
  - An install by chezmoi or by copying has to run `glib-compile-schemas schemas/` itself. That
    matches audit condition 1: "the only build step is compiling its GSettings schema".
- **Naming.** Schema id and path under `org.gnome.shell.extensions` and
  `/org/gnome/shell/extensions/`; file named `<schema-id>.gschema.xml` (review guidelines).
- **Keybinding keys** must be type `as` (string arrays of accelerators, e.g. `['<Alt>a']`) for
  `Main.wm.addKeybinding` / `Meta.Display.add_keybinding(name, settings, flags, handler)`
  (typelib). Changing the key at runtime rebinds live.
- **Fit with Chained.** Chained already writes dconf for GNOME (data/gnome.toml), so "the Tiler's
  config" on Linux could be dconf keys under the extension's schema, managed modify-style like the
  other GNOME keys. The alternative is a JSON or TOML file in `~/.config` read with `Gio.File` and
  watched with `Gio.FileMonitor`. GJS has no TOML parser, so vase's TOML would need a JS parser
  vendored in, or a switch to JSON. GSettings is the native path for keybindings (§3).

## 3. Keybindings and Shell.ActionMode

**Global chords (Alt+A, Alt+E, the Pause key):**

```js
Main.wm.addKeybinding('prefix', settings, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
    Shell.ActionMode.NORMAL /* | Shell.ActionMode.OVERVIEW */, handler);
// disable():
Main.wm.removeKeybinding('prefix');
```

- `addKeybinding` calls `global.display.add_keybinding()`, then
  `allowKeybinding(name, modes)` (`shell:ui/windowManager.js:1078-1092`).
- **The filter.** `_filterKeybinding` drops a binding whose allowed modes don't include
  `Main.actionMode`, and drops **every** binding while the action mode is `NONE`
  (`shell:ui/windowManager.js:1589-1602`).
- **`Shell.ActionMode` values** (typelib): `NONE=0, NORMAL=1, OVERVIEW=2, LOCK_SCREEN=4,
  UNLOCK_SCREEN=8, LOGIN_SCREEN=16, SYSTEM_MODAL=32, LOOKING_GLASS=64, POPUP=128, ALL=-1`.
  - Use `NORMAL` for the prefix. Adding `OVERVIEW` is optional.
  - Never `POPUP` or `SYSTEM_MODAL`, so the prefix doesn't fire over shell menus or polkit dialogs.
- **`Meta.KeyBindingFlags`** (typelib): `NONE, PER_WINDOW, BUILTIN, IS_REVERSED, NON_MASKABLE,
  IGNORE_AUTOREPEAT, NO_AUTO_GRAB, CUSTOM_TRIGGER, TRIGGER_RELEASE`.
- **Mutter runs keybindings before Wayland delivery.** Alt+A therefore never reaches Ghostty, tmux
  or zsh (`mutter:src/core/events.c:249-258`).
  - The exception: when the focused window **inhibits shortcuts** (`zwp_keyboard_shortcuts_inhibit`,
    used by VMs, remote-desktop clients and some games through Xwayland grabs), the binding is
    skipped unless the handler has `NON_MASKABLE` (`mutter:src/core/keybindings.c:1452-1460`).
  - **Design choice:** make the Pause or escape binding `NON_MASKABLE`. Leave the prefix maskable,
    so a VM or game that inhibits shortcuts gets Alt+A. That is a natural "game mode".
- **Schemaless alternative.**
  - `global.display.grab_accelerator('<Alt>a', flags)` returns an action id.
  - `Meta.external_binding_name_for_action(id)` gives its name, and
    `Main.wm.allowKeybinding(name, modes)` allows it.
  - Activation arrives on the `global.display` signal `accelerator-activated(action, device, timestamp)`;
    release with `ungrab_accelerator(id)`.
  - Used by `shell:ui/shellDBus.js:322-345`, PaperWM (`prior/PaperWM/keybindings.js:560-615`) and
    Forge (`prior/forge/lib/extension/keybindings.js:112-131`). It lets the keys come from a config
    file instead of GSettings.
  - The shell's D-Bus `GrabAccelerator` is restricted to allowed senders such as
    gnome-settings-daemon. That is part of why an external app (vase as a separate daemon) can't
    take global keys on GNOME Wayland.
- **Clash check.** `<Alt>a` and `<Alt>e` are not bound in GNOME's default wm or media keys. A
  `grab_accelerator` that fails with `NONE` signals a clash; the Tiler should report it, not fail
  silently.

## 4. Windows (Meta.Window, Mutter 18)

### 4.1 Discovery, identity and lifecycle signals

- **Enumeration.** `global.display.list_all_windows()`, `global.get_window_actors()`
  (`Meta.WindowActor`, `.get_meta_window()`), `workspace.list_windows()`,
  `global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null)` (MRU order) and
  `global.display.sort_windows_by_stacking()`.
- **Identity** (typelib):
  - `get_id()`: a u64, stable for the session, which serves as vase's `WindowId`.
  - Also `get_stable_sequence()`, `get_pid()`, `get_wm_class()`, `get_gtk_application_id()`,
    `get_sandboxed_app_id()`, `get_title()`.
  - The app id: `Shell.WindowTracker.get_default().get_window_app(w).get_id()`, the `.desktop` id.
    Use it for vase's `(app, title)` re-matching after a reboot.
- **Signals on `global.display`:**
  - `window-created(win)`, `notify::focus-window` / `focus-window(win, ts)`, `restacked`,
    `in-fullscreen-changed`, `workareas-changed`, `window-entered-monitor` /
    `window-left-monitor(monitor, win)`, `grab-op-begin` / `grab-op-end(win, op)` (the user drags
    or resizes with the mouse), `window-demands-attention`, `window-marked-urgent`.
  - Monitors also have `Main.layoutManager` `monitors-changed`.
- **Signals on `Meta.Window`:** `unmanaging` (still usable) and `unmanaged`, `focus`, `raised`,
  `shown`, `position-changed`, `size-changed`, `workspace-changed`, `configure(WindowConfig)`,
  `highest-scale-monitor-changed`, plus `notify::` for `title`, `minimized`, `fullscreen`,
  `maximized-horizontally` / `-vertically`, `above`, `skip-taskbar`, `wm-class` and `mapped`.
- **Timing of `window-created`.** On Wayland the window is created at the first xdg-toplevel
  commit, before it has a buffer, so its size and title may not be final yet.
  - Prior art waits for `windowActor.connect('first-frame', ...)`
    (`tiling-assistant src/extension/tilingWindowManager.js:1070-1080`) or for `shown`.
  - The cleaner GNOME 48+ route is `configure` (§4.3).
- **`manageable()` filter**, mapping vase's `backend::manageable`:
  - `window_type === Meta.WindowType.NORMAL`, `!is_override_redirect()`, `!is_skip_taskbar()`,
    `get_transient_for() === null`, `!is_attached_dialog()`, `allows_move() && allows_resize()`.
  - Fixed-size windows: `get_min_size()` equal to `get_max_size()` (50+).
  - **On Wayland, dialogs are `NORMAL` windows with a transient parent.** The shell itself treats
    `NORMAL` plus `get_transient_for() != null` as a dialog (`shell:ui/windowManager.js:1098-1106`),
    so the transient check is the one that matters.
  - Xwayland windows can report `DIALOG`, `UTILITY`, `SPLASHSCREEN` and so on.
  - Steam uses a `NORMAL` window for its loading splash; Tiling Assistant filters it by app match
    plus `allows_resize()` (same file, 1072-1085).

### 4.2 Moving and resizing

- **The call.** `move_resize_frame(user_op, x, y, w, h)` and `move_frame(user_op, x, y)` work in
  **frame-rect** stage coordinates. The frame rect excludes client-side shadows, so gaps measured
  between frame rects look right (`mutter:src/core/window.c:4447-4577`).
  - Read the current rect back with `get_frame_rect()`.
  - The buffer rect including shadows is `get_buffer_rect()`.
  - Wayland protocol coordinates come from `stage_to_protocol_rect()` / `protocol_to_stage_rect()`.
- **`user_op`** sets `META_MOVE_RESIZE_USER_ACTION`.
  - With `true`: skips `constrain_fully_onscreen` and `constrain_to_single_monitor` and keeps
    "fixed directions" (`mutter:src/core/constraints.c:425-436,1860-1900`).
  - With `false`: the window is pushed fully inside the work area and onto one monitor.
  - Either way, `constrain_partially_onscreen` and the size-hint, size-increment and aspect
    constraints always apply.
- **Prior-art workaround** (`tiling-assistant src/extension/tilingWindowManager.js:265-278`): some
  terminals only resize and don't move with `user_op=true`, so Tiling Assistant calls
  `move_to_monitor(m)`, then `move_frame(true, x, y)`, then `move_resize_frame(true, x, y, w, h)`.
- **Before tiling**, prior art also calls `unmaximize()`, `unmake_fullscreen()`, `unminimize()`
  and `unmake_above()` (same file, 150-185).
  - In 49+ `maximize()` / `unmaximize()` take **no flags**; `get_maximized()` is gone, replaced by
    `is_maximized()`.
  - Partial maximization goes through `set_maximize_flags()` / `set_unmaximize_flags()`
    (gjs.guide 49 page; the typelib confirms).
  - Code that must span 48 and 49+ feature-tests `window.maximize.length` (Tiling Assistant, line 205).
- **Wayland resizes are asynchronous.** Mutter sends a configure and the client acks it with a new
  buffer later. The result can differ from the request (cell-increment terminals, min sizes, apps
  that ignore configures). Listen to `size-changed` / `position-changed` and reconcile, as vase's
  `daemon/reconcile.rs` already does on macOS.
  - `override_constraints(top, left, right, bottom)` with `Meta.WindowConstraint.{NONE, WINDOW, MONITOR}`
    (typelib) tells the client which edges are tiled. That is xdg_toplevel edge constraints, Mutter
    49 !3266. Tiling Assistant uses it (lines 250-262) so CSD apps square their corners and drop
    the shadow on tiled edges.
- **Keeping windows in their tile.** On 50.x: reconcile after `size-changed`. On 51+: an
  `ExternalConstraint` with `info.set_rect()` (headline 2). Implementing a GObject interface from
  GJS is `GObject.registerClass({Implements: [Meta.ExternalConstraint]}, class ... { vfunc_constrain(window, info) {...} })`;
  this is standard GJS but not prototyped.

### 4.3 Placement with no flicker: `Meta.Window::configure`

- **What it is.** `configure(window_config)` is emitted for a Wayland xdg-toplevel **before the
  first configure is sent**: `meta_window_config_initial_new()`, then `meta_window_emit_configure()`,
  then `meta_window_apply_config()` (`mutter:src/wayland/meta-wayland-xdg-shell.c:984-993`).
  `MetaWindowConfig` was added on 2024-11-12 (commit `7e098ae671`) for Mutter 48.
- **What a handler can do** (typelib): call `config.get_is_initial()`, then
  `set_rect()`, `set_position()`, `set_size()` or `set_is_fullscreen()`. The app's first frame
  then appears at its pane's rect. That is vase's "new window lands as a tab or in a pending empty
  pane" without the move-after-map jump.
- **Unverified:** whether X11 (Xwayland) windows emit `configure`, and the exact order relative to
  `window-created`. Connect the handler inside `window-created` and test both in the prototype.

### 4.4 Showing, hiding, focus and stacking (tabs and stack items)

| Need | API | Notes |
|---|---|---|
| Focus | `w.activate(global.get_current_time())` / `w.focus(ts)`; `Main.activateWindow(w)` also switches workspace | The shell is the compositor, so focus-stealing prevention doesn't block it |
| Raise / lower | `w.raise()`, `w.lower()`, `w.raise_and_make_recent_on_workspace(ws)` | Forge's tabbed and stacked layouts switch by `raise()` (`prior/forge/lib/extension/tree.js:855`) |
| Hide without minimizing | `w.inhibit_mapped()` / `w.uninhibit_mapped()` / `w.is_mapped_inhibited()` (**50+**, mutter `6f77df353a`, !4692) | Ref-counted. It makes `showing_on_its_workspace()` false (`mutter:src/core/window.c:1789-1792,8706-8740`). Not yet tested in a desktop session: does the window leave Alt+Tab and the overview, does the client get xdg `suspended`, what happens to focus. **Prototype this first: it may be the GNOME stack primitive** |
| Minimize | `w.minimize()` / `w.unminimize()`; `Main.wm.skipNextEffect(w.get_compositor_private())` skips the animation (`shell:ui/windowManager.js:1069,1114-1117`) | Shows in the dock as minimized; the user sees the state |
| Visual-only hide | `windowActor.hide()` / `.opacity` | The window stays focusable and in window lists; a hack |
| Keep out of lists | `hide_from_window_list()` / `show_in_window_list()` (49+) | For occluded stack items, if Alt+Tab shouldn't list them |
| Close | `w.delete(global.get_current_time())` (`kill()` force-kills) | |
| Fullscreen state | `w.is_fullscreen()`, `notify::fullscreen`, `global.display` `in-fullscreen-changed`, `Main.layoutManager.monitors[i].inFullscreen` | |
| Pointer warp | `global.stage.context.get_backend().get_default_seat().warp_pointer(x, y)` | `Clutter.get_default_backend()` was removed in 51; use the stage context form |
| Launch | `Shell.AppSystem.get_default().lookup_app(id)` then `.open_new_window(-1)` or `.launch(ts, ws, gpu)` | Replaces vase's `launch` and `launchable_apps` (`get_installed()`) |

Off-screen parking is **not** in this table: mutter clamps it (headline 1).

## 5. Workspaces and monitors

- **Workspaces.** `global.workspace_manager` (`Meta.WorkspaceManager`):
  - methods `get_active_workspace()`, `get_n_workspaces()`, `get_workspace_by_index()`,
    `append_new_workspace()`, `reorder_workspace()`;
  - signals `active-workspace-changed`, `workspace-added`, `workspace-removed`,
    `workspace-switched(from, to, dir)`, `workspaces-reordered`.
- **`Meta.Workspace`:** `list_windows()`, `get_work_area_for_monitor(i)`, `activate(ts)`,
  `activate_with_focus(w, ts)`, and signals `window-added` / `window-removed`.
  - GNOME uses dynamic workspaces by default (`org.gnome.mutter dynamic-workspaces`), and
    `org.gnome.mutter workspaces-only-on-primary` changes which windows move on a switch.
  - In vase's model a GNOME workspace is an OS **Workspace** that vase doesn't manage (windows
    elsewhere are marked `◇`). That maps cleanly to `w.get_workspace()`, `located_on_workspace()`,
    `is_on_all_workspaces()` and `workspace-changed`.
  - A design option no ADR covers yet: back each vase **Tab** with a hidden GNOME workspace, so
    tab switches are native. It conflicts with vase's "no separate workspace level" and with
    GNOME's own overview, so it is only worth noting.
- **Monitors:**
  - `Main.layoutManager.monitors` (each has `index, x, y, width, height, geometryScale, inFullscreen`),
    `.primaryIndex`, `.currentMonitor`, `getWorkAreaForMonitor(i)` (`shell:ui/layout.js:662,1005`),
    signal `monitors-changed` (`:645`);
  - `global.display.get_n_monitors()`, `get_monitor_geometry()`, `get_monitor_scale()`,
    `get_current_monitor()`, `get_monitor_neighbor_index(i, Meta.DisplayDirection.*)`, signal
    `workareas-changed`; `w.get_monitor()`, `w.move_to_monitor(i)`.
  - Stage coordinates are logical pixels; with fractional scaling, `geometryScale` gives the
    physical scale.
  - The work area excludes the top bar and the Ubuntu Dock's struts; it is vase's `Display` frame.

## 6. The PREFIX mode (a tmux-like modal after Alt+A)

**Why a grab is needed.** Mutter's event filter runs keybindings, and then:
- if the stage has **no grab**, a key event goes to the focused Wayland surface
  (`meta_wayland_compositor_handle_event`), and actors in the shell see it only when the stage
  itself has key focus;
- if the stage **has a grab**, `get_window_for_event` returns NULL and the event goes to the
  grab's Clutter actor (`mutter:src/core/events.c:62-85,249-300`).

So, after Alt+A, "swallow the next key, whatever it is" (tmux semantics) needs a stage grab.
Re-registering every command key as a temporary accelerator would let unknown keys through to the
app.

**The pattern** (SwitcherPopup and PaperWM's navigator both use it):

```js
// in the Alt+A handler
const catcher = new Clutter.Actor({reactive: true});   // or the visible "PREFIX" indicator widget
Main.layoutManager.addTopChrome(catcher);
this._grab = Main.pushModal(catcher, {actionMode: Shell.ActionMode.NONE});
// pushModal = global.stage.grab(actor) + set_key_focus(actor) + Main.actionMode swap
//   (shell:ui/main.js:773-815); popModal(grab) restores focus and actionMode (:824-850)
catcher.connect('key-press-event', (a, event) => {
    const sym = event.get_key_symbol(), code = event.get_key_code(), mods = event.get_state();
    // vase KeyCode is "a US-layout character": map hardware keycode (code) to US positions,
    // or use keysym if layout-following is wanted. Handle Escape = cancel.
    // event.get_flags() & Clutter.EventFlags.FLAG_REPEATED -> repeated resize keys
    ...; return Clutter.EVENT_STOP;
});
// exit: Main.popModal(this._grab); catcher.destroy();
// also exit on this._grab 'notify::revoked' (a system-modal dialog or the lock screen took over),
// and on a safety timeout (GLib.timeout_add_once is 50+, gjs.guide 50 page)
```

**Details that matter:**
- **`actionMode: NONE` turns off all keybindings while the prefix is armed**
  (`_filterKeybinding`), so Alt+Tab, Super and similar don't fire mid-chord. Pressing Alt+A
  inside the modal arrives as a plain key event.
- **Hold the grab across key repeat** for vase's repeatable resize keys
  (`vase-core/src/input/keymap.rs:52-66`).
- **Modifier release.** The Alt release and the `a` release that follow Alt+A go to the grab
  actor. The client saw the Alt press but never the `a`, and when the grab ends it gets
  `wl_keyboard.enter` with the real pressed-key and modifier state, so nothing sticks.
  SwitcherPopup notes a race where Alt is released before the grab takes effect; it checks
  `global.get_pointer()[2]` modifiers right after grabbing (`shell:ui/switcherPopup.js:144-156`).
  That matters only if a mode depends on holding Alt.
- **Focus side-effect.** While the prefix is armed the app has no Wayland keyboard focus
  (headline 3); `display.focus_window` is unchanged, so `appears_focused` stays true. Keep the
  armed window short and visible: a small "PREFIX" chrome indicator, like tmux's status flag.
- **The Alt+E stack prefix** is a second binding that arms the same modal with the nested keymap
  (`bindings_nested()` in `vase-core/src/input/keymap.rs:97-110`).
- **`Clutter.KeyController`** (typelib 50.1: signals `key-press`, `key-release`,
  `modifier-change`; `set_trigger_keybindings()`) is the replacement GNOME 51 documents. In 51,
  `key-press-event` still works but is deprecated (gjs.guide 51 page). For 50 and 51 together,
  use `key-press-event` now and switch later.
- **Lighter alternative to `pushModal`:** `global.stage.grab(actor)` alone returns a
  `Clutter.Grab` with `dismiss()`, `is_revoked()` and a `revoked` property. It skips the
  actionMode and focus-stack bookkeeping, so shell keybindings would still fire. `pushModal` is
  the better fit.

## 7. Chrome: tab bar, stack bars, focus border, pickers

- **Where actors go:**
  - `Main.layoutManager.addChrome(actor, {affectsStruts, trackFullscreen})`: in `uiGroup` above
    all windows but **below** `global.top_window_group` (popup menus); it tracks the input region.
  - `addTopChrome(actor, params)`: above everything, including popups.
  - `trackChrome()` / `removeChrome()`. (`shell:ui/layout.js:862-945`; defaults
    `{trackFullscreen:false, affectsStruts:false}` at `:186-189`.)
- **The bottom tab bar** (vase: "the tab bar is the user's bottom bar"): `addChrome(bar, {affectsStruts: true, trackFullscreen: true})`
  along the bottom edge. Mutter then shrinks the work area so maximized and tiled windows don't
  cover it, and the bar hides over fullscreen windows (`layout.js:987-993`).
  - Watch for the Ubuntu Dock at the bottom: two struts on the same edge.
- **Stack bars and the focus border** sit on top of specific windows:
  - As chrome they float above **all** windows, including an unrelated window raised over the
    stack.
  - Forge puts its tab decorations into `global.window_group` and restacks them next to window
    actors (`prior/forge/lib/extension/tree.js:524,638-709`;
    `set_child_above_sibling(bar, windowActor)`), so they interleave with windows. The cost is
    re-stacking on `display::restacked`.
- **Widgets.** St widgets (`St.BoxLayout`, `St.Label`, `St.Button`, `St.Icon`) are styled by the
  extension's `stylesheet.css`, which is loaded automatically and reloaded on color-scheme change
  (`shell:ui/extensionSystem.js:45-46`).
  - St in 51 removes the deprecated `vertical` property (use `orientation`) and renames
    `St.ButtonMask` values. Avoid both now.
  - The native style comes from `St.Settings.get()` (`color-scheme`, accent color, and
    `reducedMotion` from 51 on); the powerline style can be drawn with `St.DrawingArea` and Cairo.
  - App icons: `Shell.WindowTracker.get_window_app(w).create_icon_texture(size)`.
- **Pickers and the shortcut sheet** are shell popups, modeled on `SwitcherPopup`
  (`Main.uiGroup.add_child` plus `pushModal`; Tiling Assistant vendors a copy at
  `src/dependencies/unexported/switcherPopup.js:54`). Use `Main.layoutManager.modalDialogGroup`
  or `ModalDialog` for dialogs.
- **Scaling.** Chrome lives in logical pixels; use `St.ThemeContext.get_for_stage(global.stage).scale_factor`
  for crisp drawing.

## 8. Animations, gaps, fullscreen, dialogs

- **Animations.**
  - `actor.ease({x, y, width, height, opacity, duration, mode: Clutter.AnimationMode.EASE_OUT_QUAD, onComplete})`
    is gnome-shell's helper on every actor; **50 adds `easeAsync()`**, which resolves on completion
    and rejects with CANCELLED when interrupted (gjs.guide 50 page).
  - Respect `St.Settings.get().enable_animations`, plus `reducedMotion` in 51.
  - Animating a *window* means animating its `Meta.WindowActor` (translation and scale) and
    committing one `move_resize_frame()` at the end; the client never sees the in-between sizes.
    Tiling Assistant and Pop Shell both do this.
  - Tab switches in a keyboard tiler should be instant; keep animation to chrome.
- **Gaps.** No mutter API is needed: subtract them in the layout math from `get_work_area_for_monitor()`
  and apply them to frame rects.
  - Tiling Assistant's "maximize with gap" shows the catch: a gapped "maximized" window is not
    `is_maximized()`, so it keeps rounded corners and shadows unless `override_constraints()`
    marks its edges tiled.
- **Fullscreen.** Leave fullscreen windows out of the layout: skip `is_fullscreen()` and re-tile
  on `notify::fullscreen`. That covers the spec's "ignoring fullscreen windows" Tiler feature.
  - `trackFullscreen` hides the chrome.
  - `global.display.get_monitor_in_fullscreen(i)` per monitor.
  - Games: fullscreen Xwayland windows are often `is_monitor_sized()` / `is_screen_sized()`
    without the fullscreen state; check both.
- **Dialogs and transients.**
  - Float anything with `get_transient_for()`, `is_attached_dialog()` (attached modal dialogs move
    with their parent), a non-`NORMAL` type, or `!allows_resize()`.
  - `w.foreach_transient(fn)` and `find_root_ancestor()` follow the chain.
  - Re-raise transients after raising a stack item: `raise()` already raises a window's
    transients with it.

## 9. The kill chord, Pause and safety

- **No kill chord.** vase's macOS and Windows daemons can be killed. The extension *is* the
  compositor process, so killing gnome-shell on Wayland ends the session.
- **Things the extension can do itself:**
  - a `NON_MASKABLE` binding that `disable()`s the Tiler's own behaviour: drop the grabs and
    chrome, stop managing, leave windows where they are;
  - a prefix-mode timeout plus `notify::revoked` handling, so a stuck modal can't trap the keyboard.
- **The system escape hatches:**
  - `switch-to-session-N` (Ctrl+Alt+F*) is allowed in `ActionMode.ALL`
    (`shell:ui/windowManager.js:541-552`). It is filtered while a modal sets `NONE`, so the prefix
    timeout matters.
  - From a VT: `gnome-extensions disable <uuid>`. That is audit condition 5 ("Pause disables the
    extension").
- **Pause for gaming.**
  - Disabling and re-enabling the extension loses the in-memory layout unless it is persisted
    first. vase already persists its layout to JSON (CONTEXT.md, "Layout persistence");
    `Gio.File` writes from `disable()` would preserve it.
  - Having an extension disable *itself* through `Main.extensionManager` counts as "interacting
    with the extension system", which the review guidelines discourage. Chained isn't publishing to
    extensions.gnome.org, but its audit applies those rules (condition 3).
  - An in-extension "paused" state may be the cleaner design, with audit condition 5 reworded to
    match.

## 10. Install and enable on the owner's desktop: the policy conflict

Current values on the desktop (read-only `gsettings get`):
- `allow-extension-installation=false` (Chained)
- `disable-user-extensions=true`
- `enabled-extensions` lists three user extensions in `~/.local/share` plus the Ubuntu ones.

What the shell does with them:
- With installation off, `collectFromDatadirs('extensions', includeUserDir=false)` scans only the
  `XDG_DATA_DIRS` entries (`shell:misc/fileUtils.js:19-41`; upstream `extensionSystem.js:711`).
  For gnome-shell (read from `/proc/<pid>/environ`) those are
  `~/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:/usr/local/share/:/usr/share/:/var/lib/snapd/desktop`.
- With `disable-user-extensions=true`, only the session mode's `enabledExtensions` run
  (`/usr/share/gnome-shell/modes/ubuntu.json`: ubuntu-dock, appindicators, ding, tiling-assistant,
  snapd-prompting, snapd-search-provider, web-search-provider). That holds even for extensions found
  in system directories (`shell:ui/extensionSystem.js:550-560`).
- Ubuntu's own patch also refuses a *per-user* copy of a mode extension
  (`shell:ui/extensionSystem.js:733-736`; not in upstream).

Options for the Tiler track (a decision, not a recommendation):
- **(a) System install.** Put it in `/usr/local/share/gnome-shell/extensions/<uuid>` with its schema
  in `/usr/local/share/glib-2.0/schemas` (needs root; Chained has a sudo step), then set
  `disable-user-extensions=false` and add the uuid to `enabled-extensions`. Chained's lint and
  docs/tools/gnome.md currently refuse those keys.
- **(b) Per-user install by chezmoi** (audit condition 1 as written). Needs
  `allow-extension-installation=true`, which brings back the extensions.gnome.org update check for
  per-user UUIDs. Audit condition 2 handles that with a UUID under the owner's own domain, but the
  audit table rejects the setting itself.
- **Note for the audit.** `~/.local/share/flatpak/exports/share` is user-writable and comes first in
  gnome-shell's `XDG_DATA_DIRS`, so a "system"-type extension can be planted there without root.
  It would still have to be in `enabled-extensions` with `disable-user-extensions=false` to run.
  Today's `disable-user-extensions=true` is what blocks it, not `allow-extension-installation`.

## 11. Mapping vase's `Backend` trait to GNOME (vase-core/src/backend.rs:7-80)

| `Backend` method | GNOME 50 equivalent |
|---|---|
| `displays()` | `Main.layoutManager.monitors` plus `getWorkAreaForMonitor(i)` |
| `list_windows()` / `all_windows()` | active-workspace `list_windows()` / `global.display.list_all_windows()`, filtered |
| `set_frame(id, rect)` | `move_frame(true, …)` then `move_resize_frame(true, …)`, reconcile on `size-changed` |
| `focus(id)` / `raise(id)` | `activate(ts)` / `raise()` |
| `forget(id)` | drop signal handlers; `uninhibit_mapped()` if inhibited |
| `title()`, `minimized()`, `fullscreen()` | `get_title()`, `minimized` property, `is_fullscreen()` |
| `tileable(info)` | the §4.1 filter plus `get_min_size()` / `get_max_size()` |
| `set_minimized()` | `minimize()` / `unminimize()` (plus `skipNextEffect`) |
| `close(id)` | `delete(ts)` |
| `launchable_apps()` / `launch(app)` / `running_apps()` / `activate(app)` | `Shell.AppSystem.get_installed()` / `lookup_app(id).open_new_window(-1)` / `get_running()` / `app.activate()` |
| `warp_cursor()` | `Clutter.Seat.warp_pointer()` |
| `badged_apps()` | no direct API (the dock's badges come from the Unity LauncherEntry D-Bus); leave empty |
| event hook (keys) | §3 bindings plus the §6 modal |

**Where the code would live** (an architecture note outside this lens, for the wayfinder):
- **GJS supports WebAssembly.** `gjs 1.88` here: `typeof WebAssembly === 'object'`; a minimal
  module compiled. vase-core's dependencies are pure Rust (serde, serde_json, toml, toml_edit), but
  it uses `std::fs`, `std::time` and `std::thread` in `daemon/*`, `state.rs` and `config.rs`.
- So the options are:
  - (i) port the core to JS;
  - (ii) compile the platform-agnostic model to wasm and call it from the extension;
  - (iii) run the Rust daemon outside and drive the extension over D-Bus as the actuator.
- Option (iii) breaks the "single daemon owns its keybindings" model.
- Options (ii) and (iii) both hit audit condition 3: no subprocesses, and extensions.gnome.org's
  "no binaries" rule.
- Not verified: wasm inside gnome-shell's own JS context, as opposed to standalone gjs.

## 12. GNOME 50 versus 45–49 (what changed for this extension)

- **45:** ESM extensions; `Extension` / `ExtensionPreferences` classes; `getSettings()` on the
  class; `imports.*` gone for shell modules.
- **46–47:** St and Clutter cleanups. Not tiler-critical.
- **48:** `Meta.WindowConfig` and the `configure` signal (Mutter commit `7e098ae671`, Nov 2024);
  initial Wayland window configuration revamped (mutter NEWS !4582).
- **49:**
  - `Meta.Rectangle` replaced by `Mtk.Rectangle`;
  - `get_maximized()` replaced by `is_maximized()`; `maximize()` / `unmaximize()` lost their flags;
    `set_maximize_flags()` / `set_unmaximize_flags()`, `set_type()`, `hide_from_window_list()` /
    `show_in_window_list()` added;
  - `Clutter.ClickAction` / `TapAction` replaced by `ClickGesture` / `LongPressGesture`;
  - xdg_toplevel edge constraints (!3266);
  - X11 disabled by default; nested sessions through `--devkit` (gjs.guide 49 page; mutter NEWS).
- **50:**
  - X11 backend dropped (!4505); `restart` / `show-restart-message` no longer emitted;
  - `easeAsync()`; `GLib.timeout_add_once()` / `idle_add_once()`;
  - `holdKeyboard()` / `releaseKeyboard()` removed from `misc/keyboardManager.js`;
  - Mutter: `get_min_size()` / `get_max_size()` (!4917); `inhibit_mapped()` (!4692); external
    constraints (!4692, read-only from JS in 50.x); "Avoid processing the same key event twice
    during keybindings" (!4724);
  - `begin_grab_op()` now takes a `Clutter.Sprite`;
  - `gnome-shell-test-tool --extension`; `gnome-extensions install --print-uuid`.
- **51** (released 2026-09, not on Ubuntu 26.04):
  - `ExternalConstraintInfo.set_rect()` makes external constraints usable;
  - `Clutter.KeyController` and the other event controllers are the documented input path, and
    `key-press-event` is deprecated;
  - `Clutter.get_default_backend()` removed; `ui/pointerWatcher.js` removed;
  - async `disable()` throws; St `vertical` removed.

## Sources

- gjs.guide porting guides: [GNOME 49](https://gjs.guide/extensions/upgrading/gnome-shell-49.html),
  [GNOME 50](https://gjs.guide/extensions/upgrading/gnome-shell-50.html),
  [GNOME 51](https://gjs.guide/extensions/upgrading/gnome-shell-51.html)
- gjs.guide [review guidelines](https://gjs.guide/extensions/review-guidelines/review-guidelines.html) and
  [preferences and GSettings](https://gjs.guide/extensions/development/preferences.html)
- Mutter 50.1 source (`https://gitlab.gnome.org/GNOME/mutter/-/raw/50.1/<path>`):
  - `src/core/events.c`, `src/core/keybindings.c`, `src/core/constraints.c`, `src/core/window.c`,
    `src/core/display.c`;
  - `src/wayland/meta-wayland-input.c`, `src/wayland/meta-wayland-xdg-shell.c`;
  - `src/meta/meta-external-constraint.h` (absent at 49.0 and 49.4, present at 50.0 and 50.1);
  - NEWS.
- Mutter commits through the gitlab API: `17b6a69868` and `0eaf17634e` (external constraints,
  [!4692](https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/4692)), `6f77df353a` (map
  inhibit), `881a00d25f` (min/max size), `f40380f6cf` (`set_rect`, main and 51 only), `7e098ae671`
  (WindowConfig).
- Mutter docs: [Meta.Window](https://mutter.gnome.org/meta/class.Window.html),
  [ExternalConstraintInfo](https://mutter.gnome.org/meta/struct.ExternalConstraintInfo.html)
  (these docs track main, API 52).
- gnome-shell 50.1:
  - the installed `libshell-18.so` gresource (Ubuntu build), and upstream
    `https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/50.1/js/ui/extensionSystem.js` for the diff;
  - `ui/main.js`, `ui/windowManager.js`, `ui/layout.js`, `ui/switcherPopup.js`, `ui/shellDBus.js`,
    `ui/extensionSystem.js`, `misc/fileUtils.js`, `extensions/extension.js`,
    `extensions/sharedInternals.js`.
- Typelibs: `Meta-18`, `Clutter-18`, `Shell-18`, `St-18` (dumped with `scratch/dump.js`).
- Prior art:
  - Ubuntu Tiling Assistant v54 (`/usr/share/gnome-shell/extensions/tiling-assistant@ubuntu.com`);
  - [PaperWM](https://github.com/paperwm/PaperWM) (`navigator.js`, `keybindings.js`);
  - [Forge](https://github.com/forge-ext/forge) (`lib/extension/tree.js`, `keybindings.js`;
    tabbed and stacked layouts with tab decorations in `window_group`);
  - [Pop Shell](https://github.com/pop-os/shell);
  - [Tiling Shell](https://github.com/domferr/tilingshell).
  - All five are active, and all declare shell-version 50 (PaperWM and Forge declare 51 too).
- Local:
  - `/usr/share/gnome-shell/modes/ubuntu.json`;
  - `/usr/lib/systemd/user/org.gnome.Shell-disable-extensions.service`;
  - Chained `docs/research/security-audit.md` section 24 and `.scratch/chained-v1/spec.md:638-656,1039`;
  - vase-chained `CONTEXT.md`, `docs/adr/*`, `vase-core/src/backend.rs`,
    `vase-core/src/input/keymap.rs`.
