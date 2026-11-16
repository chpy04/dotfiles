#!/usr/bin/env python3
"""Helpers for scripts/test-phase1.sh (stdlib only).
  expect  <expected.bin> <repo> <relpath> <start> <end> <text>   append the exact bytes Herdr should type
  wait    <fake-log> <since> <n> [timeout]                       wait for n submits since <since>
  compare <fake-log> <since> <expected.bin>                      byte-exact comparison of what arrived
  status  <pane-id>                                              print agent_status
  report  <pane-id> <state>                                      pane.report_agent as the fake agent does
"""
import json, os, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "spike"))
from herdr_rpc import call

PASTE_START, PASTE_END = b"\x1b[200~", b"\x1b[201~"


def expected_payload(repo, rel, start, end, text):
    lines = open(os.path.join(repo, rel), encoding="utf-8").read().split("\n")
    shown = lines[start - 1:end]
    width = len(str(end))
    code = "\n".join("  %*d │ %s" % (width, start + i, l.rstrip()) for i, l in enumerate(shown))
    rng = str(start) if end == start else "%d-%d" % (start, end)
    head = "[comment] %s:%s  (worktree: %s)" % (rel, rng, os.path.basename(repo))
    return "\n".join([head, code, text.rstrip()])


def events(log, since):
    out = []
    for l in open(log):
        try:
            e = json.loads(l)
        except ValueError:
            continue
        if e["t"] >= since:
            out.append(e)
    return out


def main():
    cmd = sys.argv[1]
    if cmd == "expect":
        _, _, outf, repo, rel, start, end, text = sys.argv
        p = expected_payload(repo, rel, int(start), int(end), text)
        with open(outf, "ab") as f:
            f.write(PASTE_START + p.encode() + PASTE_END + b"\r")
    elif cmd == "wait":
        log, since, n = sys.argv[2], float(sys.argv[3]), int(sys.argv[4])
        timeout = float(sys.argv[5]) if len(sys.argv) > 5 else 20
        t0 = time.time()
        while time.time() - t0 < timeout:
            if sum(1 for e in events(log, since) if e.get("event") == "submit") >= n:
                return
            time.sleep(0.1)
        print("timeout waiting for %d submits (got %d)" % (n, sum(1 for e in events(log, since) if e.get("event") == "submit")))
        sys.exit(1)
    elif cmd == "compare":
        log, since, expf = sys.argv[2], float(sys.argv[3]), sys.argv[4]
        got = b"".join(bytes.fromhex(e["hex"]) for e in events(log, since) if "hex" in e)
        want = open(expf, "rb").read()
        if got != want:
            print("MISMATCH\n  want: %r\n  got:  %r" % (want, got))
            sys.exit(1)
        print("byte-exact: %d bytes, %d submits" % (len(got), got.count(b"\r")))
    elif cmd == "status":
        print(call("agent.get", {"target": sys.argv[2]}).get("result", {}).get("agent", {}).get("agent_status", "?"))
    elif cmd == "report":
        r = call("pane.report_agent", {"pane_id": sys.argv[2], "source": "tether-fake", "agent": "claude", "state": sys.argv[3]})
        sys.exit(0 if "error" not in r else 1)
    else:
        sys.exit("unknown command " + cmd)


main()
