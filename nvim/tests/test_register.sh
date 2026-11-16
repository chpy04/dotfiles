#!/usr/bin/env bash
# Tests for nvim/plugin/tether.lua socket registration (finding 1a/6), against the real nvim:
# a plain `nvim` inside Herdr listens on <runtime>/tether/<pane>.sock without bin/tether-nvim.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
PLUGIN="$(cd "$HERE/.." && pwd)"
T="$(mktemp -d "${TMPDIR:-/tmp}/tr.XXXXXX")"; T="${T%/}"
# the "live owner" nvim started below can outlive `kill $OWNER` (nvim re-execs); reap it by its socket path
trap 'pkill -f "listen $T/" 2>/dev/null; rm -rf "$T"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1 :: $2"; }
eq()  { [ "$2" = "$3" ] && ok "$1" || bad "$1" "got [$2] want [$3]"; }
command -v nvim >/dev/null 2>&1 || { echo "nvim not installed: skipped"; exit 0; }

cat > "$T/init.lua" <<L
vim.opt.rtp:prepend("$PLUGIN")
vim.opt.swapfile = false
vim.opt.shadafile = "NONE"
L
SOCK="$T/run/tether/w9-p3.sock"
# run nvim with the plugin only; extra args are +cmds. Output lines come from writefile() in the cmds.
nv() { env -u TMPDIR -u HERDR_PANE_ID -u HERDR_ENV XDG_RUNTIME_DIR="$T/run" "$@"; }
plain() { nv HERDR_ENV=1 HERDR_PANE_ID=w9:p3 nvim --headless -u "$T/init.lua" -i NONE "$@" 2>&1; }

# 1. plain nvim in a pane registers itself
plain "+call writefile([join(serverlist(), ',')], '$T/o1')" +qa
case "$(cat "$T/o1")" in *"$SOCK"*) ok "plain nvim listens on the pane socket";; *) bad "plain nvim listens" "$(cat "$T/o1")";; esac

# 2. outside Herdr (HERDR_ENV unset / 0) and without a pane id nothing is registered
rm -rf "$T/run"
nv HERDR_ENV=0 HERDR_PANE_ID=w9:p3 nvim --headless -u "$T/init.lua" -i NONE "+call writefile([join(serverlist(), ',')], '$T/o2')" +qa
case "$(cat "$T/o2")" in *tether*) bad "HERDR_ENV=0 registers nothing" "$(cat "$T/o2")";; *) ok "HERDR_ENV=0: not registered";; esac
nv HERDR_ENV=1 nvim --headless -u "$T/init.lua" -i NONE "+call writefile([join(serverlist(), ',')], '$T/o2')" +qa
case "$(cat "$T/o2")" in *tether*) bad "no pane id registers nothing" "$(cat "$T/o2")";; *) ok "no HERDR_PANE_ID: not registered";; esac
[ ! -e "$T/run/tether" ] && ok "no socket dir created outside Herdr" || bad "socket dir created" "exists"

# 3. a stale socket file (crashed nvim) is replaced
mkdir -p "$T/run/tether"
python3 - "$SOCK" <<'P'
import socket, sys
s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()
P
plain "+call writefile([join(serverlist(), ',')], '$T/o3')" +qa
case "$(cat "$T/o3")" in *"$SOCK"*) ok "stale socket replaced";; *) bad "stale socket replaced" "$(cat "$T/o3")";; esac

# 4. a vanished socket file is re-created on FocusGained / VimResume, and answers
for ev in FocusGained VimResume; do
  rm -rf "$T/run"
  plain "+lua local s='$SOCK'; os.remove(s); local before = vim.uv.fs_stat(s) ~= nil; vim.cmd('doautocmd $ev'); local c = vim.fn.sockconnect('pipe', s, {rpc=true}); local v = c > 0 and vim.fn.rpcrequest(c, 'nvim_eval', '1+1') or -1; vim.fn.writefile({tostring(before) .. ' ' .. tostring(vim.uv.fs_stat(s) ~= nil) .. ' ' .. tostring(v)}, '$T/o4')" +qa
  eq "$ev: socket removed, then re-created and answering" "$(cat "$T/o4")" "false true 2"
done

# 5. a live nvim owning the socket is not stolen (the second nvim stays unregistered)
rm -rf "$T/run"; mkdir -p "$T/run/tether"
nv nvim --headless --listen "$SOCK" -u NONE -i NONE >/dev/null 2>&1 </dev/null &
OWNER=$!
for _ in $(seq 1 50); do [ -S "$SOCK" ] && break; sleep 0.1; done
plain "+call writefile([join(serverlist(), ',')], '$T/o5')" +qa
case "$(cat "$T/o5")" in *"$SOCK"*) bad "live owner not stolen" "$(cat "$T/o5")";; *) ok "live socket owner left alone";; esac
[ -S "$SOCK" ] && ok "live socket file kept" || bad "live socket file kept" "gone"
kill $OWNER 2>/dev/null; wait $OWNER 2>/dev/null

# 6. started through bin/tether-nvim: already on the socket, nothing changes
rm -rf "$T/run"
nv HERDR_ENV=1 HERDR_PANE_ID=w9:p3 TETHER_NVIM="nvim --headless -u $T/init.lua -i NONE" "$PLUGIN/../bin/tether-nvim" "+call writefile([v:servername . ' ' . join(serverlist(), ',')], '$T/o6')" +qa >/dev/null 2>&1
eq "wrapper-started nvim keeps v:servername = the pane socket" "$(cut -d' ' -f1 "$T/o6")" "$SOCK"

echo "register tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
