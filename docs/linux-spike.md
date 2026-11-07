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
