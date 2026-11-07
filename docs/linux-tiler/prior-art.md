# Prior art for the vase Linux Tiler (GNOME Shell extension)

Lens: what existing GNOME tilers do, and what the `vase-chained` Linux backend should copy or avoid.
Researched 2026-11-07, read-only. Repos were shallow-cloned into
`/home/martin/.claude/jobs/91ccbb14/tmp/tiler/scratch/repos/` at the commits below. File:line references are to those
commits. Mutter and gnome-shell headers were fetched from gitlab.gnome.org at tags 48.0, 49.0 and 50.1, into
`scratch/gnome/`.

| Project | Repo @ commit | Licence | Shell versions at HEAD | Activity |
|---|---|---|---|---|
| Forge | forge-ext/forge @ a458e308b5 | GPL-3.0 | 45–51 | 1.3k stars, 212 open issues; "Needs a new maintainer" (metadata.json; issue #336 since 2024-01) |
| PaperWM | paperwm/PaperWM @ 2d460db836 | GPL-3.0 | 45–51 | 4.4k stars, 205 open; 3,583 commits since 2017; very active |
| Pop Shell | pop-os/shell @ 7898b65c20 | GPL-3.0 | 45–50 | 5.3k stars, 453 open; commits by year: 620 (2020) → 19 (2023) → 6 (2025) → 1 (2026) |
| Tiling Shell | domferr/tilingshell @ 5c3c052cf0 | GPL-3.0 | 42–51 | 2.1k stars, 224 open; essentially one author |
| gTile | gTile/gTile @ 04f161a202 | LGPL-2.0-or-later (LICENSE text; GitHub says NOASSERTION) | 49–51 | 13 open; ported every cycle |
| Tiling Assistant | ubuntu/Tiling-Assistant @ 62b43228a8 (Leleat/… redirects) | GPL-2.0-or-later | 48–51 | now maintained by Canonical (Marco Trevisan) |
| Ubuntu's packaged copy on this desktop | `tiling-assistant@ubuntu.com` v54, package `gnome-shell-ubuntu-extensions`, gnome-shell 50.1-0ubuntu1.5 | GPL-2.0+ | 48–50 | read from `/usr/share/gnome-shell/extensions/tiling-assistant@ubuntu.com/metadata.json` |
| universal-tiling-window-manager (utwm) | johncarmack1984/universal-tiling-window-manager @ 58fdeecd77 | MIT | 50 only | 0 stars, new (Linux merged 2026-10-26, PR #8) |
| Window Calls | ickyicky/window-calls | **no licence file** (all rights reserved) | 45–50 | D-Bus window ops for scripts |

All the GPL/LGPL/MIT ones can legally be copied into the GPL-3.0 fork, keeping their notices. Window Calls cannot be
copied, because it has no licence. Read it only.

---

## 1. Architecture, project by project

### Forge (i3/sway-style tree, in-shell)
- Everything runs in the shell process: a tree of nodes (`lib/extension/tree.js`) and a window manager
  (`lib/extension/window.js`, ~2.7k lines). Layout types include `STACKED` and `TABBED` (tree.js:42-43).
- **Tabbed:** every child gets the same rectangle, minus a tab strip (`processTabbed`, tree.js:1534-1577). The strip is
  an `St.BoxLayout` added to `global.window_group` (tree.js:519-532), with one St tab per window (tree.js:455-516).
  The selected window is shown by **raising** it (`metaWindow.raise()`, tree.js:855). Nothing is hidden; the others are
  just covered.
- **Stacked:** i3-style cascade, where each child is offset by `stackHeight` (35 px × dpi) per index
  (`processStacked`, tree.js:1506-1517).
- **Moving windows:** `move()` (window.js:961-982) unmaximizes first, then calls `remove_all_transitions()` on the
  actor, then `move_frame` **then** `move_resize_frame`. The extra move call is because some apps ignore a move that
  comes with a resize.
- **Keys:** plain global chords. `Main.wm.addKeybinding(..., Meta.KeyBindingFlags.NONE, Shell.ActionMode.NORMAL)`
  (keybindings.js:72-76). It has a `grab_accelerator` helper marked "@deprecated" (keybindings.js:107-145). There is no
  modal prefix.
- **Rendering cadence:** events go through a queue drained by a 220 ms `GLib.timeout_add` (window.js:164-178), and
  renders are coalesced in `GLib.idle_add` (window.js:1172, 1209). Timing hacks like these are where its "stuck preview"
  and "windows open on the wrong display" bugs live (#175, #299).
- **Lock screen:** declares `session-modes: ["user","unlock-dialog"]`, so the tree survives a lock. Keybindings are
  disabled while locked (extension.js:74-90). The comment there quotes the review-guideline reason: the extension would
  otherwise lose its in-memory tree on every lock.

### PaperWM (scrollable strip, in-shell, heavy shell patching)
- About 20k lines in-shell. `tiling.js` alone is 5.7k lines.
- **Clones:** every window is shown through a `Clutter.Clone` in its own container, because "WindowActors can only live
  in the `global.window_group` and can't be moved reliably outside the monitor" (tiling.js:63-67). This bears on vase's
  off-screen-parking ADR: Mutter's constraints fight off-screen placement.
- **Chrome z-order:** "mutter keeps syncing the window_group with the window stacking and in the process destroy the
  stacking of any non-window actors". The fix is to restack the chrome on the display's `restacked` signal
  (stackoverlay.js:20-45). Pop Shell does the same thing (stack.ts:151).
- **Keyboard mode:** the "navigator" `ActionDispatcher` (navigator.js:69-200) is modelled on GNOME's
  `SwitcherPopup`. A keybinding fires, then `Main.pushModal(actor)` (navigator.js:85) routes all keys to the actor's
  `key-press-event` / `key-release-event` until the held modifier is released. Two lessons sit in its comments:
  - navigator.js:136-145: the user may release the modifier before the grab lands. Check `global.get_pointer()` mods
    right after the grab.
  - navigator.js:180-185: "Popping the modal on keypress doesn't work properly, as the release event will leak to the
    active window". Pop the modal on the **release** (issue #70).
- **Patching:** `patches.js` (695 lines) overrides shell internals: workspace animation, overview layout, alt-tab,
  screenshot UI, thumbnails (patches.js:148-445). This is the main reason it needs work every cycle (see §3).

### Pop Shell (auto-tiler with stacks, TypeScript → GJS; logic in-shell, launcher in Rust)
- An ECS/arena model (`ecs.ts`, `arena.ts`, `forest.ts`, `fork.ts`), compiled from TypeScript.
- **Stacks (tabs):** `Stack` class (stack.ts:123-200). An St tab bar is added to `global.window_group` and restacked on
  `restacked` (stack.ts:151). The **non-active windows' compositor actors are hidden** with
  `get_compositor_private().hide()` (stack.ts:243, 534, 597). This is a hide that only exists inside the shell. It is
  better than occlusion for translucent or rounded windows, but it must be undone on every exit path.
- **Keyboard mode ("adjustment mode"):** done by **swapping keybinding sets**, not with a modal grab. `enter()`
  disables the normal focus bindings and enables the tile-mode set (arrows, Enter, Esc…): tiling.ts:744-766. `exit()`
  reverses that (tiling.ts:810-822). While in the mode, keys outside the set still reach the focused app, so the mode
  leaks.
- **Rust side:** the launcher's logic is the Rust `pop-launcher`, spawned with `Gio.SubprocessLauncher`
  (utils.ts:125-138). It speaks newline-delimited JSON over stdin/stdout (launcher_service.ts:8-39, 96-98), and the UI
  is an St modal dialog (`Main.pushModal`, search.ts:280). This is the oldest working "St UI in the shell, logic in an
  external Rust process" split.
- Also exports a small D-Bus API, `com.System76.PopShell` (FocusLeft…, WindowList, WindowFocus) at
  dbus_service.ts:3-55.
- **Trajectory:** System76 moved its effort to COSMIC, its own Rust compositor (cosmic-comp). The commit counts above
  show it. Pop Shell now gets about one port commit per GNOME release.

### Tiling Shell (FancyZones/snap layouts, in-shell, TypeScript)
- Layout-zone tiling with mouse and keyboard, plus "window suggestions". No stacks or tabs. Keys are bound with
  `Main.wm.addKeybinding` (keybindings.ts:103-236).
- **System settings:** it **overrides GNOME settings at runtime** (native `edge-tiling` off, extension.ts:143-150;
  mutter/wm keybindings such as `toggle-tiled-left` and `maximize`, keybindings.ts:305-340, 404-419). The old values are
  saved in its own gsettings (`OVERRIDDEN_SETTINGS`), so it can restore them after a crash
  (settings/settingsOverride.ts:4-34).
- D-Bus: one method, `openLayoutEditor` (dbus.ts).

### gTile (grid overlay)
- A grid overlay that you drive with the keyboard or the mouse.
- **Keyboard mode:** keybinding *groups* by bitmask (`KeyBindingGroup` Global/Overlay/Autotile/Action/Preset,
  core/HotkeyManager.ts:32-44). Opening the overlay registers the Overlay group with `addKeybinding`. Closing it
  unregisters the group (HotkeyManager.ts:278-300). This is the same set-swap idea as Pop Shell.
- The codebase is small, typed and well kept (TypeScript, tests in `test/`). It is ported promptly every cycle with a
  version bump.

### Tiling Assistant (Ubuntu's current tiler)
- Snap assist and tile groups, plus a **"Tile Editing Mode"** (src/extension/tileEditingMode.js). It is a true modal:
  an `St.Widget` added to `Main.uiGroup`, with `Main.pushModal(this)` (tileEditingMode.js:44-61) and sub-modes
  DEFAULT/SWAP/RESIZE/MOVE/CLOSE (tileEditingMode.js:11-17). On Shell < 50 it checks
  `grab.get_seat_state() & Clutter.GrabState.KEYBOARD` and bails out if it did not get the keyboard
  (tileEditingMode.js:53-59). See §3 for why that check is gone on 50.
- Keybindings use `Main.wm.addKeybinding` in NORMAL (plus OVERVIEW for some keys), keybindingHandler.js:18-22.
- It ships **copies of non-exported shell modules** (`src/dependencies/unexported/altTab.js`, `altTab-48.js`,
  `switcherPopup*.js`, `windowManager.js`). Each GNOME release needs those copies refreshed.
- Its move order is the same as Forge's (move, then move+resize). utwm's PR credits Tiling Assistant for it.

### utwm (closest analogue: a cross-platform Rust tiler with a thin GNOME bridge)
A tiler with Windows and macOS backends added Linux in Oct 2026, exactly the way ADR-0001 expects vase to.
- **The extension only reports facts and carries out requests.** "Every decision … is the app's"
  (gnome-extension/extension.js:1-13). It is 516 lines, `shell-version: ["50"]`, with
  `session-modes: ["user","unlock-dialog"]` (metadata.json:6).
- **D-Bus interface** (extension.js:31-80): it owns its own bus name (`own_name`, :204). It has the properties
  `ApiVersion` and `Revision`, the methods `Screens`, `Focused`, `Windows` (each window as `(t, a{sv})` facts),
  `Describe`, `Place(id, frame, raise) → achieved frame`, `MoveTo`, `Activate` and
  `SetBindings(generation, accelerators) → grabbed[]`, and the signals `Activated(generation, index)` and
  `GrabsChanged`.
- **Facts, not titles** (extension.js:148-170): frame, pid, window type, attached-dialog, override-redirect,
  min/max/fullscreen, allows-move/resize, skip-taskbar, on-active-workspace, app-id, sandboxed-app-id, gtk-app-id and
  wm-class.
- **Place awaits the result.** `PlaceAsync` (:274-292) unmaximizes, calls `move_frame` and then `move_resize_frame`,
  then `_settle()` (:368-420). Settling waits on `size-changed`/`position-changed` until the frame matches or holds
  still: 400 ms quiet before the first change, 150 ms after, a 1 s cap (:82-87). Then it returns the frame actually
  achieved. The comment gives the reason: "A Wayland app takes its new size when it next draws".
- **Key grabs** are done the way gnome-shell grabs them for gnome-settings-daemon (`_grabAcceleratorForSender`,
  js/ui/shellDBus.js:321 at 50.1):
  - `global.display.grab_accelerator(accel, IGNORE_AUTOREPEAT)` + `Main.wm.allowKeybinding(name, NORMAL)` (:425-433).
  - Grabs are by keycode, so they don't depend on the layout. Only Ctrl/Alt/Super chords or F-keys are accepted, at
    most 200 (:99-116), so the bridge can't be turned into a keylogger.
  - One client at a time. Grabs are released when that client's unique name leaves the bus (`bus_watch_name`,
    :314-350).
  - Refused grabs are retried when keybinding gsettings or the keymap change (:186-200, `_retry`).
  - Activations go out as signals addressed to that client only (`emit_signal(this._client …)`).
- **Lock screen:** in `unlock-dialog` the bridge stays alive (its bus name is not dropped), but every window method
  throws ACCESS_DENIED while `Main.sessionMode.isLocked` (:354-357).
- **Rust side** (`crates/utwm-linux/src/shell.rs`): zbus blocking, a 2 s method timeout. Before trusting the bridge's
  name it checks that the name's owner equals the owner of `org.gnome.Shell`, "since any process may claim a name on
  the session bus" (shell.rs:23-25, 117-120, 149). The interface is versioned (`API_VERSION`, shell.rs:17), and a test
  checks that the extension and the app speak the same version (extension.rs:332).
- **Install:** the app lays the extension files into `~/.local/share/gnome-shell/extensions/`, adds the UUID to
  `enabled-extensions`, and tells the user to "log out and back in once" (install.rs:59-62, extension.rs:279-300).
- **What it lacks for vase:** no modal mode (it grabs chords only), no chrome, no stacks, no hiding.

### Window Calls / Window Control / ws-dbus (thin D-Bus window-ops extensions)
- These extensions just export window list/move/resize/close over D-Bus for scripts (extensions.gnome.org 4724,
  github.com/carlo9890/gnome-window-control, github.com/kemallette/ws-dbus). They exist because on Wayland
  `Shell.Eval` is off (only in unsafe mode; shellDBus.js:70-71 at 50.1) and `Shell.Introspect` is read-only.
- Window Control's README states the obvious security cost: **any session process can enumerate titles and move or
  close windows**. None of them checks who is calling. This is the anti-pattern for vase.

---

## 2. Keyboard modes: the three patterns in the wild

| Pattern | Who | How | Fit for vase's `Alt+A` / `Alt+E` tmux prefix |
|---|---|---|---|
| Direct chords | Forge, Tiling Shell, utwm | `Main.wm.addKeybinding` (gsettings-backed) or `global.display.grab_accelerator` + `allowKeybinding` | Only good for the prefix chords themselves (and the kill chord) |
| Swap keybinding sets | Pop Shell adjustment mode, gTile overlay | Register a second set of accelerators while the mode is on | Leaky: unbound keys reach the focused app; plain keys (`\`, `-`, `c`, digits) would become global grabs; adding and removing grabs on every prefix press is slow and racy |
| Modal grab | PaperWM navigator, Tiling Assistant Tile Editing Mode, Pop Shell launcher | `Main.pushModal(stActor)` → `global.stage.grab(actor)` (main.js:697-737 at 50.1); handle `key-press-event` / `key-release-event`; `popModal` | **The right model for a prefix.** After the prefix, every key goes to vase and none leaks. It is also what text entry needs (rename `t`, command line `:`, the pickers) |

Recommended shape, built from the pieces above:
- Grab `Alt+A` and `Alt+E` (and the kill chord) with `grab_accelerator` + `allowKeybinding`, keycode-based with
  IGNORE_AUTOREPEAT, as utwm does. `Alt+E` is `stack_prefix()` in `vase-core/src/input/keymap.rs:13`.
- On activation, `pushModal` an invisible St actor. Then forward each key-press to the daemon as a vase-neutral
  `KeyCode` + mods. vase-core already has `KeyCode` translated per backend (vase CONTEXT.md, "Keys").
- **Pop the modal on the key release** of the command key, not on the press (PaperWM #70).
- The shell must never block on the daemon. Send keys asynchronously and let the daemon reply with what to do: stay
  modal (repeatable resize, switcher, prompt) or leave.
- Always leave the modal on Escape, on a timeout, when the daemon's bus name vanishes, and in `disable()`. A modal grab
  that sticks takes the whole desktop's keyboard and pointer.
- Gate everything on `!Main.sessionMode.isLocked`.

---

## 3. GNOME breakage history (what each cycle broke)

| Cycle | What broke | Evidence |
|---|---|---|
| 45 (2023) | ESM port: every extension rewritten to `import`/`export default class extends Extension`. Forge dropped unlock-dialog mode in the 45 review, then argued it back in | Pop Shell "refactor: GNOME 45 port" 2023-10-12; Forge extension.js:78-85 comment |
| 46 | Pop Shell stacking and launcher broke, Forge needed a build change | Pop Shell "Fix launcher and stacking on Gnome 46" 2024-03-24 |
| 47 | Meta 15 API (Pop Shell); gTile compatibility release | Pop Shell "fix: gnome 47 and meta 15 support" 2024-10-09 |
| 48 | Tiling Assistant's copies of non-exported shell modules (`altTab-48.js`, `switcherPopup-48.js`); Pop Shell launcher invocation | Tiling Assistant `src/dependencies/unexported/`; Pop Shell 2025-05-21 |
| 49 | **`meta_window_maximize/unmaximize` lost their `MetaMaximizeFlags` argument and `meta_window_get_maximized` was removed** (present in mutter 48.0 window.h:266, 289-293; gone in 49.0/50.1, replaced by `get_maximize_flags`, `set_unmaximize_flags`, 50.1 window.h:289-292, 454-461). Forge ships a try/catch for both forms (window.js:965-974). Pop Shell switched to `Mtk` rectangles. `gnome-shell --nested` was replaced by `--devkit` | Mutter headers; Forge "Restore pre-GNOME 49 compatibility" 2025-09-15; Tiling Assistant commits 2026-09-20; Forge #491 "Unsupported for 49"; gjs.guide debugging page |
| 50 | **`clutter_grab_get_seat_state` removed** (present in 49.0 clutter-grab.h:46, absent in 50.1). PaperWM crashed on keybindings until "Fix for GNOME 50s API changes (GrabState and Stage Cursors)" (c0cbd80, 2026-04-09) and "on gnome 50+ grabs always succeed (wayland-only)" (29bd01a). Cursors moved to `global.stage.set_cursor_type`. GNOME 50 is Wayland-only | Mutter headers; PaperWM PR #1149; Tiling Assistant tileEditingMode.js:53 version guard |
| 51 | PaperWM: Clutter backend access, background textures, pointer tracking; Forge, gTile, Tiling Shell, Tiling Assistant metadata bumps | PaperWM commits 2026-09-20/21 (#1189, #1191) |

Lessons:
- Breakage scales with how much **shell UI internals** an extension touches: PaperWM's patches, Tiling Assistant's
  copied modules, Pop Shell's launcher. It is small for code that only uses `Meta.Window`, `global.display`, `St` and
  `Main.wm`/`Main.pushModal`. gTile and utwm stay close to that surface. utwm's whole bridge is about 500 lines that
  touch mostly stable `Meta` APIs.
- Every project still bumps `shell-version` every six months. Even an unchanged extension is refused by a new
  gnome-shell until the bump, unless the user turns off version validation. So a vase bridge needs a scheduled
  "new GNOME release" check, and it fits Chained's Upgrade flow.
- On Wayland, new or changed extension code loads only after **log out / log in**. GNOME Shell cannot restart in a
  Wayland session (gjs.guide debugging page; utwm install.rs:62). Development uses
  `dbus-run-session gnome-shell --devkit --wayland` on 49+.

---

## 4. Robustness themes seen across the projects

1. **The shell process is the compositor.** On Wayland a hang or a heavy synchronous loop in an extension freezes the
   whole desktop, and a crash ends the session. Pure-JS exceptions in signal handlers are only logged. This argues for
   keeping the in-shell part small and pushing the layout logic out-of-process, as utwm does and as Pop Shell did for
   its launcher. A crash of the Rust daemon then costs tiling, not the session.
2. **Lock and unlock re-run disable/enable.** Anything held only in the extension is lost on every screen lock unless
   `unlock-dialog` is declared and justified (Forge, utwm). With the state in a daemon, vase-core's JSON layout
   persistence keeps working across locks, and so does vase's re-matching by `(app, title)` (vase CONTEXT.md, "Layout
   persistence").
3. **Settling after a move.** Wayland clients resize when they next draw. Forge papers over this with timers (220 ms
   queue, idle renders). utwm awaits the size/position signals and returns the achieved frame. vase's `set_frame` is
   fire-and-forget today (vase-core/src/backend.rs:18), so the Linux backend should do utwm-style settling and report
   back. That also fits vase's `Resync`.
4. **Move then move+resize**, after unmaximizing. Every project does this (Forge window.js:980-981, utwm :285-286,
   Tiling Assistant).
5. **Chrome z-order** is fought for by Mutter. Chrome in `global.window_group` must be restacked on `restacked`
   (PaperWM stackoverlay.js:37-45, Pop Shell stack.ts:151).
6. **Hiding non-selected stack items:**
   - Raise/occlusion (Forge; vase today on macOS) has nothing to undo.
   - Actor `hide()` (Pop Shell) is clean visually, but if the extension errors or is disabled midway, windows stay
     invisible. Every path must `show()` them.
   - Off-screen parking fights Mutter's constraints (PaperWM tiling.js:63-67) and is unimplemented in vase anyway.
7. **App quirks** show up as tiling bugs: Chrome 149 can't join a Forge tab group (#535); XWayland video bridge
   artifacts (#309); JetBrains focus trouble in Pop Shell (#1312). Budget for an exception list. vase already has
   "rule-excluded apps" (CONTEXT.md, Managed / floating).
8. **Modal grabs** can race the modifier release (PaperWM navigator.js:136-145). The release event leaks if the modal
   is popped on the press (navigator.js:180-185).
9. **Runtime changes to system settings** (Tiling Shell's edge-tiling and keybinding overrides) need crash-safe
   restore logic. On this desktop they would also show up as **Drift** against Chained's dconf-managed GNOME settings.
   The settings the Tiler needs freed should be owned by Chained's Keymap/GNOME layer, not toggled by the extension.
10. **Coexistence:** Tiling Assistant (system, `tiling-assistant@ubuntu.com`) and the user extensions on this desktop
    (`blur-my-shell`, `just-perfection`, `rounded-window-corners`, all in `~/.local/share/gnome-shell/extensions/`) are
    live while the Tiler is built. `rounded-window-corners` and `blur-my-shell` wrap window actors with effects, so
    actor hiding or clones should be tested with them on. Two tilers moving the same windows will fight, so the
    assessment must cover "Tiling Assistant disabled". Only `enabled-extensions` changes for that;
    `allow-extension-installation=false` gates only the extensions.gnome.org install path (`installExtension`,
    js/ui/extensionDownloader.js:32-37 at 50.1), not a locally installed extension.
11. **Bus-name trust.** Any process can own a session-bus name. utwm checks that the bridge's owner is gnome-shell's
    own unique name. In the other direction the extension should accept calls only from the one client it has bound
    (utwm's single-client rule). That is far better than Window Calls / Window Control's "anyone can move anything".
12. **Maintenance risk:** Forge (one maintainer, searching since 2023) and Pop Shell (vendor moved on) show that big
    in-shell tilers rot when their one maintainer leaves. A small bridge is cheaper for one owner to keep porting.

---

## 5. What the vase Linux extension should copy

- **utwm's split, adapted.** The extension is a bridge and renderer. vase-core in the Rust daemon owns the tree,
  keymap, registry and persistence. This maps almost one-to-one onto vase's `Backend` trait (vase-core/src/backend.rs:7-67):
  - `displays` ↔ `Screens`, `list_windows` ↔ `Windows`, `set_frame` ↔ `Place`, `focus` ↔ `Activate`,
    `raise` ↔ `Place(raise)`/`raise`, `title`, `minimized` and `fullscreen` ↔ facts, `close` ↔ a new bridge method calling `Meta.Window.delete()` (utwm has none).
  - `launchable_apps`/`launch` can stay in the daemon via `Gio.DesktopAppInfo`-equivalent Rust crates or a bridge call.
- **utwm's bridge safety rules:**
  - an owner check against `org.gnome.Shell`;
  - a single bound client, with grabs released when it vanishes;
  - keycode-only accelerators restricted to modifier chords;
  - an API version on both sides with a test;
  - window calls refused while locked;
  - `unlock-dialog` with the guideline's justification comment;
  - pending calls answered when the bridge is disabled.
  - The code is MIT, so it can be borrowed with attribution.
- **utwm's `_settle`:** after a move, await the frame and return the achieved frame.
- **Forge's move order:** unmaximize, `remove_all_transitions`, `move_frame`, then `move_resize_frame`.
- **A modal prefix via `Main.pushModal`**, with PaperWM's two fixes (check the modifier race; pop on release) and
  Tiling Assistant's structure (an St widget in `Main.uiGroup` with sub-modes).
- **Chrome as St actors in the shell** (Forge/Pop Shell tab strips in `global.window_group`, restacked on
  `restacked`). On GNOME Wayland an external process can't make always-on-top, non-activating overlays: Mutter has no
  layer-shell. So the vase tab bar, stack bars, focus border, switcher, picker and help sheet have to be drawn by the
  extension. vase already splits *what* a bar shows (`chrome::bar`) from *how* it is drawn (a style), and a native
  style "lays that one out itself and hands back the tab spans clicks route against" (vase CONTEXT.md, "Chrome"). So
  the daemon can send the bar model over D-Bus and the extension can render it as a GNOME/Adwaita-style native style.
- **Pop Shell's launcher split** for anything list-like or text-like (pickers, switcher, rename, command line): an St
  modal in the shell, with matching and logic in Rust over IPC.
- **Raise/occlusion first for stacks** (Forge's approach, and what vase does on macOS). Treat Pop Shell-style actor
  hiding as a later option, with show-on-every-exit-path and tests under rounded-window-corners/blur-my-shell.
- **Meta.Window ids** (`get_id()`, a u64 stable for the session) for `WindowId`. Use app-id
  (`Shell.WindowTracker`) + title for vase's cross-reboot re-matching.

## 6. What it should avoid

- Putting the layout engine in the shell (Forge, PaperWM, Pop Shell). Every bug or hang there is a desktop bug, all
  of it must be re-ported each cycle, and vase-core would have to be rewritten in JS.
- Monkey-patching shell UI internals or vendoring non-exported shell modules (PaperWM `patches.js`, Tiling Assistant
  `unexported/`). Those are the biggest sources of per-release breakage.
- Mode-by-keybinding-set-swap (Pop Shell, gTile) for the prefix. It is leaky, and it turns plain keys into global grabs.
- Changing GNOME's own settings at runtime (Tiling Shell). It causes Drift under Chained and needs crash-restore
  logic. Let Chained's dconf layer free any conflicting keys up front.
- An unauthenticated "anyone may move windows" D-Bus API (Window Calls / Window Control).
- Timer-based "wait a bit, then re-render" choreography (Forge's 220 ms queue) in place of awaiting Mutter's signals.
- Off-screen parking (Mutter constraints; PaperWM had to use clones to get around them).
- Copying code from Window Calls (no licence).
- Bundling the Rust binary inside the extension directory, if extensions.gnome.org is ever a target ("Extensions MUST
  NOT include binary executables or libraries", gjs.guide review guidelines). For a self-installed extension it only
  matters as hygiene. Chained installs the daemon and the extension as two files from one reviewed commit.

## 7. Open questions this raises for the wayfinder

- Can the bridge be fast enough for vase's "re-render + raise" on tab switch, given per-window D-Bus round trips with
  settling? A batched `PlaceMany` call is probably needed. utwm moves one window per call.
- Should the daemon start the bridge's modal grab itself (a "BeginModal" request after the prefix `Activated` signal,
  costing one round trip), or should the extension enter the modal locally on the prefix and stream keys? The local
  version avoids the latency and the first-key race.
- Chrome rendering in St means a third chrome painter, next to AppKit and Direct2D. How much of
  `chrome::powerline`'s layout can be done in Rust and sent as positioned boxes and text, so the extension only paints?
- What is the kill chord on Linux: a bridge-level chord that drops all grabs and unhides everything, even with the
  daemon hung?
- vase CLAUDE.md asks for `just check`/`just test` on macOS/Windows. The Linux bridge needs a JS lint and a protocol
  test (as utwm does in extension.rs:332), plus a manual checklist item for the log-out step.

## Sources

- Repos (cloned, commits above): https://github.com/forge-ext/forge, https://github.com/paperwm/PaperWM,
  https://github.com/pop-os/shell, https://github.com/domferr/tilingshell, https://github.com/gTile/gTile,
  https://github.com/ubuntu/Tiling-Assistant, https://github.com/johncarmack1984/universal-tiling-window-manager
  (PR #8: https://github.com/johncarmack1984/universal-tiling-window-manager/pull/8), https://github.com/ickyicky/window-calls
- Issues: forge #336, #491, #535, #175, #299, #309; PaperWM #70 (navigator.js comment), #1147/#1149; Pop Shell #1312
- Mutter headers at 48.0 / 49.0 / 50.1: https://gitlab.gnome.org/GNOME/mutter/-/raw/50.1/src/meta/window.h,
  …/clutter/clutter/clutter-grab.h (local copies in `scratch/gnome/`)
- gnome-shell 50.1: js/ui/main.js (pushModal :697), js/ui/shellDBus.js (:70, :204-231, :321),
  js/ui/extensionDownloader.js (:32-37)
- gjs.guide review guidelines: https://gjs.guide/extensions/review-guidelines/review-guidelines.html
- gjs.guide debugging (devkit, no restart on Wayland): https://gjs.guide/extensions/development/debugging.html
- D-Bus window-op extensions: https://extensions.gnome.org/extension/4724/window-calls/,
  https://github.com/carlo9890/gnome-window-control, https://github.com/kemallette/ws-dbus
- Local system (read only): `/usr/share/gnome-shell/extensions/tiling-assistant@ubuntu.com/metadata.json`,
  `dpkg -S`, `~/.local/share/gnome-shell/extensions/`
- vase: `/home/martin/src/vase-chained/CONTEXT.md`, `vase-core/src/backend.rs`, `vase-core/src/input/keymap.rs`,
  `docs/adr/*`
