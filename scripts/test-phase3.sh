#!/usr/bin/env bash
# Phase 3 live test (F3.1, F3.2, F3.3, F3.6): the diagram viewer in a SANDBOX Herdr workspace.
#   scripts/test-phase3.sh        (TETHER_BIN=path to override; default target/debug/tether, built first)
# Steps:
#   1. `diagram create fixture --open tab` -> a tab named after the title, running the viewer, tether-made
#   2. Enter in the viewer pane -> Neovim split beside it, cursor on the linked line (F3.2)
#   3. j + Enter -> the SAME Neovim, cursor on the next link
#   4. a link whose anchor is gone: Enter does not jump and the viewer says so
#   5. <leader>ac + text + Enter -> the fake agent in tab 1 receives the diagram comment (F3.3)
#   6. q closes the viewer pane/tab
# The Neovim part needs Phase 2's `tether code open`; if the binary lacks it that part is SKIPPED with a message.
# Safety (docs/PLAN.md): talks only to the sandbox workspace it creates (tether-sandbox-*); tether runs with
# HERDR_WORKSPACE_ID/TAB_ID/PANE_ID overridden to sandbox ids; refuses to run if those equal the inherited
# ones; keys are only ever sent to the sandbox's own viewer pane; the sandbox is closed by a trap.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/.." && pwd)"
. "$ROOT/spike/lib.sh"          # rpc()
export PYTHONDONTWRITEBYTECODE=1
INHERITED_WS="${HERDR_WORKSPACE_ID:-}"; INHERITED_PANE="${HERDR_PANE_ID:-}"

if [ -z "${TETHER_BIN:-}" ]; then
  source ~/.cargo/env 2>/dev/null
  (cd "$ROOT" && cargo build -q -p tether) || { echo "cargo build failed"; exit 1; }
  TETHER_BIN="$ROOT/target/debug/tether"
fi
[ -x "$TETHER_BIN" ] || { echo "no tether binary at $TETHER_BIN"; exit 1; }
command -v jq >/dev/null || { echo "needs jq"; exit 1; }

HAVE_CODE=1
"$TETHER_BIN" code open --help >/dev/null 2>&1 || HAVE_CODE=0
HAVE_NVIM=1; command -v nvim >/dev/null || HAVE_NVIM=0
if [ "$HAVE_CODE" = 0 ]; then echo "NOTE: this tether has no 'code open' (Phase 2 not merged): the Neovim steps are SKIPPED"; fi
if [ "$HAVE_NVIM" = 0 ]; then echo "NOTE: nvim not installed: the Neovim steps are SKIPPED"; HAVE_CODE=0; fi

