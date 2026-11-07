# vase-gnome

The Linux Tiler: vase on GNOME Shell 50 (Wayland), as a thin GNOME Shell extension plus a Rust daemon that runs
vase-core (`docs/linux-tiler/wayfinder.md` §3). This directory holds M0a's skeleton only:

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
