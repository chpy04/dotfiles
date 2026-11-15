#!/usr/bin/env python3
"""Minimal Herdr socket client. Raw NDJSON over the unix socket, one request per connection.
Usage: herdr_rpc.py <method> [params-json]   -> prints the full response JSON line.
Importable: from herdr_rpc import call"""
import json, os, socket, sys, itertools
_ids = itertools.count(1)
def sock_path():
    return os.environ.get("HERDR_SOCKET_PATH") or os.path.expanduser("~/.config/herdr/herdr.sock")
def call(method, params=None, timeout=30):
    req = {"id": "req_%d_%d" % (os.getpid(), next(_ids)), "method": method, "params": params or {}}
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(sock_path())
    s.sendall((json.dumps(req) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        c = s.recv(65536)
        if not c: break
        buf += c
    s.close()
    return json.loads(buf.split(b"\n")[0])
if __name__ == "__main__":
    r = call(sys.argv[1], json.loads(sys.argv[2]) if len(sys.argv) > 2 else {})
    print(json.dumps(r))
    sys.exit(0 if "error" not in r else 1)
