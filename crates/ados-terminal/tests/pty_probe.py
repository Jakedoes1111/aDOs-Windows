#!/usr/bin/env python3
"""Interactive smoke fixture: capture terminal input without executing pasted text.

Run in the terminal: python3 .../pty_probe.py /tmp/ados-test-terminal3/input.json
Then send Ctrl+A, Alt+X, Up, Ctrl+Right, Shift+Tab, and paste `PASTE\nSECOND`.
The probe exits after 4 seconds without input (30 seconds maximum).
"""
import json
import os
import select
import sys
import termios
import time
import tty

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
data = bytearray()
try:
    tty.setraw(fd)
    os.write(sys.stdout.fileno(), b"\x1b[?2004hRAW INPUT READY\r\n")
    end = time.monotonic() + 30
    while time.monotonic() < end:
        ready, _, _ = select.select([fd], [], [], 4 if data else 30)
        if not ready:
            break
        data.extend(os.read(fd, 4096))
finally:
    os.write(sys.stdout.fileno(), b"\x1b[?2004l\r\n")
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
with open(sys.argv[1], "w", encoding="utf-8") as target:
    json.dump({"hex": data.hex(), "bytes": list(data)}, target)
print("INPUT CAPTURED:", data.hex())
