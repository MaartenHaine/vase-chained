#!/usr/bin/python3 -I
# Spike S2 (throwaway): runs inside a terminal (Ghostty), puts the tty in raw mode, turns on focus reporting
# (CSI ?1004h, so the terminal sends CSI I / CSI O on focus in and out) and logs every byte it reads as JSON lines
# with CLOCK_MONOTONIC times.
import json
import os
import sys
import time
import tty

out = open(sys.argv[1], 'a', buffering=1)
fd = sys.stdin.fileno()
tty.setraw(fd)
os.write(sys.stdout.fileno(), b'\x1b[?1004hs2 bytelog\r\n')
out.write(json.dumps({'kind': 'start', 't': time.monotonic(), 'wall': time.time()}) + '\n')
while True:
    b = os.read(fd, 1024)
    if not b:
        break
    out.write(json.dumps({'kind': 'bytes', 't': time.monotonic(), 'bytes': b.decode('latin1')}) + '\n')
    os.write(sys.stdout.fileno(), repr(b).encode() + b'\r\n')
