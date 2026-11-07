# vase-gnome

The Linux Tiler: vase on GNOME Shell 50 (Wayland), as a thin GNOME Shell extension plus a Rust daemon that runs
vase-core (`docs/linux-tiler/wayfinder.md` §3). This directory holds the daemon (M1a) and M0a's extension skeleton:

- `src/`: the daemon, `vase-gnome`, described below.
- `systemd/vase-gnome.service.in`: its `systemd --user` unit, as a template for Chained to render.
- `extension/`: the hello extension (`vase-hello@maartenhaine.github.io`). It logs `enable` and `disable` and draws one
  St label on the primary screen. ESM, `shell-version` `["50"]`, `session-modes` `["user", "tiler-dev"]` (`tiler-dev`
  is the L3 dev shell's mode), no dependencies.
- `tests/l1/`: the L1 level. `run.sh` packs the extension and runs `gnome-shell-test-tool --headless --extension` on
  `hello.js` under `dbus-run-session` and a mocked system bus (`dbusrunner.py`). `hello.js` runs the enable, disable,
  enable cycle and checks with `leakcheck.js` that the disable left no actor, signal, source, grab or held object.

How to run it, from the repo root (the host needs podman; no Rust, no GNOME test packages):

| What | Command |
| --- | --- |
| Build the Linux workspace | `just build-linux` (`tools/linux/podman.sh build`) |
| Its tests | `just test-linux` (`tools/linux/podman.sh test`) |
| The L1 hello test | `just l1` (`tools/linux/podman.sh l1`) |
| cargo-deny and cargo audit | `tools/linux/cargo-tools.sh DIR`, then `tools/linux/podman.sh supply DIR` |
| Metadata lint, tripwires, eslint | `tools/linux/podman.sh lint NODE_DIR` (CI runs `tools/linux/lint.sh` directly) |
| The L3 dev shell (a devkit window on the desktop, private buses) | `sh tools/dev-shell.sh` (`just dev-shell`) |
| Check the desktop wasn't touched | `sh tools/before-after.sh snapshot A`, run things, `snapshot B`, `compare A B` (A and B are new files) |

The L1 gnome-shell only ever starts inside the `chained-tiler-test` container: `run.sh` and `dbusrunner.py` refuse to
run anywhere else, and the container gets no network and no host bus, runtime dir or Wayland socket. The only
gnome-shell that runs on the desktop is the L3 dev shell, on private buses and a private runtime dir
(`tools/dev-shell.sh` lists its isolation; spike S11, `docs/linux-spike/S11.md`, is the finding that it runs).

## The daemon

`vase-gnome` runs vase-core's `Daemon` for the extension, over protocol v1 on a unix socket it binds itself
(`docs/linux-tiler/protocol-v1.md`). Linux only: on macOS and Windows the crate builds to nothing.

| File | What it does |
| --- | --- |
| `src/protocol.rs` | the wire: `v` on every line, the 1 MiB line cap (read in 64 KiB chunks), the messages |
| `src/mirror.rs` | the optimistic mirror of the compositor's windows: effects show at once, deltas older than an effect never undo it |
| `src/backend.rs` | `GnomeBackend`: vase-core's `Backend` over the mirror; every change becomes an effect |
| `src/painter.rs` | `GnomePainter`: the chrome as draw lists, measured from the extension's Pango advance arrays |
| `src/host.rs` | the lifecycle (Starting, Connected, Disconnected, Stopping, Quitting) and each message turned into vase calls |
| `src/server.rs` | the lock before the bind, one connection, the peer check, the write queue, SIGTERM, the 25 ms tick |
| `src/confine.rs` | Landlock: read the config, read and write the state and runtime dirs, exec nothing |

Its files: `$XDG_CONFIG_HOME/vase/config.toml` (read only), `$XDG_STATE_HOME/vase/state.json`, and
`$XDG_RUNTIME_DIR/vase/gnome.sock` with its `gnome.lock`. A second daemon finds the lock held and exits without
touching the socket. SIGTERM saves the layout and lets go of the screen without moving a window; only `q`, `:quit` and
the kill chord put windows back where they were.

How it is checked, all in `chained-tiler-build` (`just test-linux`, `just check`, `podman.sh supply`, `podman.sh lint`):

- `tests/unit/`: the host driven line by line with no socket (a prefix command, replays, sticky, a pending launch,
  freezing and thawing, a new compositor instance, SIGTERM, the kill chord, the errors), and the mirror's property
  test.
- `tests/l0.rs` with the fake extension in `tests/fake/`: the real binary over its real socket (the handshake, a v2
  line, a line over 1 MiB, a busy daemon, the lock, a restart, freezing, a re-login, SIGTERM).
- `tests/landlock.rs`: confined, a file outside its dirs, an `execve` and a TCP bind all fail.
- `tools/linux/lint-unit.py`: the unit carries exactly S10's measured options and no namespace or capability option.

A new crate goes into `Cargo.lock` with `tools/linux/podman.sh lock > Cargo.lock.new`, reviewed before it replaces
the lockfile; every other cargo step is locked and offline.
