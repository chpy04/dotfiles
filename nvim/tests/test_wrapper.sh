#!/usr/bin/env bash
# Tests for bin/tether-nvim (fake nvim for argv checks + the real nvim for the socket).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WRAP="$HERE/../../bin/tether-nvim"
T="$(mktemp -d "${TMPDIR:-/tmp}/tw.XXXXXX")"; T="${T%/}"
trap 'rm -rf "$T"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1 :: $2"; }
eq()  { [ "$2" = "$3" ] && ok "$1" || bad "$1" "got [$2] want [$3]"; }

# fake nvim: logs argv (one per line) to $FAKE_OUT; answers --server probes per $FAKE_LIVE
cat > "$T/fakenvim" <<'F'
#!/bin/sh
if [ "$1" = "--server" ]; then [ "${FAKE_LIVE:-0}" = 1 ] && exit 0 || exit 1; fi
printf '%s\n' "$@" > "$FAKE_OUT"
F
chmod +x "$T/fakenvim"
export TETHER_NVIM="$T/fakenvim" FAKE_OUT="$T/out"
unset XDG_RUNTIME_DIR HERDR_ENV HERDR_PANE_ID

# 1. outside Herdr: plain nvim, args passed through
TMPDIR="$T/rt/" "$WRAP" a.txt "b c.txt"
eq "outside Herdr: no --listen" "$(tr '\n' '|' < "$T/out")" "a.txt|b c.txt|"

# 2. inside Herdr: --listen <TMPDIR stripped>/tether/<pane with - for :>.sock
HERDR_ENV=1 HERDR_PANE_ID=w8:p2 TMPDIR="$T/rt/" "$WRAP" file.txt
eq "inside Herdr: --listen path" "$(tr '\n' '|' < "$T/out")" "--listen|$T/rt/tether/w8-p2.sock|file.txt|"
[ -d "$T/rt/tether" ] && ok "socket dir created" || bad "socket dir created" "missing"

# 3. XDG_RUNTIME_DIR wins over TMPDIR; HERDR_ENV!=1 means plain
HERDR_ENV=1 HERDR_PANE_ID=wC:p10 XDG_RUNTIME_DIR="$T/xdg" TMPDIR="$T/rt/" "$WRAP"
eq "XDG_RUNTIME_DIR preferred" "$(tr '\n' '|' < "$T/out")" "--listen|$T/xdg/tether/wC-p10.sock|"
HERDR_ENV=0 HERDR_PANE_ID=w8:p2 TMPDIR="$T/rt/" "$WRAP" x
eq "HERDR_ENV!=1: plain nvim" "$(tr '\n' '|' < "$T/out")" "x|"

# 4. stale socket is removed; live socket is left alone and nvim starts without --listen
python3 - "$T/rt/tether/w8-p2.sock" <<'P'
import socket, sys
s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()   # leaves a dead socket file
P
[ -S "$T/rt/tether/w8-p2.sock" ] && ok "stale socket fixture exists" || bad "stale socket fixture" "not created"
HERDR_ENV=1 HERDR_PANE_ID=w8:p2 TMPDIR="$T/rt/" FAKE_LIVE=0 "$WRAP" f
[ ! -e "$T/rt/tether/w8-p2.sock" ] && ok "stale socket removed before launch" || bad "stale socket removed" "still there"
eq "stale: still launches with --listen" "$(head -1 "$T/out")" "--listen"
python3 - "$T/rt/tether/w8-p2.sock" <<'P'
import socket, sys
s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()
P
HERDR_ENV=1 HERDR_PANE_ID=w8:p2 TMPDIR="$T/rt/" FAKE_LIVE=1 "$WRAP" g
eq "live socket: no --listen" "$(tr '\n' '|' < "$T/out")" "g|"
[ -e "$T/rt/tether/w8-p2.sock" ] && ok "live socket not deleted" || bad "live socket not deleted" "removed"
rm -f "$T/rt/tether/w8-p2.sock"

# 5. nvim.command with flags is word-split
TETHER_NVIM="$T/fakenvim --clean" HERDR_ENV=1 HERDR_PANE_ID=w1:p1 TMPDIR="$T/rt" "$WRAP" z
eq "command with flags" "$(tr '\n' '|' < "$T/out")" "--clean|--listen|$T/rt/tether/w1-p1.sock|z|"

# 6. the real nvim really listens on that path (colon->dash, no host:port parsing)
if command -v nvim >/dev/null 2>&1; then
  unset TETHER_NVIM
  PATH_NO_TETHER="$PATH"
  HERDR_ENV=1 HERDR_PANE_ID=w9:p3 TMPDIR="$T/rt/" PATH="$PATH_NO_TETHER" "$WRAP" --headless -u NONE -i NONE \
    "+call writefile([v:servername], '$T/servername')" +qa
  eq "real nvim --listen address" "$(cat "$T/servername" 2>/dev/null)" "$T/rt/tether/w9-p3.sock"
fi

echo "wrapper tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
