# Linux Tiler spike findings

One finding per spike (docs/linux-tiler/wayfinder.md §5, "M0b"). Each feeds round 2 and the engineering defaults.

## S11: does the devkit run under the L3 recipe? (M0a, 2026-11-07)

**Result: not answered yet.** The recipe refused to start, at its own check, before gnome-shell ran. The devkit
never started and no viewer window opened, so S11's question (does the devkit run on private, empty buses, and does
the viewer reach the host compositor through an absolute `WAYLAND_DISPLAY`) is still open.

**Host:** Ubuntu, GNOME Shell 50.1-0ubuntu1.5 Wayland session, libmutter-18-0 and mutter-dev-bin 50.1-0ubuntu2.5.
The host was under heavy load (load average about 64).

**What happened** (`tools/dev-shell.sh`, no arguments; run dir `~/.cache/tiler-dev.TfqNNeFl`):

1. Stage A (host checks, run dir, mode file, keyfile) and stage B (the private system bus) passed.
2. In stage C, the check that the `tiler-dev` mode loads runs `/usr/bin/gnome-shell --list-modes`. That process
   crashed with SIGSEGV, so its output didn't contain `tiler-dev`, and the recipe stopped:
   `refusing to start: gnome-shell --list-modes doesn't list tiler-dev; it would fall back to the 'user' mode`.
3. The cleanup trap ran. No process with the run's `TILER_DEV_RUN` marker was left, and no `mutter-devkit` ran.

**Why `--list-modes` crashes.** From the core, with gdb offline and no debug symbols: the crash is in
`gjs_context_eval_module()` ← `gjs_context_eval_module_file()` ← gnome-shell's `--list-modes` option callback ←
`g_option_context_parse()` ← `meta_context_configure()` ← `main()`. In 50.1 the option runs while the command line
is parsed, before the shell has a display or a bus. INFERENCE: this is a gnome-shell 50.1 bug in `--list-modes`, and
it would crash with any environment, not just the recipe's. To check that would mean crashing it again, which writes
another crash report (see below), so it wasn't run again.

**Before-after** (`tools/before-after.sh`, 03:32:45 and 03:34:04): **no watched item changed**, and nothing in the
unwatched list changed either. dconf hash, LockedHint (`no`), crash marker (absent), the `~/.local/share/gnome-shell`
tree hash, the `monitors.xml` mtime and hash, the owners of `org.gnome.Shell` (pid 5222), polkit, NetworkManager and
login1, the host shell's bus connections, `shell_pids` and `leftovers=0` all match.

**One change on the host that the check doesn't watch.** Apport (`kernel.core_pattern` pipes to it) wrote
`/var/crash/_usr_bin_gnome-shell.1000.crash` (8.9 MB, 03:33, `ProcCmdline: /usr/bin/gnome-shell --list-modes`).
whoopsie is inactive, so it wasn't uploaded. On the next login, Ubuntu may show it as a "system program problem".
It was left in place so the owner can decide; it can be deleted by hand. Any process the recipe starts that crashes
leaves a report like this, so `before-after.sh` should list new files in `/var/crash`.

**Small fault found along the way.** In both scripts' `/proc/*/environ` scans, `tr ... <"$e" 2>/dev/null` opens the
file before stderr is redirected, so dash prints `cannot open /proc/N/environ: Permission denied` for every process
of another user. The result is still correct, but the output is full of noise. The fix is `2>/dev/null <"$e"`, or a
`{ ...; } 2>/dev/null` around the loop.

**Decision the map asks for (keep L3, or drop it for L1 plus L4): not taken.** The finding says nothing about the
devkit itself, only about one of the recipe's checks. M0a stays at "S11 first", and nothing is built on L3 until S11
runs to the end. Options for the owner:

- **(a) Keep L3 and replace the check.** Prove that the mode loaded without `--list-modes`, for example by loading
  `sessionMode.js`'s mode file the same way the shell does, outside gnome-shell, or by stopping the run as soon as the
  shell log shows it fell back to `user` (sessionMode.js:149-152). The isolation checks don't change. This changes a
  safety check that two reviews passed, so the change needs its own review, then S11 runs again.
- **(b) Drop L3 now** and use L1 plus the owner's L4 assessment, as the map's fallback says.

Recommendation: (a). The failure is a bug in one check, not evidence against the devkit.

### S11 fix: the mode check without `--list-modes` (2026-11-07, not run yet)

Option (a) above, taken so S11 can run again; it needs its own review first. gnome-shell wasn't started for it.

