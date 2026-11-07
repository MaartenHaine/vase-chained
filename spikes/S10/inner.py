# S10 inner probe (throwaway). Run as `python3 -I -c "$(cat inner.py)"` inside a transient systemd --user unit,
# so it needs no file under $HOME (ProtectHome= could hide one). Prints one "key=value" line per measurement.
# Every check catches its own error; a syscall that a filter denies reports its errno instead of killing the run
# (the driver sets SystemCallErrorNumber=EBADE where a SystemCallFilter is under test, so a filter denial is the
# distinctive errno 52 "EBADE", not the kernel's own EPERM).
import ctypes
import errno
import mmap
import os
import socket
import sys
import threading

libc = ctypes.CDLL(None, use_errno=True)
out = []


def put(k, v):
    out.append(f"{k}={v}")


def err(e):
    n = e.errno if isinstance(e, OSError) else None
    return f"ERR:{errno.errorcode.get(n, n)}" if n is not None else f"ERR:{type(e).__name__}"


def call(k, f):
    try:
        r = f()
        put(k, "ok" if r is None else r)
    except Exception as e:  # noqa: BLE001 - a probe reports every failure
        put(k, err(e))


def cerr(rc):
    if rc == -1:
        raise OSError(ctypes.get_errno(), os.strerror(ctypes.get_errno()))
    return rc


# --- live confinement, as doctor would read it
status = {}
with open("/proc/self/status") as f:
    for line in f:
        k, _, v = line.partition(":")
        status[k] = v.strip()
for k in ("NoNewPrivs", "Seccomp", "Seccomp_filters", "CapEff", "CapBnd"):
    put(k, status.get(k, "absent"))
call("apparmor", lambda: open("/proc/self/attr/current").read().strip())
PR_GET_MDWE = 66
call("mdwe_prctl", lambda: cerr(libc.prctl(PR_GET_MDWE, 0, 0, 0, 0)))

# --- namespaces, against the reference the driver passes in (its own, the host's)
ref = dict(x.split("=", 1) for x in os.environ.get("S10_REF_NS", "").split(",") if "=" in x)
for ns in ("user", "mnt", "net", "uts", "ipc", "pid", "cgroup"):
    try:
        mine = os.readlink(f"/proc/self/ns/{ns}")
        put(f"ns.{ns}", "host" if ref.get(ns) == mine else f"NEW({mine})")
    except Exception as e:  # noqa: BLE001
        put(f"ns.{ns}", err(e))
call("uid_map", lambda: " ".join(open("/proc/self/uid_map").read().split()))

# --- file-system namespacing effects
home = os.environ.get("HOME", "")
probe = os.environ.get("S10_PROBE_DIR", "")
if not (os.path.isabs(probe) and os.path.basename(probe).startswith("s10-probe.")):
    sys.exit("inner.py: S10_PROBE_DIR must be the driver's ~/.cache/s10-probe.* dir")
call("home.listable", lambda: f"{len(os.listdir(home))} entries")
call("probe_dir.write", lambda: open(os.path.join(probe, "w.txt"), "w").close())
call("dev.entries", lambda: len(os.listdir("/dev")))
call("tmp.inode", lambda: "%d:%d" % (os.stat("/tmp").st_dev, os.stat("/tmp").st_ino))


def mountopt(path):
    best = None
    with open("/proc/self/mountinfo") as f:
        for line in f:
            p = line.split()
            if p[4] == path:
                best = p[5]
    return best or "no-own-mount"


for p in ("/proc/sys", "/sys/fs/cgroup", "/usr", "/home", "/proc", "/tmp"):
    call(f"mount{p}", lambda p=p: mountopt(p))
call("proc.other_pids_visible", lambda: sum(1 for d in os.listdir("/proc") if d.isdigit()) > 5)
call("net.ifaces", lambda: ",".join(sorted(os.listdir("/sys/class/net"))))
call("kmsg.read", lambda: open("/dev/kmsg").close())
call("modules_dir", lambda: len(os.listdir("/usr/lib/modules")))

# --- seccomp and prctl effects
call("sock.unix", lambda: socket.socket(socket.AF_UNIX, socket.SOCK_STREAM).close())
call("sock.inet", lambda: socket.socket(socket.AF_INET, socket.SOCK_STREAM).close())
call("sock.netlink", lambda: socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 0).close())


def unix_bind():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    path = os.path.join(probe, "s.sock")
    try:
        s.bind(path)
        s.listen(1)
    finally:
        s.close()
        try:
            os.unlink(path)
        except OSError:
            pass


call("sock.unix_bind_listen", unix_bind)
call("mmap.wx", lambda: mmap.mmap(-1, 4096, prot=mmap.PROT_READ | mmap.PROT_WRITE | mmap.PROT_EXEC).close())


def mprotect_x():
    libc.mmap.restype = ctypes.c_void_p
    libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
    addr = libc.mmap(None, 4096, mmap.PROT_READ | mmap.PROT_WRITE, mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS, -1, 0)
    if addr in (None, ctypes.c_void_p(-1).value):
        raise OSError(ctypes.get_errno(), "mmap")
    libc.mprotect.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
    cerr(libc.mprotect(addr, 4096, mmap.PROT_READ | mmap.PROT_EXEC))


call("mprotect.rw_to_rx", mprotect_x)
PER_LINUX32 = 0x0008


def personality():
    old = cerr(libc.personality(0xFFFFFFFF))
    cerr(libc.personality(PER_LINUX32))
    libc.personality(old)


call("personality.linux32", personality)
call("sched.fifo_prio0", lambda: os.sched_setscheduler(0, os.SCHED_FIFO, os.sched_param(0)))
call("sched.fifo_prio0.baseline_note", lambda: "kernel answers EINVAL for prio 0; a seccomp refusal answers EPERM")


def suid():
    p = os.path.join(probe, "suid.bin")
    open(p, "w").close()
    try:
        os.chmod(p, 0o4755)
    finally:
        os.unlink(p)


call("chmod.setuid_own_file", suid)
PTRACE_PEEKDATA = 2
libc.ptrace.restype = ctypes.c_long
call("ptrace.peekdata_self", lambda: cerr(libc.ptrace(PTRACE_PEEKDATA, os.getpid(), None, None)))
call("sethostname", lambda: cerr(libc.sethostname(b"x", 1)))
call("syslog.read_all", lambda: cerr(libc.klogctl(10, None, 0)))  # SYSLOG_ACTION_SIZE_BUFFER


def in_child(f):
    # unshare and exec run in a forked child, so the probe itself keeps its namespaces.
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(r)
        try:
            res = f()
            msg = "ok" if res is None else str(res)
        except Exception as e:  # noqa: BLE001
            msg = err(e)
        os.write(w, msg.encode())
        os._exit(0)
    os.close(w)
    data = os.read(r, 200).decode()
    os.waitpid(pid, 0)
    return data or "child-died"


call("unshare.user", lambda: in_child(lambda: os.unshare(os.CLONE_NEWUSER)))
call("unshare.uts", lambda: in_child(lambda: os.unshare(os.CLONE_NEWUTS)))
call("unshare.net", lambda: in_child(lambda: os.unshare(os.CLONE_NEWNET)))


def spawn():
    pid = os.posix_spawn("/usr/bin/true", ["true"], {})
    return f"exit={os.waitpid(pid, 0)[1]}"


call("spawn.true", spawn)


def thread():
    t = threading.Thread(target=lambda: None)
    t.start()
    t.join()


call("thread.start", thread)
print("\n".join(out))
sys.stdout.flush()
