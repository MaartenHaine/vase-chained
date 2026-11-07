# vase-gnome

The Linux Tiler: vase on GNOME Shell 50 (Wayland), as a thin GNOME Shell extension plus a Rust daemon that runs
vase-core (`docs/linux-tiler/wayfinder.md` §3). This directory holds M0a's skeleton only:

- `extension/`: the hello extension (`vase-hello@maartenhaine.github.io`). It logs `enable` and `disable` and draws one
  St label on the primary screen. ESM, `shell-version` `["50"]`, no dependencies.
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
| Check the desktop wasn't touched | `tools/before-after.sh snapshot A`, run things, `snapshot B`, `compare A B` |

gnome-shell only ever starts inside the `chained-tiler-test` container: `run.sh` and `dbusrunner.py` refuse to run
anywhere else, and the container gets no network and no host bus, runtime dir or Wayland socket.
