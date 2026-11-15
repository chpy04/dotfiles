#!/usr/bin/env python3
"""Delivery checks against an agent pane.
  check_delivery.py fake   <pane> <fake-log>   # fake agent started by run-all (byte-exact assertions)
  check_delivery.py claude <pane>              # real Claude Code (haiku), screen assertions
Exit 0 = pass; prints FAIL reasons otherwise."""
import sys, os, json, time, uuid
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from herdr_rpc import call

def comment(tok):
    return ("[comment] src/auth/session.ts:142  (worktree: feat-login)\n"
            "  142 │ if (token.expiresAt < Date.now()) {\n"
            "Why client clock instead of server time here? (test %s: do not investigate, reply with just OK-%s)" % (tok, tok))

def status(pane):
    try: return call("agent.get", {"target": pane})["result"]["agent"]["agent_status"]
    except (KeyError, TypeError): return None

def fail(msg): print("FAIL:", msg); sys.exit(1)
def ok(r):
    if "error" in r: fail("rpc error %s" % r["error"])
    return r["result"]

def events(log, since):
    out = []
    for l in open(log):
        try: e = json.loads(l)
        except ValueError: continue
        if e["t"] >= since: out.append(e)
    return out

def wait_for(pred, timeout=10, step=0.05):
    t = time.time()
    while time.time() - t < timeout:
        v = pred()
        if v: return v
        time.sleep(step)
    return None

def payload(evs):
    """Reassemble bytes received since marker, split into paste body and trailing bytes."""
    raw = b"".join(bytes.fromhex(e["hex"]) for e in evs if "hex" in e)
    return raw

def fake(pane, log):
    since = time.time()
    # 1. agent.prompt while idle
    tok = uuid.uuid4().hex[:6]; msg = comment(tok)
    if not wait_for(lambda: status(pane) in ("idle", "done"), 10):
        fail("fake agent not idle")
    since = time.time() - 0.01
    ok(call("agent.prompt", {"target": pane, "text": msg}))
    evs = wait_for(lambda: [e for e in events(log, since) if e.get("event") == "submit"] and events(log, since), 5)
    if not evs: fail("no submit received (agent.prompt)")
    raw = payload(evs)
    want = b"\x1b[200~" + msg.encode() + b"\x1b[201~\r"
    if raw != want: fail("agent.prompt bytes differ: %r" % raw)
    if [e["event"] for e in evs if "event" in e].count("submit") != 1: fail("more than one submit")
    # delay between paste end and CR
    tp = [e["t"] for e in evs if e.get("event") == "paste_end"][0]; tc = [e["t"] for e in evs if e.get("event") == "submit"][0]
    gap = tc - tp
    # 2. agent.prompt while working: must still deliver (agent's own queue handles it)
    wait_for(lambda: status(pane) in ("idle", "done"), 10)
    ok(call("pane.report_agent", {"pane_id": pane, "source": "tether-fake", "agent": "claude", "state": "working"}))
    if status(pane) != "working": fail("could not switch to working")
    since = time.time() - 0.01; tok2 = uuid.uuid4().hex[:6]; msg2 = comment(tok2)
    ok(call("agent.prompt", {"target": pane, "text": msg2}))
    evs = wait_for(lambda: [e for e in events(log, since) if e.get("event") == "submit"] and events(log, since), 5)
    if not evs: fail("no submit while working")
    if payload(evs) != b"\x1b[200~" + msg2.encode() + b"\x1b[201~\r": fail("working-state bytes differ: %r" % payload(evs))
    # 3. pane.send_text + pane.send_keys enter (fallback path): raw text, no paste markers
    wait_for(lambda: status(pane) in ("idle", "done"), 10)
    since = time.time() - 0.01; tok3 = uuid.uuid4().hex[:6]; msg3 = comment(tok3)
    ok(call("pane.send_text", {"pane_id": pane, "text": msg3})); ok(call("pane.send_keys", {"pane_id": pane, "keys": ["enter"]}))
    evs = wait_for(lambda: [e for e in events(log, since) if e.get("event") == "submit"] and events(log, since), 5)
    if not evs: fail("no submit (send_text path)")
    if payload(evs) != msg3.encode() + b"\r": fail("send_text bytes differ: %r" % payload(evs))
    print("PASS fake (agent.prompt paste->CR gap %.0fms)" % (gap * 1000))

def screen(pane):
    return ok(call("pane.read", {"pane_id": pane, "source": "recent_unwrapped"}))["read"]["text"]

def claude(pane):
    st = lambda: status(pane)
    if not wait_for(lambda: st() in ("idle", "done"), 60): fail("claude not idle: %s" % st())
    tok = uuid.uuid4().hex[:6]; msg = comment(tok)
    ok(call("agent.prompt", {"target": pane, "text": msg, "wait": {"timeout_ms": 90000}}))
    if not wait_for(lambda: st() in ("idle", "done") and ("OK-" + tok) in screen(pane).split("(test")[-1].replace("\n", " ") , 60):
        fail("no reply seen")
    s = screen(pane); s = s[s.rindex("\u276f [comment]"):]   # only the latest prompt (earlier runs stay in scrollback)
    # prompt echoed intact, once: header, code line with the box char, question
    for needle in ["[comment] src/auth/session.ts:142  (worktree: feat-login)", "142 │ if (token.expiresAt < Date.now()) {",
                   "Why client clock instead of server time here? (test %s" % tok]:
        if s.count(needle) != 1: fail("echo of %r count=%d" % (needle, s.count(needle)))
    if s.count("❯ [comment]") < 1: fail("not shown as a submitted prompt")
    if ("⏺ OK-" + tok) not in s: fail("reply OK-%s missing" % tok)
    print("PASS claude")

if __name__ == "__main__":
    if sys.argv[1] == "fake": fake(sys.argv[2], sys.argv[3])
    else: claude(sys.argv[2])
