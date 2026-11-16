#!/usr/bin/env python3
"""Fake agent: registers with Herdr via pane.report_agent, logs every received byte.
env: FAKE_LOG (file, required), FAKE_WORK_SECS (default 1.5; seconds spent 'working' after a submit),
     FAKE_BPASTE (1=enable bracketed paste mode like Claude Code, default 1)
Log lines (JSONL): {"t":..., "hex":"...", "repr":"..."} per read() chunk; plus {"event":"submit","text":...}
 whenever a bare CR (outside a paste) arrives. State switches also via external pane.report_agent
 with --source tether-fake (touch FAKE_LOG.state to be written by tests is not needed)."""
import os, sys, time, json, tty, termios, select
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from herdr_rpc import call
pane = os.environ["HERDR_PANE_ID"]; log = open(os.environ["FAKE_LOG"], "a", buffering=1)
work = float(os.environ.get("FAKE_WORK_SECS", "1.5"))
def report(state):
    call("pane.report_agent", {"pane_id": pane, "source": "tether-fake", "agent": "claude", "state": state})
def w(o): o["t"] = round(time.time(), 3); log.write(json.dumps(o) + "\n")
fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); tty.setraw(fd)
if os.environ.get("FAKE_BPASTE", "1") == "1": sys.stdout.write("\x1b[?2004h")
sys.stdout.write("fake-agent ready\r\n"); sys.stdout.flush()
report("idle"); w({"event": "ready"})
buf = b""; inpaste = False; cur = ""; work_until = 0
try:
    while True:
        r, _, _ = select.select([fd], [], [], 0.1)
        if work_until and time.time() > work_until:
            work_until = 0; report("idle"); w({"event": "idle"})
        if not r: continue
        d = os.read(fd, 65536)
        if not d: break
        w({"hex": d.hex(), "repr": d.decode("utf8", "replace")})
        buf += d
        while buf:
            if buf.startswith(b"\x1b[200~"): inpaste = True; buf = buf[6:]; w({"event": "paste_start"}); continue
            if buf.startswith(b"\x1b[201~"): inpaste = False; buf = buf[6:]; w({"event": "paste_end"}); continue
            if b"\x1b[200~"[:len(buf)] == buf or b"\x1b[201~"[:len(buf)] == buf: break  # partial marker
            b = buf[:1]; buf = buf[1:]
            if b == b"\r" and not inpaste:
                w({"event": "submit", "text": cur}); cur = ""
                if work: report("working"); work_until = time.time() + work
            elif b == b"\x03": raise KeyboardInterrupt
            else: cur += b.decode("latin1") if b < b"\x80" else ""  # ascii-only text accumulation (good enough)
            if b == b"\x03": pass
except KeyboardInterrupt: pass
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
    sys.stdout.write("\x1b[?2004l"); sys.stdout.flush()
    call("pane.release_agent", {"pane_id": pane, "source": "tether-fake", "agent": "claude"})
