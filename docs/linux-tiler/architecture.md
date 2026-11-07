# Linux Tiler: architecture options for running vase's model on GNOME

Research only. Nothing was installed or changed, and no shell or nested session was run. Local facts were read from the
desktop: GNOME Shell 50.1, gjs 1.88.0, libmozjs-140 (140.8.0), mutter 50.1 (`gir1.2-mutter-18`). The vase fork was read
at `022340f`. Scratch copies of the upstream sources are under `scratch/`.

## 0. Constraints, from the sources

| # | Fact | Source |
|---|---|---|
| C1 | On GNOME Wayland only the shell can move other apps' windows, so some part of the Linux Tiler must be a GNOME Shell extension. | Chained `docs/adr/0001-tiler-is-a-fork-of-vase.md` |
| C2 | GNOME 50 is Wayland-only (mutter's X11 back-end was removed in 50.alpha), so there is no X11 or xdotool fallback. | Phoronix "Mutter 50 Alpha ... X11 Backend Removed"; 9to5Linux GNOME 50 release |
| C3 | Mutter has no wlr-layer-shell, so an external process cannot draw always-on-top, non-activating chrome. **The chrome (tab bar, stack bars, lists, focus border) must be St/Clutter actors in the extension, whichever option is chosen.** | mutter issue #973 (open / won't fix); conky wiki "Mutter – WON'T FIX" |
| C4 | The core's platform seam is two **synchronous** traits. `Backend` has reads that return values (`list_windows`, `title`, `minimized`, `fullscreen`, ...) and commands (`set_frame`, `focus`, `raise`, ...). `Painter::measure(text, size) -> f64` is called many times in each layout: `ellipsize` searches with it. `Painter::bar(&Bar) -> BarHits` returns hit spans. | `vase-core/src/backend.rs:7-80`, `chrome/paint.rs:47-80`, `chrome/bar.rs:72-125` |
| C5 | The core assumes one daemon that owns the key hook and the model: "a modal prefix needs in-process state". The Windows backend already handles "the hook has to answer before the daemon can": the daemon mirrors a `modal()` flag that the hook reads without asking it. | `CONTEXT.md:49-51`; `vase-core/src/daemon/input.rs:12-16`; `vase-windows/src/lib.rs:28-37`; `vase-windows/src/bin/vase.rs:122-140` |
| C6 | The core is driven by polling: the Windows loop wakes about every 25 ms and reconciles about every 100 ms. | `vase-windows/src/bin/vase.rs:217,249` |
| C7 | The core uses `std::time::Instant::now()` (17 call sites) and `std::fs` (config and state). On `wasm32-unknown-unknown`, `std::fs` "always return[s] errors", and `Instant::now()` is `panic!("time not implemented on this platform")`. | grep of `vase-core/src`; rustc book, platform-support/wasm32-unknown-unknown; rust `library/std/src/sys/time/unsupported.rs:12-13` |
| C8 | vase-core's only dependencies are serde, serde_json, toml and toml_edit: pure Rust, with no sys crates. | `vase-core/Cargo.toml` |
| C9 | **WebAssembly is available in gjs 1.88 / mozjs 140.** I checked this in a standalone `gjs -c`: `typeof WebAssembly === "object"`, a hand-built module ran (`add(2,3)=5`), and `WebAssembly.instantiate` exists. gjs's engine setup only toggles the JIT and asm.js (`GJS_DISABLE_JIT`) and has no wasm switch. The gnome-shell 50.1 sources contain no `wasm` or `WebAssembly` reference, so the shell's GjsContext should expose it too. **I did not check this inside the running shell** (rules). The owner can type `typeof WebAssembly` in Looking Glass. | gjs `gjs/engine.cpp:242-257` @1.88.0; gjs NEWS (the only wasm mention is the old "disabled by default unless you compile mozjs yourself", 1.4x era); grep of gnome-shell@50.1 |
| C10 | e.g.o rules: extensions "MUST NOT include binary executables or libraries"; "Processes MUST be spawned carefully and exit cleanly"; code "MUST be readable and reviewable JavaScript"; licences must be GPL-2.0-or-later compatible; everything is created in `enable()` and destroyed in `disable()`; GSettings schemas sit under `org.gnome.shell.extensions`. | gjs.guide review-guidelines |
| C11 | The shell calls `disable()`/`enable()` around mode changes, **including every screen lock**, unless `metadata.json` has `"session-modes": ["user","unlock-dialog"]`. That mode has to be justified in a comment and must disconnect all keyboard signals while locked. | gjs.guide session-modes; review-guidelines |
| C12 | GNOME locks down its own session-bus window and key APIs: `org.gnome.Shell.Introspect` and `GrabAccelerator` check callers against an allowlist (portals, Settings, the media-keys daemon), and `Eval` needs unsafe mode. | gnome-shell@50.1 `js/misc/introspect.js:7-10,51`; `js/ui/shellDBus.js:27-31,70-71,204-208`; `js/misc/util.js:344+` |
| C13 | `allow-extension-installation=false` only blocks the remote install path (D-Bus `InstallRemoteExtension` → `installExtension`). Extensions installed locally under `~/.local/share/gnome-shell/extensions/` and listed in `enabled-extensions` still load. | gnome-shell@50.1 `js/ui/extensionDownloader.js:32-38` |
| C14 | The GNOME APIs a backend needs all exist at 50.1. Windows: `meta_window_move_resize_frame`, `activate`, `raise`, `minimize`, `delete`, `get_id` (a u64, matching `WindowId(u64)`). Keys: `Main.wm.addKeybinding` (GSettings-backed) or `global.display.grab_accelerator` plus `accelerator-activated` (dynamic, so it suits keys from vase's TOML). Grabs: `Main.pushModal(actor)` is a `global.stage.grab` that delivers every key to the extension until `popModal`. | mutter@50.1 `src/meta/window.h:163,210,295,301,321,422`; gnome-shell@50.1 `js/ui/windowManager.js:1078-1092`, `js/ui/shellDBus.js:322`, `js/ui/main.js:697-760` |
| C15 | gnome-shell is the Wayland compositor, so its death ends the session and every app with it. JS exceptions in signal handlers are logged and survive. Exceptions in `enable()` put the extension in the ERROR state. A JS (or wasm) busy loop, or synchronous I/O on the main thread, freezes the whole desktop. | gjs.guide architecture ("it effectively becomes a part of GNOME Shell"); mutter design |

The latency picture that follows from C14. **In every option the prefix arms inside gnome-shell.** Mutter matches the
Alt+A accelerator and the extension calls `pushModal` in the same callback. Every key after that is grabbed, so none can
leak to the app while the core decides. The consume-or-pass race the Windows hook has (C5) goes away. In its place, a
key the core does *not* bind after the prefix has to be re-injected through a Clutter virtual keyboard
(`seat.create_virtual_device(KEYBOARD_DEVICE)`) if `Decision::PassThrough` is to keep tmux's behaviour
(`vase-core/src/input/mod.rs:166-172`). Dropping it is simpler and matches tmux.

## 1. The options

### (a) A pure GJS extension that reimplements vase-core in JavaScript

- **Shape:** one process. The model, reducer, layout, keymap, config parser and powerline layout would all be ported to
  JS, and the backend and chrome written directly against Meta and St. Prior art: Forge (GPL-3.0, tree tiling in GJS),
  PaperWM, Pop Shell (TypeScript compiled to GJS), and Ubuntu's own Tiling Assistant (about 12.1k lines of JS on this
  desktop, `/usr/share/gnome-shell/extensions/tiling-assistant@ubuntu.com`).
- **Pros:**
  - The lowest possible latency: every call is in-process and synchronous.
  - No binary, no daemon and no IPC, so it is e.g.o-compatible.
  - Chained installs files only, with no Rust toolchain on Ubuntu.
  - The simplest packaging.
- **Cons:**
  - **A second implementation of the product.** That is 5.9k lines of core plus 2.3k lines (177 tests) to port. Every
    upstream merge (monthly at most, per the spec's audit) has to be ported again and reviewed twice. Behaviour, keymap
    and config drift from macOS and Windows, which defeats ADR-0001's reason for choosing vase ("one model and keymap").
  - The TOML config needs a JS TOML parser: vendored, a new dependency to audit, or hand-written.
  - The 177 Rust tests don't carry over.
  - No crash isolation (C15).
- **Effort:** the largest. A rough estimate is 6 to 10 focused weeks to parity, then a permanent porting tax.
- **Risk:** low technical risk and high maintenance and drift risk.
- **GPL:** a port of vase is a derivative work, so it is GPL-3.0. That is compatible with the shell's
  GPL-2.0-or-later.

### (b) A thin GJS extension as the platform backend, talking to a Rust daemon that runs vase-core

- **Shape:** a new crate in the fork, say `vase-gnome`, implements `Backend` and `Painter` as proxies and runs the
  existing `Daemon<B, C>` (`vase-core/src/daemon/mod.rs:38`) **with no change to vase-core**. The extension is the
  "hook + compositor" half: it owns the grabs, the Meta signals, the St chrome and the kill chord.
- **How the synchronous traits (C4) cross a process boundary:**
  - *Reads* come from a **mirror** in the daemon. The extension pushes a snapshot, then deltas from Meta signals
    (`window-created`, `unmanaged`, `notify::title`, `size-changed`, `position-changed`, `notify::minimized`,
    `notify::fullscreen`, `focus-window`, `monitors-changed`). `list_windows()`, `title()` and the rest read the mirror
    locally, and the daemon's 100 ms reconcile (C6) polls that local mirror. Reads can lag behind writes, which vase
    already tolerates: macOS AX lags too, and `focus_cooldown` exists for "the window list's lag on a just-raised
    window" (`daemon/mod.rs:78`).
  - *Commands* (`set_frame`, `focus`, `raise`, `close`, `launch`, `warp_cursor`) are queued and sent as one batch per
    `dispatch`. Nothing waits for a reply.
  - *Painter:* the proxy calls `chrome::powerline::layout(bar, mark, measure)` in Rust, as the Windows painter does
    (`vase-windows/src/chrome/mod.rs:87`). It returns `BarHits` at once and sends the finished `BarLayout` (runs and x
    positions) for the extension to draw. `measure` uses a width table the extension pushes once per font and size,
    measured with Pango. Strings the extension reports as different can be cached and re-laid out. Linux uses the
    powerline style (there is no "native" GNOME style yet), which keeps layout in the core.
  - *Keys:* while the prefix or any overlay is modal, the extension forwards each grabbed key in order to
    `intercept_key`/`KeyRouter`. The daemon replies with effects plus the `modal()` flag (the Windows mirror pattern,
    C5), and the extension calls `popModal` when the flag drops. Global chords (the prefix, the stack prefix, the
    `[[app_focus]]` hotkeys) are grabbed with `grab_accelerator` from a list the daemon sends at startup.
- **Latency:**
  - The prefix arms in-process (§0).
  - A post-prefix command costs one local round trip: an estimated tens of µs on a unix socket and well under 1 ms on
    D-Bus. Neither was measured.
  - Both are far below one 60 Hz frame (16.7 ms) and below the Wayland configure/ack round trip each resized client
    needs anyway. Keys typed quickly after the prefix are queued by the grab, so none are lost.
- **Crash isolation, the main point in its favour:**
  - A panic in the daemon kills only the daemon. The extension sees EOF, calls `popModal`, ungrabs, leaves windows where
    they are, and restarts it with backoff (or systemd's `Restart=on-failure` does).
  - A hang in the daemon is handled by a timeout in the extension: no reply within about 200 ms while modal → `popModal`
    so the keyboard is never held.
  - **The kill chord (Ctrl+Alt+Super+Esc, hard-coded as in `vase-windows/src/hook.rs:118` and
    `vase-macos/src/event_tap.rs:104`) is handled entirely in the extension** and works whatever state the daemon is
    in.
  - What is left in the shell process is a small, reviewable JS surface.
- **Transport and lifecycle (the choice inside (b)):**
  1. *The extension spawns the daemon with stdio pipes,* as in `Gio.SubprocessLauncher(STDIN_PIPE|STDOUT_PIPE)` and
     line-delimited JSON. This is the precedent Pop Shell uses for `pop-launcher` (pop-os/shell `src/utils.ts:124-136`,
     `src/launcher.ts:345-347`, `src/launcher_service.ts`).
     - Pros: no listening endpoint at all, so only the shell can talk to it (a capability by construction); the
       simplest option.
     - Con: `disable()` runs on every lock (C11), so the daemon would be stopped at each lock. vase saves state on every
       dispatch (`daemon/mod.rs:191`) and re-matches by id within a session, and Meta ids survive while the shell runs,
       so this is recoverable. But the daemon needs a "detach" exit that **does not** run `restore()` (which moves every
       window back to its original frame, `daemon/effects.rs:109-121`). That is new code in the proxy crate, not in the
       core.
     - Con: there is no systemd sandboxing.
  2. *A `systemd --user` unit with a private unix socket* in `$XDG_RUNTIME_DIR/vase/` (mode 0700). The daemon outlives
     lock and unlock and the extension reconnects. Chained can confine it: `NoNewPrivileges`, `PrivateNetwork=yes`,
     `RestrictAddressFamilies=AF_UNIX`, `ProtectSystem=strict`, `ProtectHome=read-only` with `ReadWritePaths` for the
     config and state directories, and `SystemCallFilter=@system-service`. This fits the fork's existing cargo-deny ban
     on network crates. Restarts and logs come from systemd and journald. Peer checking: `Gio.Socket.get_credentials()`
     gives the peer's pid, and either side can check `/proc/<pid>/exe`. That is defence in depth only: a process with
     the same uid can already edit the extension's files.
  3. *The D-Bus session bus.* This is **not recommended**: any unsandboxed process in the session can call it, which
     puts window titles and window control on the bus. GNOME itself fenced exactly these off (C12). Pop Shell
     (`com.System76.PopShell` with `WindowList` and `WindowFocus`, `src/dbus_service.ts`) and Window Calls (e.g.o 4724)
     show that people do it. They are not the bar Chained's audit sets.
- **Pros:**
  - Unmodified vase-core, and the same config, keymap, state format and 177 tests as macOS and Windows.
  - Upstream merges flow through untouched.
  - The extension stays thin (an estimated 2 to 3k lines of JS, about one backend's worth, since `vase-macos` plus
    `vase-windows` come to 5.5k lines of Rust), so a full review at each pin move is affordable.
  - It is e.g.o-compatible if ever wanted, because the binary is installed separately, as with pop-launcher.
- **Cons:**
  - It breaks CONTEXT.md's "single daemon owns the hook" assumption, so it needs a vase ADR.
  - There are now a protocol and a mirror to build and keep consistent.
  - Two artifacts to install, version-match and audit.
  - A Rust build for Ubuntu: either the fork's CI builds a Linux binary with an attestation (the same pattern as the
    ThinkPad's Windows asset, spec 105/507), or the Ubuntu Upgrade Machine builds at the reviewed commit with a pinned
    toolchain.
  - The prefix key needs a schema or `grab_accelerator`.
- **Effort:** an estimated 4 to 6 weeks: extension backend and chrome, protocol, `vase-gnome` proxy crate, unit and
  packaging.
- **Risk:** medium. Most of it sits in GNOME behaviour (async resizes, grabs), which every option shares. The protocol
  and mirror consistency is new risk that can be tested in Rust (below).

### (c) vase-core compiled to WebAssembly and loaded inside GJS

- **Feasibility:** the engine supports it (C9: confirmed in standalone gjs 1.88, likely in the shell, not checked
  there). The extension reads the `.wasm` with `Gio.File.load_contents` and calls `new WebAssembly.Module(bytes)`
  (synchronous) or `WebAssembly.instantiate`. **A wasm call into a JS import is synchronous**, so `Backend` and
  `Painter`, `measure` included, map straight onto imported JS functions. No mirror or protocol is needed. Without
  wasm-bindgen, a hand-written `extern "C"` ABI exchanging JSON through linear memory adds no new crates or CLI tool to
  audit.
- **Core changes needed (it is *not* unmodified):**
  - The 17 `Instant::now()` sites panic on `wasm32-unknown-unknown` (C7), so a clock seam is needed: pass `now` in, or
    add a `Backend::now()`.
  - `Config::load(&Path)` (`config.rs:39`) and `state::load/save` (`state.rs:31-44`) use `std::fs`, which always
    errors there. Byte or str entry points are needed, and the JS does the file I/O.
  - These are small, upstreamable seams, but until upstream takes them they are fork patches on every merge.
  - The alternative target `wasm32-wasip1` would need a WASI shim written in GJS, since there is none built in.
- **Pros:**
  - One process and synchronous calls: the existing seams are used exactly as designed.
  - The same Rust logic and tests as the other platforms.
  - No daemon, socket or unit.
  - Wasm's capability confinement: the module can do nothing but call the imports it is given.
  - The core plus the glue can be unit-tested under **standalone `gjs`** with a fake Meta, because C9 works there.
- **Cons:**
  - **No crash isolation for hangs.** A loop in the core freezes the compositor (C15).
  - A Rust panic becomes a wasm trap, which surfaces as a JS `RuntimeError`. That can be recovered by dropping the
    instance and re-instantiating from the saved state, but the instance's memory has to be treated as poisoned.
  - SpiderMonkey's wasm runtime in mozjs 140 is a large surface that gnome-shell has never exercised, and the
    compositor process would carry its memory reservations and fault-handling paths. That risk is unknown, not
    measured.
  - The `.wasm` is a binary, so it is **not e.g.o-uploadable** (C10).
  - Its review is the Rust source plus a build-reproducibility check, not reading the file.
  - The model is rebuilt on every `enable()` (every unlock) unless `unlock-dialog` mode is used (C11).
  - The 25 ms tick and 100 ms reconcile run on the compositor's main thread, so the cost must stay well under a frame.
- **Effort:** an estimated 3 to 5 weeks: extension backend and chrome, wasm ABI glue (about 0.8k lines across both
  sides), core seams.
- **Risk:** medium-high and front-loaded. Wasm inside the live shell is unverified, the stability of mozjs wasm inside
  mutter is unknown, and the core patches have to be accepted upstream.

### (d) Other options considered

- **(d1) A native Rust library loaded into gnome-shell through GObject Introspection (cdylib plus typelib).** It gives
  synchronous calls with no wasm, but a Rust panic across the FFI aborts the compositor, which ends the session. e.g.o
  forbids bundled libraries, and the GObject bindings bring sys crates that the fork's cargo-deny bans. **Rejected.**
- **(d2) A Rust daemon plus an off-the-shelf D-Bus window extension (Window Calls or Window Calls Extended).** It cannot
  draw chrome (C3) or capture modal keys, it puts window control on the session bus (see (b)3), and it adds a
  third-party extension to audit. **Rejected.** It is useful prior art for what the protocol's method set looks like.
- **(d3) An external app using xdg-desktop-portal GlobalShortcuts (GNOME 48+) for the prefix.** It gets global chords
  but cannot move windows or grab the next key modally, so the extension is still needed. **Insufficient on its own.**
- **(d4) Leaving GNOME for a Rust Wayland compositor (niri, or vase on a wlroots compositor).** It removes the
  extension problem but goes against the desktop and the spec's assumptions. It is only an escape hatch if the GNOME
  prototype fails.
- **(d5) A protocol-first hybrid, the recommended way to stage (b) and (c).** The extension is written against one
  event and effect message boundary. Extension → core: window deltas, display changes, keys, clicks, ticks, widths.
  Core → extension: a batch of effects, a draw list, the modal flag and grab requests. An in-process wasm core answers
  "immediately" and a daemon answers "over a socket", but the extension code is the same, so (b) and (c) become
  deployment choices behind a single seam. The cost is that (c) loses its "no mirror needed" advantage, because the
  daemon-shaped protocol is used even in-process.

## 2. Comparison

| | (a) JS port | (b) extension + daemon | (c) wasm in GJS |
|---|---|---|---|
| Prefix and key latency | in-process | prefix in-process; command +1 local round trip (≪ 1 frame, estimated) | in-process |
| Daemon or core crash | session-wide (hang); exception survives | **isolated**; kill chord always works | hang freezes the session; panic is a recoverable trap |
| Shares behaviour, keymap and config with macOS and Windows | no (a port) | **yes, unmodified core** | yes, with clock and fs seams |
| Survives lock (`disable()`) | rebuild from state | daemon persists (systemd) or rebuild (spawned) | rebuild from state |
| Testability | new JS tests | Rust tests as-is, plus protocol tests with a fake extension | Rust tests, plus glue tests under standalone gjs |
| e.g.o rules | compatible | compatible (binary installed separately) | **not** compatible (.wasm) |
| What Chained audits | about 8k+ lines of JS logic at each pin | about 2-3k lines of JS + daemon via the fork's existing cargo-deny, audit and CI; systemd sandbox | about 2-3k lines of JS + Rust source + reproducible .wasm |
| GPL | GPL-3.0 (compatible with the shell) | GPL-3.0 on both sides | GPL-3.0 |
| Effort (estimated) | 6-10 weeks + porting tax | 4-6 weeks | 3-5 weeks + upstreaming seams |
| Main risk | drift and maintenance | protocol and mirror consistency | wasm inside the compositor, unverified |

## 3. How Chained would install and audit it (any option, details for (b))

- **The extension:** the reviewed commit's `gnome-extension/` directory is copied (never symlinked) to
  `~/.local/share/gnome-shell/extensions/<uuid>/`. Its `metadata.json` lists `"shell-version": ["50"]`. The uuid is
  added to `org.gnome.shell enabled-extensions` and `tiling-assistant@ubuntu.com` is disabled in the same step, because
  two tilers must never both move windows (spec 107/653: Tiling Assistant stays until the Linux Tiler passes its
  assessment, so this is a Profile switch). `allow-extension-installation=false` stays as it is (C13). A new extension
  only loads after a log-out on Wayland, which reuses Bootstrap's existing guided GNOME log-out step.
- **The daemon:** either built on the Ubuntu Upgrade Machine at the reviewed SHA (locked and offline, as for the Mac),
  or produced by the fork's CI as an attested Linux asset, verified the ThinkPad way (bundle in the repo plus hash in
  the bootstrap lock). Installed to a Chained-owned path, with a `systemd --user` unit that Chained renders, including
  the sandbox directives above.
- **Doctor:** the extension's state is ACTIVE (`gnome-extensions info <uuid>`), not ERROR or OUT OF DATE; the
  extension files match the reviewed commit; the daemon's hash matches the lock; the unit is active; Tiling Assistant
  is disabled whenever the Tiler is enabled.
- **Fork CI:** add a Linux lint and test job (it currently covers macOS and Windows), eslint plus a
  readable-JS check on the extension, and the existing tripwires (process spawning: the extension's spawn or
  `systemctl` call becomes an allow-listed one).
- **Review surface:** the extension JS is reviewed in full at each pin move, because it runs inside the compositor
  with the session's full power. The daemon gets the Rust conditions the spec already has for vase.

## 4. Recommendation

**Target (b), built protocol-first (d5), with transport 2 (a systemd user unit and a private unix socket).**

- (b) is the only option that meets the crash-isolation bar ("a daemon crash must not take the session down").
- It keeps vase-core unmodified, so behaviour, keymap and config stay one implementation across the three OSes.
- It keeps the in-compositor code small enough to review in full.
- Latency is not a real cost: the prefix arms in-process, and the grab removes the consume-or-pass race.

Keep (c) as the documented fallback behind the same boundary, in case the mirror proves painful. Its
in-shell `typeof WebAssembly` check takes one minute in Looking Glass.

Reject (a), because it is a second product, and (d1) to (d4).

**Order for the prototype, which the wayfinder can take as its first spike.** Most of the risk is in the GNOME half,
which every option shares, so de-risk that first:

1. An extension skeleton with a stub JS "core" that does no tiling. It checks four things:
   - the Alt+A `grab_accelerator`, then `pushModal`, then the next key, then `popModal`, with the kill chord;
   - `move_resize_frame` on a GTK4, an Electron, a Ghostty and an XWayland window, timing the configure lag;
   - St chrome drawn above windows and below the shell's own popups, with click routing;
   - behaviour across lock and unlock (`disable`/`enable`).
2. The protocol and the `vase-gnome` proxy crate, driving the real `Daemon` from the skeleton.
3. An ADR in the fork recording the Linux process split, which amends CONTEXT.md's "single daemon owns the hook".

## Sources

- Chained: `docs/adr/0001-tiler-is-a-fork-of-vase.md`; `.scratch/chained-v1/spec.md:171-176,638-654,865-880,1039,1058`;
  `docs/keymap.md:21-28`.
- vase-chained @022340f: `CONTEXT.md:45-66`; `vase-core/Cargo.toml`; `vase-core/src/backend.rs:7-80`;
  `chrome/paint.rs:47-80`; `chrome/bar.rs:72-125`; `chrome/deck.rs:170,207`; `input/mod.rs:138-185`;
  `input/keymap.rs`; `daemon/mod.rs:38,78,191`; `daemon/input.rs:12-60`; `daemon/effects.rs:13-121`;
  `daemon/reconcile.rs:128`; `config.rs:39-81`; `state.rs:31-44`; `vase-windows/src/hook.rs:1-140`;
  `vase-windows/src/lib.rs:28-37`; `vase-windows/src/bin/vase.rs:80-260`; `vase-windows/src/chrome/mod.rs:87`;
  `vase-macos/src/event_tap.rs:104`.
- gnome-shell 50.1 (https://gitlab.gnome.org/GNOME/gnome-shell/-/tree/50.1): `js/ui/main.js:697-760`;
  `js/ui/windowManager.js:1078-1092`; `js/ui/shellDBus.js:27-31,70-71,204-230,322`; `js/misc/introspect.js:7-10,51`;
  `js/misc/util.js:344+`; `js/ui/extensionDownloader.js:32-38`.
- mutter 50.1 (https://gitlab.gnome.org/GNOME/mutter/-/blob/50.1/src/meta/window.h): lines 163, 210, 295, 301, 321,
  325, 422. Layer-shell: https://gitlab.gnome.org/GNOME/mutter/-/issues/973;
  https://github.com/brndnmtthws/conky/wiki/Configuration.
- gjs 1.88.0 (https://gitlab.gnome.org/GNOME/gjs/-/blob/1.88.0/gjs/engine.cpp): lines 242-257; NEWS.
- gjs.guide: https://gjs.guide/extensions/review-guidelines/review-guidelines.html;
  https://gjs.guide/extensions/topics/session-modes.html; https://gjs.guide/extensions/overview/architecture.html.
- Rust: https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html;
  https://github.com/rust-lang/rust/blob/master/library/std/src/sys/time/unsupported.rs (lines 12-13).
- GNOME 50 Wayland-only: https://www.phoronix.com/news/GNOME-Mutter-Shell-50-Alpha;
  https://9to5linux.com/gnome-50-tokyo-desktop-environment-officially-released-this-is-whats-new.
- Prior art: Pop Shell https://github.com/pop-os/shell (branch master_noble: `src/utils.ts:91-136`,
  `src/launcher.ts:345-347`, `src/launcher_service.ts`, `src/dbus_service.ts`); Window Calls
  https://extensions.gnome.org/extension/4724/window-calls/ and https://github.com/ickyicky/window-calls; Ubuntu
  Tiling Assistant (local, `/usr/share/gnome-shell/extensions/tiling-assistant@ubuntu.com`,
  `src/extension/layoutsManager.js:61-71`).
- Local check (read-only, standalone gjs, not the shell): `gjs -c` →
  `typeof WebAssembly` = `object`; module `add(2,3)` = 5.
