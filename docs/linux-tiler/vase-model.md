# vase's own model and code: what a Linux (GNOME Shell) backend must provide

Lens: vase-core's concepts, the platform seam the macOS and Windows backends implement, threading, config and state, and what the Linux Tiler would need. Research only: nothing in either repo was changed.

**Source:** `/home/martin/src/vase-chained` at `022340fe91015f7d11c358eb9dccc1c4e6abdbdd` (2026-08-30, `main`, which is also `origin/main`). File:line references are to that tree. GNOME API facts were checked against the online mutter docs (https://gnome.pages.gitlab.gnome.org/mutter/meta/). Those docs track mutter main (they report "API Version 52"), not the 50.x tag, so every GNOME call named below still has to be checked against the 50.1 sources. That is a wayfinder task.

---

## 1. Size

The workspace has three crates (`Cargo.toml`: `vase-core`, `vase-macos`, `vase-windows`) and no Linux crate. The justfile already builds and tests `-p vase-core` alone on Linux (`justfile`, `host_crates`).

| Crate | src lines (no tests) | test lines | `#[test]`s | Deps |
|---|---|---|---|---|
| vase-core | **5,928** | 2,303 (`tests/unit/**`) | 183 | serde, serde_json, toml, toml_edit, all pure Rust |
| vase-macos | 2,738 (+79 example/build) | ~1 | 3 | accessibility-sys, core-foundation, core-graphics, objc2-* |
| vase-windows | 2,597 (+80 example) | 166 (`*_test.rs`) | 11 | windows 0.62 (Win32, D2D, DWrite, DComp, DWM), windows-numerics |

How vase-core's lines split by module:

| Module | Lines | What it is |
|---|---|---|
| `model/` (mod, command, query, topology, reducer/{mod,panes,stacks,tabs,windows}) | 1,180 | layout state and the pure reducer `apply(Model, Command) -> (Model, Vec<Effect>)` |
| `tree/` (mod, query, edit/{split,stack}) | 359 | the pane tree `Node` and its edits |
| `geometry.rs`, `focus.rs` | 103 + 38 | `Rect`, layout, `screen_of`, `any_covered`, directional `neighbor` |
| `input/` (mod, keycode, keymap, entry, switcher/{mod,nav}) | 676 | `KeyRouter`, the default keymap, number entry, list navigation |
| `chrome/` (mod, bar, deck, paint, powerline, help, theme) | 1,300 | what every surface shows, the `Painter` trait, powerline layout, the help sheet, palettes |
| `daemon/` (mod, effects, reconcile, input, bar, prompt, app_focus, help, switcher/*, pane_picker/*) | 1,670 | `Daemon<B: Backend, C: Painter>`, the orchestrator |
| `config.rs`, `state.rs`, `registry.rs`, `backend.rs`, `lib.rs` | 254 + 99 + 126 + 108 + 15 | TOML config, JSON state, per-window bookkeeping, the platform seam |

Each existing backend is about 2.6–2.7k lines. More than half of that is chrome painting: the macOS `overlay/` is 1,579 lines and the Windows `chrome/` is 1,305. The rest is the `Backend` impl (macOS `ax/` + `cg.rs` ≈ 440; Windows `backend.rs` 428), the input hook (141 / 184), the keycode map (111 / 45), the menu-bar or tray item (134 / 219), paths (30 / 33) and `bin/vase.rs` (170 / 254).

---

## 2. Concepts (vase-core)

The canonical glossary is `CONTEXT.md`. The code is named after it.

- **Window**: a native OS window. `WindowId(pub u64)` is opaque, `Copy`, and serde-serialisable (`tree/mod.rs:11`). Each backend chooses what goes in it: macOS uses `kCGWindowNumber` (`vase-macos/src/cg.rs:87`), Windows uses the HWND value (`id_of(hwnd)`, `vase-windows/src/backend.rs:81`).
- **Pane / Split / Stack**: `Node::{Leaf{id, pane}, Split{dir, ratios, children}, Stack{id, items, selected}}`, with `Pane::{Empty, Window(id)}` (`tree/mod.rs:22-50`). `PaneId(u64)` comes from a monotonic allocator in `Model::next_pane_id`. Empty panes are first-class: focus can sit on one, and the **pane picker** opens over it to fill it with an existing window or a launched app (`daemon/pane_picker/`).
- **Stack**: its items share one rect. Only the selected item is shown, the rest are occluded, and a vase-drawn local bar sits on top. Switching items is a pure raise: the reducer emits only `FocusWindow` (ADR 2026.08.03 off-screen-parking, refinement 2026-08-06).
- **Tab**: `Tab{root, focused, name, zoomed}` (`model/mod.rs:18`), owned by a **Screen**: `Screen{rect (usable area), tabs, current}`. The **Model** is `{screens, focused_screen, names: HashMap<WindowId,String> (stack-item names), next_pane_id}` (`model/mod.rs:56`). There is no workspace or session level. Every screen's tabs show in one tab bar on the main display, numbered flat in bar order (`model/query.rs` `screen_tab`).
- **Zoom**: `Tab.zoomed` is set per tab, so the zoom stays with its tab (commit 1f08a71). `placements()` gives the zoomed focused window the whole screen rect (`model/query.rs:44-52`). The bar marks the tab with `∗` (`chrome/mod.rs` `ZOOM_MARK`).
- **Deliberate placement**: a new manageable window becomes a new tab on the screen its frame centre falls on (`Command::AddWindow`, `daemon/reconcile.rs:27-40`). The exception is a pending launch into a focused empty pane, which fills that pane (`FillPane`). Nothing is auto-tiled.
- **Hiding**: windows of non-current tabs are left where they are and **occluded by raising** the current tab's windows. Off-screen parking is described in an ADR but not implemented (`CONTEXT.md` Architecture; `Effect::Render` doc, `model/command.rs:104-110`).
- **Effects**: there are exactly two. `Render(Vec<(WindowId, Rect)>)` places windows and raises the newly visible ones. `FocusWindow(WindowId)` gives a window OS focus (`model/command.rs:104`). The daemon executes them in `daemon/effects.rs:13-63`. It raises only newly shown windows, then on focus re-fronts every visible pane of the focused tab so it is never partly covered (`tab_partly_covered`, which uses `geometry::any_covered`).
- **Registry**: per-window data kept outside the layout: `{app, title, original frame (restored on exit), minimized, placed}` (`registry.rs:9-21`).
- **Managed / floating**: decided by `backend::manageable(w)`: `layer == 0 && !title.is_empty() && w,h >= 40` (`backend.rs:106`), plus the backend's `tileable()` check, which fails open (`reconcile.rs:137-140`). There is **no rules engine**: `CONTEXT.md` mentions "rule-excluded apps", but nothing in the code implements them.
- **Workspace**: an OS desktop vase does not manage. A managed window that is on another workspace keeps its tab, which gets the `◇` mark (`off_workspace`, `reconcile.rs:150-170`).
- **Windowless apps**: running apps with no window, shown as bare icons after the tabs. Clicking one calls `activate`. Controlled by `[tabbar] windowless = off|seen|all`.
- **Fullscreen**: when the frontmost window is fullscreen, the daemon hides **all chrome** (`reconcile.rs:180-188`, `refresh()` in `daemon/mod.rs`). The window itself stays managed. This is the only fullscreen handling there is, and nothing is ignored.

### Input model (prefix, stack prefix, modes)
- `KeyCode` is platform-neutral: a US-layout `Char(c)` or a named key (`input/keycode.rs`). `Mods{meta, ctrl, alt, shift}`, where meta is Command on macOS, Win on Windows, and would be Super on Linux (`input/mod.rs:14`).
- `KeyRouter` (`input/mod.rs:149-185`) holds a list of `(prefix chord, bindings)` modes. `key()` returns `Decision::{PassThrough, Consume, ConsumeAndRun(cmd)}`. A prefix chord arms its mode; the next key resolves through that mode's table, and **an unbound key after the prefix is PassThrough**, not swallowed.
- Two modes are hardcoded (`input/keymap.rs:8-18`): **prefix `Alt+A`** (tabs and panes) and **stack prefix `Alt+E`**, which runs the same table but redirects the tab keys (`. , t Shift+, Shift+. 1-9`) at the focused stack (`bindings_nested`, `keymap.rs:96-110`). The bindings are: arrows focus; Shift+arrows or Shift+HJKL resize; Meta/Ctrl/Alt+HJKL or arrows move a pane; `\` and `-` split; `z` zoom; `x` break pane; `s` stackify; `[` and `]` cycle the stack; `c` new tab; `. ,` next/prev tab; `Shift+, Shift+.` move tab; `Shift+[ Shift+]` send the tab to the other monitor; `1-9` select a tab (multi-digit with a 120 ms entry timeout, `input/entry.rs:4`); `l` last; `w` switcher; `t` rename; `:` command line (`q`, `rename`, `close`, `split`, `vsplit`, `zoom`, `tab N`; `daemon/prompt.rs:96-131`); `m` warp cursor; Ctrl/Meta/Alt+R resync; `?` help; `q` quit. **The keymap is not user-configurable** (`keymap.rs:1`).
- **Modal precedence** comes before the router, in `Daemon::intercept_key` (`daemon/input.rs:20-58`): help sheet → pending launch (Esc only) → prompt/command line (typing) → switcher → pane picker → multi-digit tab entry → **`[[app_focus]]` global hotkeys** (direct chords, e.g. `ctrl+grave` → Ghostty) → **sticky repeat** (after a resize, bare Shift+arrows keep resizing; after a move-tab, bare Shift+, and Shift+. keep moving). `Daemon::modal()` (`input.rs:15`) mirrors these guards for a platform that has to decide without asking the daemon.
- The **kill chord** is **Ctrl+Alt+Meta+Esc**. It is hardcoded in each backend's hook, not in the core (`vase-macos/src/event_tap.rs:104-108`, `vase-windows/src/hook.rs:118-122`). It sets an atomic quit flag, and the run loop then restores every window's original frame.
- None of these exist yet: Pause, a game ignore list, ignoring fullscreen windows, configurable keys, launch-at-login. A grep of vase-core finds no pause, ignore or exclude logic. This agrees with the Chained spec, where they are Tiler-track deliverables (`.scratch/chained-v1/spec.md:654,1039`).

---

## 3. The platform seam

A backend supplies **four things** to vase-core, plus a `main`.

### 3a. `Backend` trait (`vase-core/src/backend.rs:7-79`). Pull-based and synchronous, `&mut self`
| Method | Contract | macOS | Windows |
|---|---|---|---|
| `displays() -> Vec<Display>` | sorted left→right, top→bottom; `Display{id: u32 stable across hotplug, bounds, work_area}` in **global top-left points** | NSScreen | EnumDisplayMonitors, id = hash of device name |
| `list_windows()` | windows on the current workspace, **frontmost first** (the first entry is used as the OS focus) | CGWindowList on-screen | EnumWindows (z-order) filtered to the current virtual desktop |
| `all_windows()` | all workspaces (called only when something is missing) | CGWindowList all | EnumWindows |
| `set_frame(id, rect)` | move and resize | AX size→pos→size | un-maximise, SetWindowPos, re-place on DPI change |
| `focus(id)` | keyboard focus to that specific window, wherever it is | private SkyLight `_SLPSSetFrontProcessWithOptions` + `SLPSPostEventRecordTo` + AXRaise (`ax/skylight.rs`) | SetForegroundWindow with the foreground-lock-timeout dance |
| `raise(id)` | z-order raise without focus | AXRaise + app frontmost | SetWindowPos(HWND_TOP, NOACTIVATE) |
| `forget(id)` | drop cached handles | | |
| `title(id)` | live title, polled for every managed window on every reconcile | AXTitle | GetWindowText |
| `tileable(&info) -> Option<bool>` | a real top-level window, not a transient | AXSubrole == AXStandardWindow | caption, not a tool window, not owned, not cloaked |
| `minimized`, `minimized_info`, `set_minimized` | | AXMinimized | IsIconic / ShowWindow |
| `fullscreen(&info)` | | AXFullScreen | covers the monitor bounds and has no caption |
| `close(id)` | ask the window to close | AXCloseButton press | WM_CLOSE |
| `launchable_apps()`, `launch(app)` | names for the picker; spawn a new instance | scan `.app` dirs; `open -na` | shell Applications folder |
| `warp_cursor(x,y)` | | CGWarpMouseCursorPosition | SetCursorPos |
| `badged_apps()` (default empty) | apps with a notification badge | scrape the Dock's AX tree | (default) |
| `running_apps()` (default empty) | | NSWorkspace regular apps | (default) |
| `activate(app)` (default `launch`) | front a running app | `open -a` | |
| `default_bar_position()` (default Bottom) | | Bottom | Top (the taskbar is at the bottom) |

`WindowInfo{id, pid: i32, app: String (display name; config `app_focus.app` matches it), title, frame: Rect, layer: i64 (0 = normal app layer)}` (`backend.rs:83-92`). The core uses `layer` only for `manageable` and `tab_partly_covered`.

### 3b. `Painter` trait (`vase-core/src/chrome/paint.rs:48-83`). The chrome
`measure(text, size) -> f64` (synchronous, called during layout), `bar(&Bar) -> BarHits` (returns clickable x-spans), `prompt`, `hide_bar`, `stack_bars(&[Bar]) -> Vec<Hits>`, `panes(&[(Rect, focused)])` (empty-pane placeholders), `focus_border(Option<Rect>)`, `list(ListAt, header, rows, selected)` (switcher and picker), `hide_list`, `help(&HelpLayout)`, `hide_help`, `hide_bars`, `hide_all`, `prewarm_icon(app)`.
- The core decides what is shown and where (`chrome/deck.rs`, `Deck::sync` is the only redraw path). The **powerline** style is laid out in the core (`chrome/powerline.rs`) and needs only `measure`. A **native** style lays itself out in the platform crate and hands back hit spans (ADR 2026.08.21). A platform with no native style draws powerline; Windows does that today.
- Clicks: the platform hook hands **global points** to `Daemon::click(px, py)`, which routes them against the hit maps from the last sync (`daemon/bar.rs:79-95`, `deck.rs` `route_click`). The surfaces must be **non-activating, always on top and on every workspace** (ADR 2026.08.10).
- Theme and mark are `thread_local!` globals (`chrome/theme.rs:239-258`), set from config.

### 3c. The input hook contract
Each backend owns a global hook and calls `EventTap::install` or `Hooks::install` with the core's `router()` plus four callbacks (`vase-macos/src/event_tap.rs:28-63`, `vase-windows/src/hook.rs:62-75`):
`on_command(InputCommand)` → `Daemon::run`; `on_click((x,y)) -> bool consumed` → `Daemon::click`; `on_key_intercept(Key) -> bool consumed` → `Daemon::intercept_key`; `on_arm(bool)` → `prefix_armed` and a refresh (the prefix dot).
The per-key algorithm, identical on both platforms: kill chord → translate scancode to `KeyCode` (an unknown key passes) → `intercept_key` → `router.key` → **return consume or pass synchronously to the OS**. Both hooks see **every KeyDown system-wide** and decide whether to swallow it on the spot. Both use `catch_unwind` so a panic still passes the event through. macOS taps only KeyDown and LeftMouseDown, never FlagsChanged, so modifier presses always reach apps. Windows injects a masking Ctrl tap after a swallowed Alt chord so menus don't open (`hook.rs:27-35`).

### 3d. The run-loop contract (`bin/vase.rs`, near-identical on both)
At startup: read displays, pick `main_screen` = **the display at (0,0)**, load config, cut the screen rects with `chrome::usable(work_area, is_main, bar_position)` (which subtracts the bar strip), adopt manageable windows into a `Registry`, call `state::restore(saved, live, rects)`, build `Daemon::new(model, backend, painter, registry, paths, main_screen, screens, display_ids)`, arm the `RestoreGuard` (restore on drop or panic), install the hooks, then do the first `Render` and `FocusWindow`.
Each loop iteration: wait up to **25 ms** for events, then call `tick_tab_entry`, `tick_switcher`, `tick_pane_picker` and `tick_reframe`. Every 4th wake (**about 100 ms**) it calls `reconcile()` and `warm_icons()` (macOS `bin/vase.rs:131-170`, Windows `bin/vase.rs:197-253`). Menu or tray actions (new tab, reload config, resync, help, quit) arrive as `AtomicBool` flags the loop drains (`vase-macos/src/lib.rs`, `vase-windows/src/lib.rs`). On exit it calls `restore()`, which puts every adopted window back at its **pre-adoption frame** (`daemon/effects.rs:112-124`).
`reconcile()` (`daemon/reconcile.rs:128-225`) is **polling**: it diffs the screens, diffs the windows (adopt new, keep minimized and off-workspace ones, forget closed ones), follows OS focus edge-triggered from the frontmost window with a cooldown of 3 polls after vase's own focus move, refreshes titles, checks fullscreen, expires pending launches, and every 5th poll refreshes badges and windowless apps. The core uses **no OS events at all**.

### Config file and state file
- **Config**: `config.toml` in the platform support directory (`~/Library/Application Support/vase/` on macOS, `%APPDATA%\vase\` on Windows; `paths.rs` in each crate). The schema is in `config.rs` and the documented default is `docs/vase.example.toml`, which is embedded with `include_str!` and **written on first run if the file is missing** (`Config::ensure`). Keys: `favorites` (written back by vase through `toml_edit`, keeping comments; `save_favorites`), `focus_border`, `[theme] style/name/9 colour overrides`, `[tabbar] mark/position/windowless/titles`, `[[app_focus]] key/app`. Reloaded from the menu (`reload_config`, `daemon/app_focus.rs:17-42`). Chained's rule (spec:373) that config is a "modify target keeping Tiler-written keys such as favourites" matches this: `favorites` is the one key vase rewrites, and it also creates the file.
- **State**: `state.json` next to the config. It holds the serde-JSON `Model` plus `(id, app, title)` for each window, and is **saved after every dispatch** (`daemon/mod.rs` `save_state`, `state.rs:37`). At startup `match_windows` (`state.rs:71`) re-matches by id within a session, then by `(app, title)`, then by app alone. This is platform-neutral.

### Threading
- **Single-threaded by design.** The daemon lives in `Rc<RefCell<Daemon>>` on the main thread, and the hook callbacks run on that thread (CFRunLoop source; WH_*_LL hooks are delivered to the installing thread's message loop). On Windows, calls such as SetForegroundWindow, SetWindowPos and DComp commits pump messages, so a hook can **re-enter** while the daemon is already borrowed. `try_borrow_mut` plus a `Deferred` queue handles that, and an atomic `MODAL` mirror decides whether to swallow the key meanwhile (`vase-windows/src/bin/vase.rs:85-160`). The only other thread is the Windows icon resolver (`chrome/icons.rs:84`, mpsc, and a posted `ICONS_RESOLVED`).
- The core's `thread_local!` theme and `Rc` state mean the whole core is `!Send`. Any host has to run it on one thread.

---

## 4. Pure logic vs platform glue

| Part | Lines | Classification |
|---|---|---|
| `tree/`, `model/` (reducer, queries, topology), `geometry.rs`, `focus.rs` | ~1,680 | **Pure**, fully unit-tested, reusable as-is on any host |
| `input/` (router, keymap, keycode, entry, switcher nav) | 676 | **Pure** logic, but it **assumes an observe-every-key host** (see §5.1) |
| `state.rs`, `registry.rs`, `config.rs` | 479 | Pure apart from `std::fs` reads and writes (state, config, ensure, save_favorites). Paths are injected. |
| `chrome/` bar, deck, powerline, help, theme | 1,300 | Pure layout and the click-routing model. Painting is behind `Painter`. |
| `daemon/` | 1,670 | Generic over `Backend + Painter`, with no OS calls, but it encodes **macOS/Windows assumptions**: polling, frontmost-first focus following, `main_screen` = display at (0,0) (`reconcile.rs:61`), the bar strip subtracted from the work area (`chrome::usable`), `std::time::Instant` read directly (9 call sites: `daemon/bar.rs`, `effects.rs`, `switcher/keys.rs`, `pane_picker/keys.rs`, `input/entry.rs`) |
| `backend.rs` | 108 | The seam |
| each `vase-<os>` crate | ~2.6–2.7k | **All glue**: Backend impl, hook, keycodes, painter (more than half), tray or menu, paths, main and the run loop |

So all 5.9k lines of vase-core are reusable as a library. About 4.2k of them are pure model and layout with no assumption about the host. The ~1.7k daemon lines are reusable if the host can offer vase's polling, synchronous model.

---

## 5. What a Linux backend has to provide, and where GNOME doesn't fit vase's model

ADR-0001 (Chained): on GNOME Wayland only the compositor can move other apps' windows, so the window-control half of the backend **has to run inside gnome-shell as a GJS extension**. Every `Backend` primitive maps onto something in-shell that is cheap to call. The input contract and the process and threading model are the parts that **don't map**.

### 5.1 Input: the biggest mismatch with vase's model
vase's design (§3c) assumes a hook that **sees every KeyDown and swallows or passes it synchronously**. Everything depends on that: the prefix router (an unbound second key is PassThrough), the modal overlays that take typed text (prompt, switcher filter, picker), multi-digit tab entry, sticky repeat with *bare* Shift+arrows, and the `[[app_focus]]` chords. Inside GNOME Shell an extension gets **grabbed accelerators** (`Meta.Display.grab_accelerator` / `accelerator-activated`, or `Main.wm.addKeybinding`) and **modal grabs** (push a modal grab while armed or in an overlay, so every key goes to the shell until it is popped). It does not get a pass-through observer for ordinary keys. A Linux host therefore has to:
- grab Alt+A, Alt+E, every `app_focus` chord and the kill chord as accelerators;
- on arming, or while any overlay or sticky mode is active, take a modal grab, feed keys to the core's router and `intercept_key`, and release the grab when the core leaves those modes;
- decide what happens to an **unbound key after the prefix**. vase passes it through, but under a modal grab it is swallowed. Re-injecting it needs a Clutter virtual input device. tmux itself ignores an unbound key after its prefix, so swallowing is a defensible choice, but it is a behaviour change from vase;
- treat **sticky resize and move-tab** as an explicit grabbed mode with a visible state, since bare keys can't be watched without a grab;
- follow the EGO rule that "all signals related to keyboard events MUST be disconnected in `unlock-dialog` session mode" (https://gjs.guide/extensions/review-guidelines/review-guidelines.html).
- One upside: the chrome is in-shell actors that receive their own clicks, so there is no global mouse hook. The core's coordinate-based `click(px, py)` routing can still be used.

### 5.2 Each Backend method mapped to GNOME (to verify at 50.x)
| vase needs | Likely GNOME Shell primitive | Notes |
|---|---|---|
| `WindowId(u64)` | `Meta.Window.get_id()`, which returns `uint64_t` (https://gnome.pages.gitlab.gnome.org/mutter/meta/method.Window.get_id.html) | Session-scoped like CG ids and HWNDs, so state re-matching by (app, title) works unchanged |
| `pid`, `app`, `title` | `get_pid()`, `Shell.WindowTracker.get_window_app(w).get_name()`, `get_title()` / `notify::title` | `app` is the display name `app_focus` matches against (e.g. "Ghostty") |
| `frame`, `set_frame` | `get_frame_rect()`, `move_resize_frame(user_op, x, y, w, h)` (https://gnome.pages.gitlab.gnome.org/mutter/meta/method.Window.move_resize_frame.html) | Un-maximise or un-tile first, as Windows does. Ubuntu's Tiling Assistant would fight over the same windows, so it has to be off wherever the Tiler runs |
| `focus`, `raise` | `activate(timestamp)` (https://gnome.pages.gitlab.gnome.org/mutter/meta/method.Window.activate.html) with `global.get_current_time()`; `raise()` | Easier than either existing OS: no focus-stealing prevention against the compositor itself |
| `list_windows` frontmost-first, `all_windows` | `global.display.get_tab_list(...)` / `sort_windows_by_stacking`, filtered by the active workspace or `is_on_all_workspaces()` | The core wants z-order front first and the first entry as focus. GNOME has `focus-window` and `restacked` signals directly |
| `tileable`, `layer` | `get_window_type() == NORMAL`, no `get_transient_for()`, not `is_skip_taskbar()`, `allows_resize()` | Map non-NORMAL to `layer != 0` |
| `minimized`, `set_minimized`, `fullscreen` | the `minimized` property, `minimize()`/`unminimize()`, `is_fullscreen()` (Display `in-fullscreen-changed`) | |
| `close` | `delete(timestamp)` | |
| `displays` (stable `u32` id, bounds, work area) | `global.display.get_n_monitors()` / `get_monitor_geometry(i)`, `Main.layoutManager.getWorkAreaForMonitor(i)`; id from a hash of the monitor connector | **Core assumption to change:** the main display = the one at (0,0) (`reconcile.rs:61`, both `bin/vase.rs`). GNOME's primary monitor need not be at the origin, so a `Display.primary` flag or a Backend hook is needed |
| bar strip | `Main.layoutManager.addChrome(actor, {affectsStruts: true})` would shrink the work area natively | The core already subtracts the strip itself (`chrome::usable`), so a backend must do one or the other to avoid subtracting it twice. GNOME's top bar is at the top, so `default_bar_position` = Bottom |
| `launchable_apps`, `launch`, `activate`, `running_apps` | `Shell.AppSystem` (`get_installed` / `lookup_app`, `app.open_new_window(-1)`, `app.activate()`, `get_running()`) | |
| `badged_apps` | no shell API; the Unity LauncherEntry D-Bus convention is what Ubuntu Dock reads | Can stay at the default empty set |
| `warp_cursor` | Clutter seat `warp_pointer(x, y)` | |
| Painter | St / Clutter actors in the shell's chrome layer; app icons from `Shell.App.get_icon()`; text measured with Pango in-process | Non-activating and always on top come for free in-shell. Either powerline (core layout plus `measure`) or a new native "Adwaita" style |
| kill chord | an accelerator that runs `restore()` and releases all grabs | `disable()` must also restore. Shell may call `disable()` and `enable()` across lock and unlock (https://gjs.guide/extensions/topics/session-modes.html), so a lock would restore every frame and an unlock would re-tile from `state.json` (re-matched by id within the session) unless the extension declares `unlock-dialog` |
| config and state paths | `$XDG_CONFIG_HOME/vase/config.toml`, `$XDG_STATE_HOME/vase/state.json` (new in a Linux `paths.rs`) | Chained's modify-target rule carries over |
| run loop | GLib main loop: a 25 ms timeout source for ticks and 100 ms for reconcile, or better, signals | |

### 5.3 Where the core lives: three shapes, judged by how much of vase they reuse
1. **Rust daemon plus a thin extension over D-Bus.** A `vase-gnome` crate implements `Backend` and `Painter` as calls to an extension-exposed session-bus interface. It reuses all of vase-core. The costs:
   - every `Backend` call is a round-trip. The 100 ms poll calls `title()` for each window, so the interface would want batched calls;
   - `Painter::measure` is synchronous and per-string, which favours a native style laid out in the extension;
   - **key decisions can't wait for an IPC hop** under a modal grab, so the router at least effectively moves into the extension;
   - it adds a second process and its lifecycle;
   - it puts a window-control D-Bus API on the session bus, which any same-user process could call unless the sender is restricted;
   - it adds a D-Bus crate (zbus is pure Rust), which the fork's cargo-deny bans on "network crates and new sys crates" (spec:868-871) would have to be reviewed for;
   - EGO: "Processes MUST be spawned carefully and exit cleanly".
2. **Port the core to GJS inside the extension.** No IPC, signal-driven, and the natural GNOME shape. It reuses none of the 5.9k lines or the 183 tests, and the Linux Tiler would drift from upstream vase, so the fork's monthly upstream merges would stop helping Linux. TOML config needs a JS parser, because GJS has none built in.
3. **vase-core compiled to WebAssembly and loaded by the extension.** One source and one model. All four deps are pure Rust. Known blockers:
   - `std::time::Instant::now()` (9 sites) is unsupported on `wasm32-unknown-unknown`, so a clock has to be injected;
   - `std::fs` in config.rs and state.rs needs host-provided I/O;
   - the `!Send` single-thread design suits wasm;
   - **whether GJS (SpiderMonkey ESR) exposes `WebAssembly` was not confirmed** from primary sources: none found. A 2020 Go issue only shows that Node-style glue fails (https://github.com/golang/go/issues/41482);
   - EGO says "Extensions MUST NOT include binary executables or libraries". That matters only if the extension is ever published, since Chained installs it locally (and sets `allow-extension-installation=false`).

Whichever shape is chosen, the core needs small changes: a primary-display flag in place of "display at (0,0)"; an option for the backend to own the bar strip; a host-agnostic clock; and an input contract that also allows a **grab-based host** (e.g. `KeyRouter` / `Daemon` exposing "I'm armed or modal, keep the grab" alongside `Decision`, which `Daemon::modal()` already half-provides).

---

## 6. Asides found on the way (for Chained, not for this lens)
- `vase-chained/CONTEXT.md` and ADR 2026.08.03 say there is "no private-API injection beyond the read-only `_AXUIElementGetWindow` lookup", but `vase-macos/src/ax/skylight.rs:10-44` also calls private SkyLight `_SLPSSetFrontProcessWithOptions` and posts synthetic event records with `SLPSPostEventRecordTo` (the byte layout is from yabai). This matters for the spec's "baseline review of the input and I/O code" before the Mac permission grant (spec:647).
- `Chained/docs/keymap.md` (Tiler section, lines 21-28) lists only Alt+A. The stack prefix **Alt+E** and the kill chord **Ctrl+Alt+Super(Win/Cmd)+Esc** are not in it, although spec:652 says "The kill chord is documented". Chained's "nothing binds Alt+A" rules (spec:147) have no Alt+E counterpart.
- vase writes `config.toml` itself on first run (`Config::ensure`) as well as `favorites`. Chained's modify target has to tolerate both.