WS=""; TMP="$(mktemp -d "${TMPDIR:-/tmp}/tether-p3.XXXXXX")"; TMP="${TMP%/}"
cleanup() {
  set +e
  [ -n "$WS" ] && "$ROOT/scripts/sandbox.sh" down "$WS"
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

pass=0; fail=0; skip=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1${2:+ :: $2}"; }
skipped() { skip=$((skip+1)); echo "  skip $1"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "got [$2] want [$3]"; fi; }

# ---- a git worktree with the files the fixture links to --------------------------------------------------
REPO="$TMP/feat-login"; mkdir -p "$REPO/src/auth" "$TMP/run"
git -C "$REPO" init -q
python3 - "$REPO" <<'P'
import sys, os
r = sys.argv[1]
def put(rel, special):
    lines = [special.get(n, "// line %d" % n) for n in range(1, 201)]
    open(os.path.join(r, rel), "w").write("\n".join(lines) + "\n")
put("src/auth/session.ts", {142: "if (token.expiresAt < Date.now()) {"})
put("src/auth/refresh.ts", {})
P
REPO="$(cd "$REPO" && pwd -P)"
SESSION="$REPO/src/auth/session.ts"; REFRESH="$REPO/src/auth/refresh.ts"
FIXTURE="$ROOT/crates/tether-core/tests/fixtures/diagrams/prd_flowchart.json"

cat > "$TMP/nvim-wrapper.sh" <<W
#!/bin/sh
XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM="nvim --clean -n" exec "$ROOT/bin/tether-nvim" "\$@"
W
chmod +x "$TMP/nvim-wrapper.sh"

# ---- sandbox workspace + fake agent in tab 1 --------------------------------------------------------------
eval "$(SANDBOX_CWD="$REPO" "$ROOT/scripts/sandbox.sh" up "phase3-$$")" || { echo "sandbox up failed"; exit 1; }
WS="$SANDBOX_WORKSPACE_ID"; TAB1="$SANDBOX_TAB_ID"; PANE1="$SANDBOX_PANE_ID"
if [ "$WS" = "w7" ] || [ "$WS" = "$INHERITED_WS" ] || [ "$PANE1" = "$INHERITED_PANE" ]; then echo "refusing: sandbox resolved to an inherited workspace/pane ($WS/$PANE1)"; exit 1; fi
echo "sandbox workspace $WS tab $TAB1 pane $PANE1 (inherited ws=$INHERITED_WS pane=$INHERITED_PANE)"

FLOG="$TMP/fake.log"; : > "$FLOG"
rpc pane.send_input "$(jq -nc --arg p "$PANE1" --arg t "FAKE_LOG=$FLOG FAKE_WORK_SECS=1 $ROOT/spike/fake-agent.sh" '{pane_id:$p,text:$t,keys:["enter"]}')" >/dev/null
for i in $(seq 1 100); do grep -q '"event": *"ready"' "$FLOG" 2>/dev/null && break; sleep 0.2; done
grep -q '"event": *"ready"' "$FLOG" && ok "fake agent running in the sandbox's tab 1 pane" || bad "fake agent did not start"
sleep 1

# tether always runs with the sandbox ids; the env it is given is carried into the viewer pane by `diagram create`
t() { env HERDR_WORKSPACE_ID="$WS" HERDR_TAB_ID="$TAB1" HERDR_PANE_ID="$PANE1" TETHER_STATE="$TMP/state.json" \
        TETHER_CONFIG="$TMP/none.toml" XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM_WRAPPER="$TMP/nvim-wrapper.sh" "$TETHER_BIN" "$@"; }
tc() { (cd "$REPO" && t "$@"); }
tabs_json() { rpc tab.list "{\"workspace_id\":\"$WS\"}" | jq -c '.result.tabs | map({tab_id,number,label})'; }
panes_of() { rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq -r --arg t "$1" '.result.panes[] | select(.tab_id==$t) | .pane_id'; }
pane_text() { rpc pane.read "{\"pane_id\":\"$1\",\"source\":\"visible\"}" | jq -r .result.read.text; }
nvim_pos() { nvim --server "$1" --remote-expr 'expand("%:p").":".line(".")' 2>/dev/null; }
sockfor() { echo "$TMP/run/tether/${1//:/-}.sock"; }
wait_for() { # <seconds> <shell expression, re-evaluated each time>
  local n=$(( $1 * 5 )); shift; local i
  for i in $(seq 1 "$n"); do eval "$*" >/dev/null 2>&1 && return 0; sleep 0.2; done; return 1
}
# keys go to the viewer pane only (a sandbox pane we created); refuse anything else
VIEW_PANE=""
vkeys() {
  [ -n "$VIEW_PANE" ] && [ "$VIEW_PANE" != "$INHERITED_PANE" ] && [ "$(rpc pane.get "{\"pane_id\":\"$VIEW_PANE\"}" | jq -r .result.pane.workspace_id)" = "$WS" ] || { echo "refusing to send keys to $VIEW_PANE"; return 1; }
  local k; for k in "$@"; do rpc pane.send_keys "$(jq -nc --arg p "$VIEW_PANE" --arg k "$k" '{pane_id:$p,keys:[$k]}')" >/dev/null; sleep 0.15; done
}
has_text() { pane_text "$VIEW_PANE" | grep -q -- "$1"; }

echo "# 1. diagram create --open tab"
out=$(tc diagram create "$FIXTURE" --open tab --json); rc=$?
echo "       $out"
check "create exit 0" "$rc" 0
ID=$(echo "$out" | jq -r .id); VIEW_TAB=$(echo "$out" | jq -r .opened.tab_id); VIEW_PANE=$(echo "$out" | jq -r .opened.pane_id)
check "opened as a tab" "$(echo "$out" | jq -r .opened.mode)" tab
check "a tab named after the title exists" "$(tabs_json | jq -r --arg id "$VIEW_TAB" '.[] | select(.tab_id==$id) | .label')" "Session refresh"
check "the tab is tether-made (state file)" "$(jq -r --arg w "$WS" --arg t "$VIEW_TAB" '.workspaces[$w].tabs | map(select(.tab_id==$t)) | length' "$TMP/state.json")" 1
check "focus not stolen (active tab still tab 1)" "$(rpc workspace.get "{\"workspace_id\":\"$WS\"}" | jq -r .result.workspace.active_tab_id)" "$TAB1"
check "diagram stored in the worktree" "$([ -f "$REPO/.tether/diagrams/$ID.json" ] && echo yes)" yes
wait_for 15 "has_text 'Token expired'" && ok "the pane runs the viewer (it drew the diagram)" || { bad "viewer did not draw" "$(pane_text "$VIEW_PANE" | head -5)"; }
echo "       ---- viewer pane ----"; pane_text "$VIEW_PANE" | sed 's/^/       | /' | head -22
has_text "hjkl move" && ok "footer key hints are visible" || bad "footer hints"
has_text "Call /token/refresh" && ok "children are drawn" || bad "child node text"
check "the viewer is the pane's foreground process" "$(rpc pane.process_info "{\"pane_id\":\"$VIEW_PANE\"}" | jq -r '.. | .name? // empty' | grep -c '^tether$' | sed 's/^[1-9].*/1/')" 1

if [ "$HAVE_CODE" = 1 ]; then
  echo "# 2. Enter -> Neovim split at the linked line (F3.2)"
  N0=$(panes_of "$VIEW_TAB" | wc -l | tr -d ' ')
  check "viewer tab starts with one pane" "$N0" 1
  vkeys enter
  wait_for 25 'test "$(panes_of "$VIEW_TAB" | wc -l | tr -d " ")" = 2' && ok "a second pane (Neovim) appeared in the viewer's tab" || bad "no Neovim split appeared" "$(pane_text "$VIEW_PANE" | tail -4)"
  NV_PANE=$(panes_of "$VIEW_TAB" | grep -v -x "$VIEW_PANE" | head -1)
  SOCK="$(sockfor "$NV_PANE")"
  wait_for 20 'test "$(nvim_pos "$SOCK")" = "$SESSION:142"'
  check "Neovim cursor is on session.ts:142 (queried over RPC)" "$(nvim_pos "$SOCK")" "$SESSION:142"

  echo "# 3. j + Enter -> the same Neovim, next link"
  vkeys j enter
  wait_for 20 'test "$(nvim_pos "$SOCK")" = "$REFRESH:18"'
  check "cursor moved to refresh.ts:18" "$(nvim_pos "$SOCK")" "$REFRESH:18"
  check "same Neovim pane reused (still two panes)" "$(panes_of "$VIEW_TAB" | wc -l | tr -d ' ')" 2
  check "  the pane is the one we saw before" "$(panes_of "$VIEW_TAB" | grep -v -x "$VIEW_PANE" | head -1)" "$NV_PANE"
  vkeys k enter
  wait_for 20 'test "$(nvim_pos "$SOCK")" = "$SESSION:142"'
  check "k + Enter goes back: session.ts:142 in the same Neovim" "$(nvim_pos "$SOCK")" "$SESSION:142"
  check "  still exactly one Neovim" "$(panes_of "$VIEW_TAB" | wc -l | tr -d ' ')" 2

  echo "# 4. stale link: no jump, the viewer says so"
  vkeys j enter   # refresh.ts:18 so we can tell a jump from no jump
  wait_for 20 'test "$(nvim_pos "$SOCK")" = "$REFRESH:18"'
  python3 - "$REPO/src/auth/session.ts" <<'P'
import sys
p = sys.argv[1]
s = open(p).read().replace("token.expiresAt <", "REMOVED")
open(p, "w").write(s)
P
  vkeys k enter
  wait_for 10 "has_text 'stale link'" && ok "status line reports the stale link" || bad "no stale message" "$(pane_text "$VIEW_PANE" | tail -4)"
  sleep 1
  check "Neovim did NOT jump (still refresh.ts:18)" "$(nvim_pos "$SOCK")" "$REFRESH:18"
else
  skipped "steps 2-4 (Enter opens/reuses Neovim, stale links) need Phase 2's \`tether code open\` and nvim"
fi

echo "# 5. <leader>ac comment from the viewer reaches the fake agent (F3.3)"
vkeys space a c
wait_for 5 "has_text claude" && ok "comment float is open (title mentions the agent)" || bad "comment float did not open" "$(pane_text "$VIEW_PANE" | tail -8)"
echo "       ---- viewer pane with the float ----"; pane_text "$VIEW_PANE" | sed 's/^/       | /' | tail -14
SINCE=$(python3 -c 'import time;print(time.time())')
vkeys W h y '?' enter
python3 - "$FLOG" "$SINCE" <<'P' && ok "the fake agent received the diagram comment (title, node label, code ref, text)" || bad "comment never arrived at the agent"
import json, sys, time
log, since = sys.argv[1], float(sys.argv[2])
end = time.time() + 20
while time.time() < end:
    got = b""
    submits = 0
    for l in open(log):
        try: e = json.loads(l)
        except ValueError: continue
        if e.get("t", 0) < since: continue
        if "hex" in e: got += bytes.fromhex(e["hex"])
        if e.get("event") == "submit": submits += 1
    if submits:
        t = got.decode("utf8", "replace")
        print(t.replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip())
        ok = '[comment] diagram "Session refresh" \u203a node "Token expired?"  src/auth/session.ts:142' in t and t.rstrip().endswith("Why?\x1b[201~\r".rstrip())
        sys.exit(0 if ok else 1)
    time.sleep(0.2)
sys.exit(1)
P

echo "# 6. q closes the viewer"
vkeys q
wait_for 10 'test "$(rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq -r --arg p "$VIEW_PANE" "[.result.panes[] | select(.pane_id==\$p)] | length")" = 0' && ok "the viewer's pane closed when the viewer quit" || bad "viewer pane still there after q"
if [ "$HAVE_CODE" = 1 ]; then
  check "the Neovim split stays (only the viewer pane went away)" "$(panes_of "$VIEW_TAB" | grep -c .)" 1
else
  check "no Neovim was opened, so the viewer's tab went away with its only pane" "$(tabs_json | jq -r --arg id "$VIEW_TAB" '[.[] | select(.tab_id==$id)] | length')" 0
fi
check "tab 1 (the agent's) is untouched" "$(tabs_json | jq -r '.[0].tab_id')" "$TAB1"

echo
echo "phase3 live test: $pass passed, $fail failed, $skip skipped"
[ "$fail" -eq 0 ]