**What the shell does with a mode** (gnome-shell 50.1 `ui/sessionMode.js` and `misc/fileUtils.js`, read from the
installed `libshell-18.so`): `_loadModes()` walks `gnome-shell/modes` in each `XDG_DATA_DIRS` entry in order (not
`XDG_DATA_HOME`), names a mode after its file name up to the first `.json`, keeps the first file of each name that
`JSON.parse` accepts, skips a failed one without a word, and marks every loaded mode primary. SessionMode's
constructor then uses `--mode` if that mode is loaded and primary, and `'user'` otherwise.

**Before the start, the shell's own loader** (`tools/mode-check.js`, run with plain gjs in stage C's environment):
the two files are extracted from `libshell-18.so` at run time and must match pinned sha256s (another build is
refused until they are read again); four stubs stand in for imports the loader never reads. The check proves the
mode file is the only file in any modes dir that can define `tiler-dev`, that it sits in the first data dir, that it
parses the way `_loadMode` parses it (same libgjs and libmozjs-140 as gnome-shell) with no `parentMode`, no
components and only the extension; then it runs the real `listModes()`, which prints exactly the modes the
constructor accepts, and the recipe requires `tiler-dev` among them. Tried in a scratch tree: a good file passes;
broken JSON, a `parentMode`, a second `tiler-dev.json.bak` and a wrong first data dir are each refused, and the real
loader alone drops the broken file from its list.

**After the start, signs on the private session bus**, polled every 0.2 s; the shell is killed (SIGKILL) at the
first wrong sign, or if there is no proof 90 s after the start:
- `org.gnome.keyring.SystemPrompter` has no owner. Only `user` has components, and the keyring component claims
  that name, so a fallback shows as soon as the components start, before `org.gnome.Shell` is exported.
- `org.gnome.Shell`'s `Mode` is `tiler-dev`. **This alone proves nothing about the fallback:** `Mode` returns
  `global.session_mode`, the `--mode` argument (`shellDBus.js`, `get Mode`), not the mode SessionMode settled on.
  It only proves the option arrived.
- `GetExtensionInfo(uuid)` says `enabled: true`. That field is whether the uuid is on the extension manager's enabled
  list, which with the keyfile's `disable-user-extensions=true` is the running mode's `enabledExtensions` and nothing
  else; `user` has none. It doesn't depend on the extension's own code working. This is the positive proof.

Tried against a fake shell on a private bus: the right signs pass; `Mode` `user`, `enabled: false` and an owned
keyring prompter are each refused.

**How bad a few seconds in `user` would be.** It can only happen before the kill, since the components start
inside `_initializeUI()`, before `org.gnome.Shell` exists. On these buses:
- the polkit agent and the NetworkManager secret agent register with services that aren't on the private system
  bus, and nothing can activate them, so no authentication or Wi-Fi secret is ever asked for;
- the keyring prompter claims its name on the private session bus, where the real gnome-keyring-daemon never looks;
- no ScreenShield, as in `tiler-dev`: `canLock()` asks GDM on the private system bus and gets no answer;
- the automount manager mounts, once at startup, every volume the volume monitor offers. With
  `GIO_USE_VOLUME_MONITOR=unix` and no gvfs or udisks on the buses that is fstab entries marked `user`, and this
  host's fstab has none (its NFS entries are `noauto,x-systemd.automount`);
- the rest is UI inside the viewer: the fuller panel and quick settings (whose NetworkManager, Bluetooth and power
  items find nothing on the buses, and whose sound item finds no PulseAudio socket in the private runtime dir),
  and maybe the welcome dialog; no extension runs.
Files, the host Wayland socket and PipeWire are the same in both modes. So a short fallback is a nested shell with a
fuller UI and three agents talking to nobody. The checks keep it short, and the owner's "never in a `user`-parent
mode" rule still holds for any run that gets past them.

**Crash reports.** Apport writes a report whatever the core ulimit: the `--list-modes` crash came in at `core limit
0` (`/var/log/apport.log`). A limit of exactly 1 byte is the kernel's own "skip the `core_pattern` pipe" value. Tried
with an unpackaged copy of `sleep`: at 0, apport was called and ignored it; at 1, the kernel logged
`RLIMIT_CORE is set to 1, aborting core`, apport wasn't called, and `/var/crash` didn't change. The recipe now runs
stages B and C under `prlimit --core=1` (soft and hard, so nothing inside can raise it), and stage C refuses if the
limit isn't 1. A crash still prints its backtrace to `shell.log` (`SHELL_DEBUG=backtrace-segfaults`), but leaves no
core.

**before-after.sh** now records every entry of `/var/crash` and `~/.local/share/apport` (type, size, mtime, path:
names only, since other users' reports aren't readable). A report that appears or changes fails the check; one that
goes away is listed but doesn't. Snapshots are v2, and a v1 snapshot is refused. Both scripts' `/proc/*/environ`
scans now redirect stderr before opening the file, so permission errors are silent.
