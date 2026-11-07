# Spike S9: socket activation, emulated the way systemd --user does it. This process binds and listens, forks, and
# the child execs the daemon with the listening socket as fd 3 (LISTEN_FDS=1). The parent stays, like the user
# manager. SO_PEERCRED on a client's connection then names this parent (wayfinder F13). Throwaway.
#
#   python3 -I activate.py SOCKET DAEMON
import os
import signal
import socket
import sys
import time

path, daemon = sys.argv[1], sys.argv[2]
os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
try:
    os.unlink(path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(path)
s.listen(16)
child = os.fork()
if child == 0:
    if s.fileno() != 3:
        os.dup2(s.fileno(), 3)
    else:
        os.set_inheritable(3, True)
    os.environ["LISTEN_FDS"] = "1"
    os.environ["LISTEN_PID"] = str(os.getpid())
    os.execv(daemon, [daemon, "--listen-fd"])
s.close()


def stop(*_):
    os.kill(child, signal.SIGKILL)
    os._exit(0)


signal.signal(signal.SIGTERM, stop)
print(f"[s9-activate] listening pid {os.getpid()}, daemon pid {child}", file=sys.stderr, flush=True)
while True:
    time.sleep(3600)
