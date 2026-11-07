# The Linux Tiler protocol, version 1

The wire between the two pieces of the Linux Tiler: the GNOME Shell extension (inside the compositor) and `vase-gnome`
(the Rust daemon that runs vase-core's `Daemon`). Written for M1a (`wayfinder.md` §5) from the map's §3 and the M0b
spike findings (`docs/linux-spike/S2.md`, `S3.md`, `S4.md`, `S6.md`, `S9.md`). Both sides test against this document:
the daemon at L0 with a fake extension (M1a), the extension at L1 (M1b, M1c).

Status: **v1, the daemon's half implemented in M1a** (`vase-gnome`, tested at L0 against a fake extension; the
extension's half lands in M1b and M1c). The amendments M1a made while building it are listed in §12.3. Words in bold
caps (**MUST**, **MUST NOT**, **SHOULD**) are requirements a test checks; the rest explains them. Terms are
CONTEXT.md's: window, pane, tab, stack, screen, chrome, prefix.

Contents:
- §1 the transport and the peer check;
- §2 framing: lines, the version, the caps;
- §3 the common types;
- §4 the handshake;
- §5 extension to daemon;
- §6 daemon to extension;
- §7 the lifecycle (§3.1 of the map, revised after S9);
- §8 key ownership (§3.3 of the map, revised after S2, S3 and S9);
- §9 the core seams each message relies on;
- §10 errors;
- §11 what the L0 tests check;
- §12 decisions this spec takes that the map left open, and what is still open.

---

## 1. Transport

| Rule | From |
|---|---|
| The daemon binds `$XDG_RUNTIME_DIR/vase/gnome.sock` itself; the directory is 0700. No socket activation: under activation the peer is systemd's user manager and the peer check can't work. | F13, S9 |
| Before `bind`, the daemon takes `flock(LOCK_EX \| LOCK_NB)` on `$XDG_RUNTIME_DIR/vase/gnome.lock`. If the lock is held, it logs one line and exits 0 **without touching the socket file**. Holding the lock, it removes a stale socket file (a killed daemon leaves one) and binds. | S9 item 5 |
| One connection at a time. A second connection while one is live is closed at once with an `error` `busy` (§10). | S9 stub |
| The extension connects with `Gio.SocketClient.connect_async` to a `Gio.UnixSocketAddress`, and on failure retries with backoff: 100 ms, doubling, capped at 2 s. A restarted daemon is picked up within one step. The extension **never** spawns the daemon. | S9 item 5, §3.1 |
| On connect, before reading or writing anything, the extension checks the peer: `Gio.Socket.get_credentials()` uid equals its own, and `/proc/<pid>/exe` equals `/usr/local/libexec/vase/vase-gnome`. An `exe` that ends in ` (deleted)` is refused with the log line `daemon binary replaced; restart vase-gnome.service`. Any refusal closes the connection and backs off. | §3.5, F29 |
| The daemon checks the uid of what connects (`SO_PEERCRED`) and closes anything else without a line. It does not check the shell's `exe`: confined by Landlock it may not read `/proc/<pid>/exe` of a process outside its domain (Landlock scopes ptrace access, which that read needs). | M1a |
| The daemon is installed 0755 and never calls `PR_SET_DUMPABLE(0)`: either makes `exe` unreadable to the shell. | F29 |
| The check is defence in depth: any same-uid process can reach the compositor in other ways. | `devtest-security.md` §5.5 |

## 2. Framing

- **One JSON object per line**, UTF-8, ended by `\n` (0x0A). No other whitespace is significant. A line that is not a
  JSON object is a fatal `bad_json` error.
- **Every line carries `"v": 1` and `"t": "<type>"`.** A line whose `v` is not 1 is refused with a fatal `version`
  error; the connection closes and the extension backs off normally and logs one line per run of refusals, not one per
  attempt (S9: "4 refusals in 2.5 s", then the normal backoff).
- **1 MiB per line, both directions.** A line is at most 1,048,576 bytes before its `\n`. Each reader **MUST** read in
  chunks with its own line splitter (`read_bytes_async(65536)` in the extension, not `DataInputStream.read_line_async`,
  which has no cap: an 8 MB line stalled painting for about 150 ms in S9) and **MUST** fail with a fatal `too_long`
  error as soon as the bytes buffered since the last `\n` pass the cap, **without** reading the rest of the line.
- **No synchronous I/O on the socket in the extension.** One blocked write froze the compositor for 1 s (S9). All
  writes go through one async queue (`write_bytes_async`), capped at 1 MiB of unsent bytes. Hitting the cap means the
  daemon is hung: the extension disconnects (EOF handling, §7.2) and backs off. A tripwire flags synchronous stream
  calls on the connection (M1b).
- **Draw lists have a budget:** one `draw` line **SHOULD** stay under 16 KiB (S9: 16 KB per reply cost nothing
  measurable) and **MUST** stay under 64 KiB. The daemon checks the encoded size; over 64 KiB it drops list rows from
  the bottom until it fits and logs once.
- **Unknown fields are ignored.** An unknown `t` is answered with a non-fatal `unknown_type` error and otherwise
  ignored, so a v1 peer can add an optional message without a version bump. Anything else that changes meaning bumps
  `v`.
- **`seq`.** Each side numbers the lines it sends, from 1 on each connection, in a `seq` field on every line. A reply
  names what it answers in `re` (the other side's `seq`). Examples below that leave out `v`, `seq` or `ack` do so for
  space only.

## 3. Common types

| Type | Wire form | Notes |
|---|---|---|
| Rect | `[x, y, w, h]`, numbers | Global stage (logical) pixels, top-left origin, the space `Meta.Window.get_frame_rect()` and `Main.layoutManager.monitors` use. The Tiler never converts X11 geometry (S4 rule 4). |
| Point | `[x, y]` | Same space. |
| Window id | number | `Meta.Window.get_id()`. Random 32-bit base per shell process (F23), so below 2^53. vase's `WindowId`. |
| Display | `{"id": u32, "connector": "HDMI-3", "bounds": Rect, "work_area": Rect, "scale": 1.5}` | `id` is a 32-bit FNV-1a hash of the connector name, so it is stable across a hotplug or a reorder (vase's `Display::id`). `work_area` from `Meta.Workspace.get_work_area_for_monitor` of the active workspace. |
| Key | `{"code": 30, "mods": ["alt"]}` | `code` is the **evdev code**: Clutter's hardware keycode minus 8 (F22: the only input source is `us`, and vase's keymap uses US positions). `mods` is a set, any order, of `"shift"`, `"ctrl"`, `"alt"`, `"super"`, from `SHIFT_MASK`, `CONTROL_MASK`, `MOD1_MASK` and `MOD4_MASK` (S2: inside a grab Super is `MOD4_MASK`, never `SUPER_MASK`). The daemon maps codes to vase's `KeyCode`; the extension never sees `KeyCode`. |
| Time | `time_us`, number | Clutter's event time, `get_time_us()`: CLOCK_MONOTONIC in µs, the clock `GLib.get_monotonic_time()` reads. Only compared within one side; the daemon uses its own clock for its timers. |
| App | `"app": "Files"`, `"app_id": "org.gnome.Nautilus.desktop"` | `app` is `Shell.App.get_name()`: vase's display name, which `[[app_focus]] app` and the picker match. `app_id` is the desktop-file id, used for `launch`, `activate` and the placement hint; empty when the window has no app. |
| Instance | `"<boot_id>:<pid>:<starttime>"` | The compositor instance: `/proc/sys/kernel/random/boot_id`, the shell's pid, and field 22 of `/proc/self/stat` (start time in clock ticks). A lock, an unlock and an extension rebase keep it; a re-login or a shell restart changes it. |

A **window object** (snapshot and `window`):

```json
{"id": 2183041, "pid": 4120, "app": "Ghostty", "app_id": "com.mitchellh.ghostty.desktop", "title": "~",
 "frame": [2560, 32, 960, 1048], "layer": 0, "tileable": true, "minimized": false, "fullscreen": false,
 "on_workspace": true, "x11": false}
```

- `layer` is 0 for a window on mutter's normal layer and nonzero otherwise (vase's `WindowInfo::layer`: `manageable`
  and `tab_partly_covered` only care whether it is 0).
- `tileable` is vase's `Backend::tileable`: `Meta.WindowType.NORMAL`, no transient parent, not skip-taskbar. It is
  evaluated in `configure` for a new window, because on Wayland the title and app id are not set at `window-created`
  (S6).
- `on_workspace` is true for a window on the active workspace or on all workspaces (with the owner's
  `workspaces-only-on-primary=true`, every window on a non-primary monitor, F11). vase's `list_windows` is the windows
  with `on_workspace`; `all_windows` is all of them.
- `x11` marks an Xwayland window (for the settling tolerance, §6.2).

## 4. Handshake

```
extension                                     daemon
   | connect, peer check (§1)                   |
   |------------- hello ----------------------->|  both sides send hello at once
   |<------------ hello ------------------------|  and check the other's v (and the build)
   |------------- measure (glyphs, titles) ---->|  at the sizes the daemon's hello named
   |------------- snapshot -------------------->|
   |                                            |  lifecycle transition (§7.1)
   |<------------ grabs ------------------------|
   |<------------ effects (first render) -------|
   |<------------ draw -------------------------|
   |<------------ placement_hint ---------------|
   |<------------ mode -------------------------|
   |  live: deltas, keys, clicks / effects, draw, mode, replay
```

**Extension hello:**

```json
{"v": 1, "t": "hello", "seq": 1, "role": "extension", "instance": "3f2a…:5222:4123", "build": "<tree hash>",
 "shell": "50.1"}
```

**Daemon hello:**

```json
{"v": 1, "t": "hello", "seq": 1, "role": "daemon", "build": "<build hash>", "pid": 6101, "mdwe": true,
 "landlock": "full", "sizes": [12, 13], "glyphs": "0123456789 …∗◇★"}
```

- `build` is the hash of the reviewed SHA's daemon build; the extension compares it with the value Chained's sudo step
  wrote next to the extension, and refuses a mismatch (a fatal `build` error) with the log line naming both.
- `mdwe` is `prctl(PR_GET_MDWE)` at start (S10: `/proc` doesn't show it); doctor compares it. `landlock` is `"full"`,
  `"partial"` or `"off"`, present only if Q2.3 is yes.
- `sizes` are the chrome font sizes the daemon measures at (`FONT_SIZE` 12 and the mark's 13 today); `glyphs` is the
  small table the extension measures once per connection, for strings that are not titles (§5.6): the digits, the
  space, the ellipsis and the marks (`ZOOM_MARK`, `WORKSPACE_MARK`, `FAVORITE_MARK`).
- Neither side sends anything but `hello` before it has the other's hello. The extension then sends `measure` for the
  glyph table and for every title in the snapshot, then the `snapshot`; the daemon sends nothing more until it has the
  snapshot, except a `pong` or an `error`.
- Every answer, the snapshot's included, goes out in one order: `effects`, `draw`, `measure_request`, `replay`,
  `placement_hint`, and last `mode`. The snapshot's answer starts with `grabs`.

## 5. Extension to daemon

Every message below also carries `ack`: the `seq` of the newest daemon `effects` batch the extension has finished
applying (0 for none). The daemon's mirror uses it to tell a delta that predates its own effect from one that follows
it (§6.1).

### 5.1 `snapshot`

```json
{"v": 1, "t": "snapshot", "seq": 2, "ack": 0, "displays": [Display, …], "primary": 1,
 "windows": [window object, …], "focus": 2183041,
 "apps": [{"app": "Calculator", "app_id": "org.gnome.Calculator.desktop"}, …]}
```

- `displays` are ordered left to right, then top to bottom (vase's `Backend::displays`). `primary` is
  `Main.layoutManager.primaryIndex` mapped into that order; the daemon's `Backend::primary_display` returns it
  (seam D1).
- `windows` are **frontmost first**, from `global.display.sort_windows_by_stacking`, reversed. The first window with
  `on_workspace` is what vase's `reconcile()` takes as the OS focus.
- `focus` is `global.display.focus_window`'s id, or `null`.
- `apps` (optional) are the installed apps the picker offers to launch (`Shell.AppSystem` `get_installed()`), by
  display name and desktop-file id. vase names apps by display name; the daemon maps a name to its id for `launch`,
  `activate`, the placement hint and icons, from this list or else from a window of that app.
- At least one display, and a `frame` for every window; otherwise the snapshot is a fatal `bad_json`.

### 5.2 Deltas

| `t` | Fields | Sent when |
|---|---|---|
| `window` | a window object's fields, at the top level of the line | a window becomes known (`configure` for a new one, S6; at `enable()` for windows already open) or any field of it changes (`notify::title`, `size-changed`, `position-changed`, `notify::minimized`, `notify::fullscreen`, `workspace-changed`) |
| `window_gone` | `id` | `unmanaging` |
| `stacking` | `ids`, frontmost first | `global.display` `restacked` |
| `focus` | `id` or `null` | `notify::focus-window` |
| `displays` | `displays`, `primary` | `monitors-changed`, `workareas-changed`, a primary change |

- Deltas are sent in the order the signals fired, never batched across a main-loop turn. The socket keeps them in
  order; what can still cross is a delta and a daemon effect (§6.1).
- A `window` delta for a window the daemon placed reports the frame **as mutter has it**, not the requested rect.
  While a placement of that window is in flight (§6.2) its `frame` is left out of the delta; `settled` reports it.

### 5.3 `key`

```json
{"v": 1, "t": "key", "seq": 41, "ack": 17, "key": {"code": 43, "mods": []}, "time_us": 81234567890,
 "via": "grab"}
```

- A key **press** the daemon owns under §8: a grabbed chord's accelerator firing (`"via": "accelerator"`), a key the
  stage grab took (`"via": "grab"`), or a temporary accelerator (`"via": "temporary"`). Releases are never sent: vase
  routes presses only, and the extension times its own pop on releases (§8).
- The extension **MUST NOT** send a modifier-only press (S9: the chord's own Alt can reach the grab first) or a key
  whose `time_us` is earlier than the arming chord's (S2: IBus can hand back a key typed before Alt+A after the grab is
  up).
- The kill chord and Pause are never sent as keys (§5.5).
- Every `key` gets exactly one `mode` reply (§6.4). No reply within **200 ms** means the daemon is hung: the extension
  ends the state (§8) and disconnects.
- The daemon runs each key the way the macOS and Windows hooks do: `Daemon::intercept_key`, then the `KeyRouter`
  (`ConsumeAndRun` goes to `Daemon::run`), so one keymap serves all three platforms.

### 5.4 `click`

```json
{"v": 1, "t": "click", "seq": 52, "ack": 20, "at": [2700, 1060], "button": 1, "time_us": 81240000000,
 "target": "chrome"}
```

- `"target": "chrome"`: a press on a reactive chrome actor (the tab bar, a stack bar). The daemon runs
  `Daemon::click(x, y)`.
- `"target": "outside"`: a press outside an open overlay, taken by the overlay's grab. The daemon closes the overlay
  and answers with a `replay` of the click (§6.5), which the extension injects **on the button's release**, where the
  pointer is then (S3).

### 5.5 `end`, `kill`, `pause`

| `t` | Fields | Meaning | Daemon does |
|---|---|---|---|
| `end` | `what`: `"prefix"`, `"overlay"`, `"sticky"`; `reason`: `"timeout"`, `"revoked"`, `"focus"`, `"missed_reply"` | The extension ended a state on its own (§8) | prefix: `KeyRouter::disarm` and `Daemon::cancel_prefix` (K1). sticky: `Daemon::end_sticky` (K3). overlay: §12.2. Then a `mode` reply |
| `kill` | – | The kill chord (Ctrl+Super+Alt+Esc): the extension has already popped every grab | Quitting (§7.1): `restore()`, `bye`, exit |
| `pause` | `paused`: bool | The Pause chord (M6, seam P1) | Reserved in v1: answered with a non-fatal `unsupported` error |

### 5.6 `measure`

```json
{"v": 1, "t": "measure", "seq": 9, "ack": 0, "texts": [{"text": "~/src", "size": 12, "advances": [7.1, 14.9, …]}]}
```

- `advances` holds the cumulative advance after each Unicode scalar value of `text`
  (`Pango.Layout.index_to_pos` at each scalar's byte index, in stage pixels), so `advances.len()` equals the number of
  `char`s and the width of the first `n` chars is `advances[n-1]`.
- The extension sends: the `glyphs` table from the daemon's hello, one entry per character, at each size, once per
  connection; every title at each size **with** its `window` delta (in the line just before it), so the daemon never
  lays out a title it can't measure; and any text a `measure_request` asked for (§6.7).
- The daemon's `GnomePainter::measure` uses the title's array for a title and its prefixes, adds the `…` advance for an
  ellipsized prefix (an approximation where the two glyphs kern), and sums glyph-table advances for anything else
  until a `measure` answer replaces the guess. L1 golden tests compare with Pango within a tolerance (M4).

### 5.7 `settled`

```json
{"v": 1, "t": "settled", "seq": 60, "ack": 21, "id": 2183041, "re": 21, "frame": [2560, 32, 960, 1048]}
```

The achieved frame after a `place` from batch `re` settled (§6.2). It is never a reverted rect: a frame equal to an
older request is re-sent once and reported only after that second settle.

### 5.8 `ping`

`{"v": 1, "t": "ping", "seq": 70, "ack": 21}` every 2 s while nothing else was sent. The daemon answers `pong` with
`re`. No `pong` within 2 s means the daemon is hung (§7.2).

## 6. Daemon to extension

### 6.1 `effects`

```json
{"v": 1, "t": "effects", "seq": 22, "re": 41,
 "effects": [{"k": "place", "id": 2183041, "rect": [2560, 32, 960, 1048]}, {"k": "raise", "id": 2183041},
             {"k": "focus", "id": 2183041}]}
```

One batch per `Daemon::dispatch` (or per tick that moved anything), in the order `GnomeBackend` was called. `re` is the
key or click that caused it, when one did.

| `k` | Fields | `Backend` call | Extension does |
|---|---|---|---|
| `place` | `id`, `rect` | `set_frame` | §6.2 |
| `raise` | `id` | `raise` | `Meta.Window.raise()` |
| `focus` | `id` | `focus` | `win.activate(global.display.get_current_time_roundtrip())`, not `Main.activateWindow` (S9) |
| `minimize` | `id`, `minimized` | `set_minimized` | `minimize()` / `unminimize()` |
| `close` | `id` | `close` | `delete(time)` |
| `launch` | `app_id` | `launch` | `Shell.AppSystem` `open_new_window(-1)`, only for an id in `get_installed()` (§3.2 of the map) |
| `activate` | `app_id` | `activate` | `Shell.App.activate()`, same check |
| `warp` | `at` | `warp_cursor` | the extension's virtual pointer, `notify_absolute_motion` |

- `GnomeBackend` leaves out a `place` whose rect the extension already reports for that window with no placement in
  flight: it would change nothing on screen, and leaving it out is what keeps a daemon restart or a reconnect from
  sending any `place` at all.
- The extension looks every `id` up again (`list_all_windows` index kept by `window-created` and `unmanaging`) and
  **ignores** an effect on an unknown or unmanaged id.
- **The mirror is optimistic.** `GnomeBackend` applies `place`, `raise` and `focus` to its own mirror as it emits them,
  so the reads that follow in the same dispatch (`tab_partly_covered` calls `list_windows()` straight after `focus`)
  see the new stacking. A delta whose `ack` is older than the newest batch that touched a window updates every field of
  that window **except** the ones that batch set (frame, stacking position, focus); a delta with a newer `ack` is
  authoritative. That is how the mirror converges after delayed or reordered deltas (the M1a property test).

### 6.2 Settling (the extension's rules, from S4)

- Every `place` is `move_frame(true, x, y)` then `move_resize_frame(true, x, y, w, h)`, for Wayland and X11 alike.
  `move_resize_frame` alone loses a move across monitors.
- **One request in flight per window.** A newer `place` for a window whose last one hasn't settled replaces the held
  rect and is sent when the first settles.
- A request has settled at its first `size-changed` (or at once if the size didn't change) plus a quiet period (50 ms
  as the starting default; M1b tunes it at L1, and the desktop's number comes from the L3 feel check).
  If the frame then equals an older request rather than the newest, the newest is re-sent once (S4: a late ack can
  revert it, 8 of 18 across monitors with `workspaces-only-on-primary=true`).
- Tolerance: exact on Wayland; for an `x11` window short by less than one resize increment per axis, never larger.
  The daemon accepts a `settled` frame within that tolerance as the placement.
- vase's own `tick_reframe` (re-assert a frame 150 ms after a move to another monitor) still runs in the daemon. It
  arrives as one more `place` of the same rect, which the one-in-flight rule holds until the first settles.

### 6.3 `placement_hint`

```json
{"v": 1, "t": "placement_hint", "seq": 23, "default": [0, 0, 2560, 1440],
 "apps": {"org.gnome.Calculator.desktop": [2560, 32, 960, 1048]}}
```

Where the next new window goes, kept current at all times, because the extension places a new window inside
`configure`, which can't wait for a reply (S6).

- `default` is the rect a new managed window gets as its own tab: the content rect of the focused screen (a new tab's
  single pane), or `null` for "leave it where mutter puts it".
- `apps` maps an `app_id` to a rect that wins over `default`: a pending launch's empty pane (its `app_id`).
- The daemon sends a new hint after every dispatch that changed the focused screen, its rect, or the pending launch,
  and after every `displays` delta. The extension keeps the newest only.
- In `configure`, with `get_is_initial()` true and the window `tileable`, the extension calls `set_position` and
  `set_size`, **always both**, from the hint (for an Xwayland window with a mutter-drawn frame, `set_size` with the
  learned title-bar height taken off). It **never** calls `get_rect`, `set_rect`, `get_position` or `get_size` on a
  `Meta.WindowConfig` (S6: `get_rect()` segfaults the shell). A window that got no initial `configure` (mapped
  maximized or fullscreen, or open before `enable()`) is moved once on `first-frame` with an ordinary `place`.
- The extension then reports the window as usual (`measure`, then `window`). The daemon adopts it the vase way
  (`reconcile`: `FillPane` for a pending launch, else `AddWindow` on the screen its frame is on) and may move it again
  like any window.

### 6.4 `mode`

```json
{"v": 1, "t": "mode", "seq": 24, "re": 41, "mode": "sticky",
 "keys": [{"code": 105, "mods": ["shift"]}, {"code": 106, "mods": ["shift"]}, …]}
```

The daemon's `Daemon::key_mode()` (seam K2) after it handled `re`, sent as the **last** line of that key's answer
(after its `effects`, `draw` and `replay`), and also whenever the mode changes on a tick (a launch's window arrived, a
tab-number entry committed on its 120 ms timer).

| `mode` | Extra fields | From `KeyMode` |
|---|---|---|
| `"idle"` | – | `Idle` |
| `"prefix"` | – | `Prefix`: the router is armed, or a `prefix-<number>` waits for a digit |
| `"overlay"` | `kind`: `"help"`, `"prompt"`, `"switcher"`, `"picker"` | `Overlay` |
| `"sticky"` | `keys`: the repeat keys, each with `["shift"]` | `Sticky(keys)`, mapped to evdev codes |
| `"pending_launch"` | – | `PendingLaunch` |

`kind` comes from `vase-gnome`, which reads it from the daemon's public overlay fields (`prompt`, `switcher`,
`pane_picker`; the sheet is what is left), so `KeyMode` itself doesn't need it.

### 6.5 `replay`

```json
{"v": 1, "t": "replay", "seq": 25, "re": 42, "keys": [{"code": 30, "mods": ["alt"]}]}
{"v": 1, "t": "replay", "seq": 31, "re": 52, "click": {"button": 1}}
```

Keys or a click the extension took from the user and must hand to the app (S3), sent before the `mode` that ends the
state that took them:

- a key the router answered `PassThrough`: a key with no row after a prefix ("any key with no row passes through to
  the app", `keymap.md:70`), Esc included;
- "Alt+A, then Alt+A": the second Alt+A has no row either, so it is replayed the same way as one Alt+A
  (`input/mod.rs:166-172`); `InputCommand::SendPrefix` is unbound and stays a no-op until seam K4 (M1c);
- a click outside an overlay.

The extension injects them through its own virtual keyboard and virtual pointer **after** `popModal`, in order, under
the rules of the seat (F26):
- a key only once the user has released it (a replayed press of a held key is dropped), a click on the button's
  release;
- a key carries the modifiers held at replay time ("hold Alt, tap A twice" replays exactly Alt+a);
- a replayed chord that is one of the Tiler's own accelerators passes a one-shot `filter-keybinding` fence for its
  press and its release, with a 100 ms fallback reset, so it doesn't re-arm the prefix.

In M1a the daemon emits `replay`; the extension side lands in M1c.

### 6.6 `grabs`

```json
{"v": 1, "t": "grabs", "seq": 20, "chords": [{"key": {"code": 30, "mods": ["alt"]}, "role": "prefix"},
 {"key": {"code": 18, "mods": ["alt"]}, "role": "prefix"},
 {"key": {"code": 41, "mods": ["ctrl"]}, "role": "app_focus"}]}
```

The chords the extension grabs with `grab_accelerator` in Idle: the two prefixes and the `[[app_focus]]` chords from
the config. Sent after the handshake and after a config reload. `role` tells the extension which chords arm a stage
grab synchronously in their callback (`prefix`) and which only forward a key (`app_focus`). The kill chord and Pause are
not in it: the extension owns them (`addKeybinding`, `NON_MASKABLE`, a compiled schema), so a daemon can't take them
away.

### 6.7 `draw` and `measure_request`

```json
{"v": 1, "t": "draw", "seq": 26, "panes": [{"rect": [1280, 0, 1280, 1440], "focused": false}],
 "border": [0, 0, 1280, 1440], "list": {"rect": [0, 0, 1280, 1440], "centered": false, "header": "", "rows": [ … ],
 "selected": 0}}
```

At most one line per answer (§4), carrying only the surfaces that changed since the last `draw`; the first after a
connect names every surface (the example: two empty panes, the picker over the focused one, so only the other gets a
placeholder). Each surface field is either an object (draw it), `null` (hide it) or absent (unchanged). Coordinates
are stage pixels. Hiding everything (`hide_all`, a fullscreen window, a detach) is every surface `null`.

| Field | `Painter` call | Object |
|---|---|---|
| `bar` | `bar`, `prompt`, `hide_bar` | `{"rect", "items": [draw item…], "hits": [[x0, x1]…]}`: the powerline bar laid out in Rust (`chrome::powerline`), so the daemon already knows the hit spans. The command line is `{"rect", "prompt": text, "edge": "top" \| "bottom", "items": [], "hits": []}` |
| `stack_bars` | `stack_bars` | an array of `bar` objects |
| `panes` | `panes` | `[{"rect", "focused"}…]`, the empty-pane placeholders |
| `border` | `focus_border` | a Rect, or `null` |
| `list` | `list`, `hide_list` | `{"rect", "centered": bool, "header", "rows": [{"number", "prefix", "icons": [app_id…], "label", "dim", "off_workspace", "favorite", "current"}…], "selected"}` (`SwitchRow`) |
| `help` | `help`, `hide_help` | `{"rect", "texts": [{"rect", "text", "style": "title" \| "section" \| "keys" \| "label"}…], "cells": [{"rect", "kind": "plain" \| "active" \| "ghost"}…]}`: the `HelpLayout`, already placed (text rects card-local) |

A draw item is one of `{"k": "rect", "r": Rect, "fill": "#rrggbbaa", "radius": n}`, `{"k": "poly", "pts": [Point…],
"fill"}`, `{"k": "text", "at": Point, "text", "size", "color", "bold": bool}` and `{"k": "icon", "r": Rect, "app_id"}`.
M1a builds every surface; the bar's items are a first cut (a rect per tab, its texts and icons, the prefix dot) that
M4 finishes and checks against Pango.

`measure_request` (`{"t": "measure_request", "texts": ["my tab"], "sizes": [12]}`) asks for texts the daemon has no
array for (a tab name the user typed). The extension answers with `measure`; the daemon redraws when it arrives.

### 6.8 `bye`, `pong`

- `pong`: `re` names the `ping`.
- `bye` (`{"t": "bye", "reason": "stop" | "quit"}`): the last line before the daemon closes the socket on purpose.
  `"stop"` is a detach (SIGTERM); `"quit"` follows a `restore()`. The extension handles it as EOF (§7.2), and logs the
  reason instead of an error.

## 7. Lifecycle

### 7.1 The daemon

The extension is disabled at every lock and every rebase (F7); the daemon is stopped at log-out and restarted after
every upgrade (S9). So the daemon has an explicit "extension gone" state, and only a quit ever restores.

| State | Entered by | What runs | Sends | Leaves to |
|---|---|---|---|---|
| **Starting** | the unit starts: lock, bind (§1) | nothing; no `Daemon` yet | its `hello` on accept | **Connected** on a valid hello and snapshot: `state::load`, `state::restore` against the snapshot, `Daemon::new` with `main_screen` from `primary` (D1), the first render |
| **Connected(instance)** | a valid hello and snapshot | effects, the 25 ms ticks, the 100 ms `reconcile()` | everything in §6 | **Disconnected** on EOF, a fatal error, or 6 s with no line (twice the ping interval plus slack); **Stopping**; **Quitting** |
| **Disconnected(instance)** | the above | **nothing**: no reconcile, no ticks, no effects. `Daemon::detach()` once (seam L1): save, drop half-typed input, hide chrome | – | a hello with the **same instance**: the snapshot is diffed into the mirror, `refresh()`, then reconcile resumes and forgets only the windows that really closed. A **new instance** (a new shell: re-login while the user manager lingered): drop the `Daemon` and go through Starting's path, which re-matches by `(app, title)` because window ids are new (F23) |
| **Stopping** | SIGTERM (log-out, `systemctl --user restart`, an upgrade) | `Daemon::detach()`: save and hide chrome; **never `restore()`** | `bye` `"stop"` | exit 0 |
| **Quitting** | `q`, `:quit`, `kill` | `restore()`: every window back to its pre-adoption frame | the restore's `effects`, then `bye` `"quit"` | exit 0 (the unit's `Restart=on-failure` doesn't restart it) |

- The freeze is the platform loop not calling `reconcile()` or the ticks (F8), not a core seam.
- A second daemon started against a live one's lock exits without touching the socket (§1).

### 7.2 The extension

| State | Entered by | Holds | Leaves to |
|---|---|---|---|
| **Off** | `disable()`, or before `enable()` | nothing | Connecting on `enable()` |
| **Connecting** | `enable()`; EOF; a refusal | the kill chord and Pause only; the backoff timer | Handshake on a connect that passes the peer check |
| **Handshake** | connected | as Connecting | Live on the daemon's hello with `v` 1 and the expected build; Connecting on a refusal |
| **Live** | the hello, then its own snapshot | the grabs of §8 | Connecting on EOF, a fatal error, a full write queue, a missed `pong` or a missed key reply; Off on `disable()` |

- **EOF, from any state**, in one callback (S9: within 0.3-70 ms of a SIGKILL): pop the modal, ungrab every chord but
  the kill chord and Pause, hide the chrome, keep the placement it already applied, back off.
- `disable()` tears down in `try`/`finally` steps: pop the modal; uninhibit every window it inhibited (its own set);
  ungrab every chord; destroy the chrome; close the socket (the daemon sees EOF and freezes); drop the virtual devices.
  `enable()` cleans up after itself if it throws.
- **What a lock costs:** a teardown, a reconnect (about 10 ms after `enable()`, S9) and a snapshot, with no window
  moving. Checked at L1 in M1b.

## 8. Key ownership

The extension holds the state and owns every grab; the daemon's `mode` moves it. Under GNOME a key is either grabbed
(the app never gets it) or not seen at all (S2: an event filter and `captured-event` saw 0 of 4 client keys), so each
state names exactly what the Tiler takes.

| State | Entered by | Grab | `actionMode` | Ends on | Keys in flight |
|---|---|---|---|---|---|
| **Idle** | Live; a `mode` `idle` | `grab_accelerator` for the `grabs` chords, allowed in `Shell.ActionMode.NORMAL` only (S2: in the overview a shell entry has focus and IBus can reorder keys). The kill chord and Pause through `addKeybinding`, `NON_MASKABLE` | – | – | an `app_focus` chord: sent as a `key` (`via: "accelerator"`) |
| **Prefix** | a `prefix` chord's callback calls `Main.pushModal` **synchronously**, then sends the chord as a `key`; no check that Alt is still held (S2) | stage grab | NONE | a `mode` other than `prefix`, **then** the release of the key that ended it **and** no key pressed under the grab still down (popping on the press leaks the release, S2; a replay needs the user's release, F26); **3 s with no key** (`end` `timeout`; fired at 3002 ms, S2); `revoked` **turning true** (`end` `revoked`; it flips back later, F28); 200 ms with no `mode` (`end` `missed_reply`, then disconnect); EOF; the kill chord | every press goes to the daemon in order as a `key`, except modifier-only presses and keys older than the arming chord. Keys answered with `replay` are injected after the pop |
| **Overlay** | a `mode` `overlay` | stage grab | NORMAL, so GNOME's Alt+Tab, Super, the lock and VT switching keep working | a `mode` other than `overlay`; a click outside (`click` `outside`, replayed on release); a focus change (`end` `focus`); `revoked` turning true (`end` `revoked`: Alt+Tab's switcher revokes the grab and moves focus while the overlay stays armed, S2, so both exits are needed); EOF | every press goes to the daemon. Alt+A firing its accelerator inside the overlay (NORMAL lets it through) is sent as a `key` from the callback, not a new prefix (S3: the grab actor never sees that `a` press) |
| **Sticky** | a `mode` `sticky` | a temporary `grab_accelerator` for each of its `keys`, with **`TRIGGER_RELEASE`** (without it the key's release reaches the app, S2) | – | **500 ms after the last repeat** (`end` `timeout`; tmux's `repeat-time`, because GNOME can't show the Tiler "any other key", F28); a focus change (`end` `focus`); a prefix chord; the kill chord | none; other keys reach the app. A repeat key goes to the daemon as a `key` (`via: "temporary"`) |
| **Pending launch** | a `mode` `pending_launch` | a temporary `grab_accelerator` for Esc only, with `TRIGGER_RELEASE` | – | a `mode` other than `pending_launch` (the window mapped, the daemon's 10 s cap, or Esc) | Esc as a `key`; every other key reaches the apps. A Linux deviation: vase swallows them elsewhere, but here only a stage grab could, and it would take the pointer too (F4) |
| **Fullscreen focused** | the focus window is fullscreen (local, no round trip) | the prefix and `app_focus` chords **ungrabbed**; the kill chord and Pause kept | – | focus leaves the fullscreen window | – |
| **Paused** | the Pause chord (M6) | the Pause chord and the kill chord only | – | the Pause chord | – |
| **Disconnected** | EOF; Connecting or Handshake | the kill chord and Pause only | – | Live | – |

Rules in every state:
- **The kill chord is checked first** in every stage-grab key handler, on `CONTROL_MASK`, `MOD1_MASK` and `MOD4_MASK`,
  because a NONE grab filters every binding, NON_MASKABLE ones included (F14). Outside a grab it is an `addKeybinding`
  binding with `NON_MASKABLE`, so a client that inhibits shortcuts can't mask it. It pops every grab, then sends
  `kill`. The same holds for Pause (M6).
- **Inside a NONE prefix GNOME's own chords reach the daemon** (S2: Super+L arrives as `l` with `super`), so Alt+A then
  Super+L moves the pane instead of locking the screen.
- **A prefix ends before VT switching works again:** Ctrl+Alt+F3 is filtered while a NONE grab is up (F15), so the 3 s
  timeout is what makes the console reachable during a stuck prefix.
- **What the focused app sees** (F27): a Wayland app loses and regains the keyboard at each prefix; an Xwayland app sees
  an Alt press with no release. An M1c L1 test decides whether the extension replays an Alt release for Xwayland focus
  windows or the Keymap gets a note (Q2.1).
- The daemon's half is `Daemon::key_mode()`, `Daemon::cancel_prefix()` with `KeyRouter::disarm()`, and
  `Daemon::end_sticky()` (§9). The 3 s and 500 ms timers live in the extension, because it holds the grab and must
  release it even when the daemon is hung.

## 9. The core seams each message relies on

| Seam | vase-core API | Used for |
|---|---|---|
| K1 | `KeyRouter::disarm() -> bool`, `Daemon::cancel_prefix() -> bool` | `end` `prefix` (timeout, revoked, missed reply) and Disconnected |
| K2 | `Daemon::key_mode() -> KeyMode` (`Idle`, `Prefix`, `Overlay`, `Sticky(Vec<Key>)`, `PendingLaunch`); `modal()` is a view of it | every `mode` line |
| K3 | `Daemon::end_sticky() -> bool` | `end` `sticky` |
| D1 | `Backend::primary_display(&[Display]) -> usize`, default `origin_display` | `snapshot` and `displays` `primary` |
| L1 | `Daemon::detach()` | Disconnected and Stopping |
| K4 (M1c) | `Backend::send_key`, default no-op | `replay` for `SendPrefix`, once a binding uses it |

Each defaults to today's behaviour on macOS and Windows and is offered upstream on its own (map §3.4).

## 10. Errors

`{"v": 1, "t": "error", "seq": n, "re": m, "code": "too_long", "fatal": true, "message": "line over 1048576 bytes"}`.
A fatal error is the last line its sender writes before closing the connection; the receiver of a fatal error closes
too.

| `code` | Fatal | When |
|---|---|---|
| `version` | yes | a line's `v` is not 1 |
| `bad_json` | yes | a line is not a JSON object, or a required field is missing or of the wrong type |
| `too_long` | yes | a line passed 1 MiB (§2) |
| `order` | yes | anything but `hello` before the other side's `hello`; a delta, `key`, `click`, `end` or `settled` before `snapshot`; a second `snapshot` |
| `build` | yes | the daemon's build hash isn't the installed one (the extension refuses) |
| `busy` | yes | a second connection while one is live |
| `unknown_type` | no | an unknown `t` |
| `unsupported` | no | a known message this build doesn't do yet (`pause` in v1) |

The extension's peer check (§1) fails before any line is sent, so it has no error line: it only logs and backs off.

## 11. What the L0 tests check (M1a)

Driven by a fake extension in Rust over a real socket pair, at L0 in `chained-tiler-build`:

- a snapshot, then Alt+A and `\`, gives the expected `effects` and `mode` lines (`prefix`, then `overlay` `picker` on
  the new empty pane);
- every §8 state's entry and exit as the daemon sees it: `end` `timeout` disarming the prefix (K1), `end` `sticky`
  through K3, `pending_launch` and its keys, Disconnected emitting nothing;
- the mirror converges after random effects and delayed or reordered deltas (a property test over `ack`, §6.1);
- disconnect, then the same instance with a changed snapshot: no line while disconnected, and only closed windows are
  forgotten; a new instance re-matches by `(app, title)`;
- SIGTERM writes `state.json`, sends `bye` `stop` and no `place`;
- the primary at x=2560 gets the bar's strip and the placement hint's default rect;
- a `v` 2 line is refused; a line over 1 MiB is refused without being read whole;
- a second daemon against a live one's lock exits without touching the socket;
- the placement hint follows focus and a pending launch's pane;
- a PassThrough key and "Alt+A, then Alt+A" produce a `replay`, in order, before the `mode` that leaves `prefix`.

The core seams have their own unit tests in vase-core (`vase-core/tests/unit/daemon/`).

## 12. Decisions taken here, and what is still open

### 12.1 Decisions the map left open

1. **Accelerators are forwarded as keys.** The prefix chords, the `app_focus` chords and the temporary sticky and Esc
   accelerators all reach the daemon as a `key` with a `via`. The daemon routes them through `intercept_key` and the
   router like any key, so the keymap stays vase's alone; the extension only knows which chords arm a grab.
2. **Esc is not special in a prefix.** It goes to the daemon like any key; with no row after the prefix it is
   replayed to the app, as on the other platforms.
3. **Fullscreen and the work area have no delta of their own:** fullscreen is a field of `window`, the work areas are
   part of `displays`.
4. **`measure_request`** is added for texts that aren't titles (tab names), so the glyph table is only a first guess.
5. **The instance id** is boot id, shell pid and start time.
6. **Heartbeat:** a `ping` every 2 s, a hung daemon after 2 s without `pong`; the daemon treats 6 s of silence as EOF.

### 12.2 Open, with an owner

| Question | Where it is settled |
|---|---|
| What the daemon does with `end` `overlay` for the **picker**. The sheet, the command line and the switcher close as Esc would. The picker can't take Esc (Esc closes its pane), and it reopens on every refresh while its empty pane has focus, so it needs a "dismissed until focus returns" flag in `vase-gnome` or a core seam. | M1c, with Q2.2's feel check |
| Whether `popModal` of a revoked grab is safe (INFERENCE, `main.js:840-855`). | an L1 test in M1c |
| Whether a replayed Alt release clears the stuck Alt in an Xwayland client (F27). | an L1 test in M1c |
| The draw items for the bar, the stack bars and the sheet (M1a sends a first cut of the bar's). | M4 |
| Saving picker favorites: vase writes them into `config.toml`, which Landlock keeps read-only, so the write fails quietly. Move favorites to the state dir, or allow that one file. | M5 |
| At startup vase focuses screen 0, the leftmost (`Model::adopt`), not the screen of the focused window or the primary: on the owner's layout the first `placement_hint` and the first prefix act on the 4K monitor. Whether `vase-gnome` should seed the focused screen from the snapshot's `focus`. | M1b, with the first L1 run on two monitors |
| `pause` (P1). | M6 |

### 12.3 Amendments made in M1a, building the daemon

- **The daemon's own peer check is the uid alone** (§1): Landlock forbids the confined daemon to read the shell's
  `/proc/<pid>/exe`. The extension's full check on the daemon is unchanged.
- **One answer order** (§4): `effects`, `draw`, `measure_request`, `replay`, `placement_hint`, `mode` last; before the
  snapshot, only `pong` and `error`.
- **The snapshot carries `apps`** (§5.1), the installed apps by name and desktop-file id, because vase names apps by
  display name and the effects need the id.
- **A `window` delta carries the window object's fields at the top level** of its line (§5.2).
- **`draw` is one line per answer with only what changed** (§6.7), and hiding everything is every surface `null`; the
  `hide_all` field is gone. The command line and the shortcut sheet have their object shapes.
- **No `place` for a frame the extension already reports** (§6.1).

Where each L0 check of §11 lives: the line-by-line ones in `vase-gnome/tests/unit/host.rs` (the prefix command and its
modes, replay order, the timeout, sticky, pending launch, the hint, freezing and thawing, a new instance, SIGTERM,
the kill chord, the errors), the mirror's property test in `tests/unit/mirror.rs`, and the ones that need a process
and a socket in `tests/l0.rs` (the handshake on the wire, a v2 line, the line cap, the peer check, busy, the lock, a
restart, freezing, a re-login, SIGTERM). Landlock is `tests/landlock.rs`.
