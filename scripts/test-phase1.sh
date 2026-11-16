#!/usr/bin/env bash
# Phase 1 live test: `tether send` delivers to a fake agent in a SANDBOX Herdr workspace.
#   scripts/test-phase1.sh            (TETHER_BIN=path to override the binary; default target/debug/tether,
#                                      built first with cargo unless TETHER_BIN is set)
# Safety (docs/PLAN.md): only ever talks to the sandbox workspace it creates; tether is run with
# HERDR_WORKSPACE_ID/TAB_ID/PANE_ID overridden to the sandbox; refuses to run if those ids equal the
# inherited ones; the sandbox is closed by a trap.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/.."
. "$ROOT/spike/lib.sh"          # rpc()
export PYTHONDONTWRITEBYTECODE=1
PYC="$HERE/test_phase1_check.py"
INHERITED_WS="${HERDR_WORKSPACE_ID:-}"; INHERITED_PANE="${HERDR_PANE_ID:-}"

if [ -z "${TETHER_BIN:-}" ]; then
  source ~/.cargo/env 2>/dev/null
  (cd "$ROOT" && cargo build -q -p tether) || { echo "cargo build failed"; exit 1; }
  TETHER_BIN="$ROOT/target/debug/tether"
fi
[ -x "$TETHER_BIN" ] || { echo "no tether binary at $TETHER_BIN"; exit 1; }

WS=""; TMP="$(mktemp -d "${TMPDIR:-/tmp}/tether-p1.XXXXXX")"; TMP="${TMP%/}"
cleanup() {
  set +e
  if [ -n "$WS" ]; then
    [ -n "${PANE:-}" ] && rpc pane.send_keys "{\"pane_id\":\"$PANE\",\"keys\":[\"ctrl+c\"]}" >/dev/null 2>&1   # sandbox pane only
    "$ROOT/scripts/sandbox.sh" down "$WS"
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1${2:+ :: $2}"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "got [$2] want [$3]"; fi; }

# ---- a git worktree to comment on ---------------------------------------------------------------------
REPO="$TMP/feat-login"; mkdir -p "$REPO/src/auth"
git -C "$REPO" init -q
python3 - "$REPO/src/auth/session.ts" <<'P'
import sys
open(sys.argv[1], "w", encoding="utf-8").write("\n".join(
    ["if (token.expiresAt < Date.now()) {" if n == 142 else "const café_%d = \"héllo → 世界\"; // line %d" % (n, n) for n in range(1, 201)]) + "\n")
P

# ---- sandbox workspace + fake agent -----------------------------------------------------------------------
eval "$(SANDBOX_CWD="$REPO" "$ROOT/scripts/sandbox.sh" up "phase1-$$")" || { echo "sandbox up failed"; exit 1; }
WS="$SANDBOX_WORKSPACE_ID"; TAB="$SANDBOX_TAB_ID"; PANE="$SANDBOX_PANE_ID"
if [ "$WS" = "w7" ] || [ "$WS" = "$INHERITED_WS" ] || [ "$PANE" = "$INHERITED_PANE" ]; then echo "refusing: sandbox resolved to an inherited workspace/pane ($WS/$PANE)"; exit 1; fi
echo "sandbox workspace $WS tab $TAB pane $PANE (inherited ws=$INHERITED_WS pane=$INHERITED_PANE)"

LOG="$TMP/fake.log"; : > "$LOG"
rpc pane.send_input "$(jq -nc --arg p "$PANE" --arg t "FAKE_LOG=$LOG FAKE_WORK_SECS=1 $ROOT/spike/fake-agent.sh" '{pane_id:$p,text:$t,keys:["enter"]}')" >/dev/null
for i in $(seq 1 100); do grep -q '"event": *"ready"' "$LOG" 2>/dev/null && break; sleep 0.2; done
grep -q '"event": *"ready"' "$LOG" || { echo "fake agent did not start"; exit 1; }
wait_idle() { local i st; for i in $(seq 1 100); do st=$(python3 "$PYC" status "$PANE"); { [ "$st" = idle ] || [ "$st" = done ]; } && return 0; sleep 0.2; done; return 1; }
wait_idle || { echo "fake agent never became idle"; exit 1; }

# tether always runs with the sandbox ids (never the inherited ones), state in TMP
t() { env HERDR_WORKSPACE_ID="$WS" HERDR_TAB_ID="$TAB" HERDR_PANE_ID="$PANE" TETHER_STATE="$TMP/state.json" TETHER_CONFIG="$TMP/none.toml" "$TETHER_BIN" "$@"; }
tc() { (cd "$REPO" && t "$@"); }   # run from inside the worktree, like an editor would

echo "# agents / primary"
out=$(tc agents --json); rc=$?
check "tether agents --json exits 0" "$rc" 0
check "lists exactly the sandbox agent" "$(echo "$out" | jq 'length')" 1
check "pane id" "$(echo "$out" | jq -r '.[0].pane_id')" "$PANE"
check "kind" "$(echo "$out" | jq -r '.[0].kind')" claude
check "tab index" "$(echo "$out" | jq -r '.[0].tab_index')" 1
check "is_primary" "$(echo "$out" | jq -r '.[0].is_primary')" true
check "state is idle or done" "$(echo "$out" | jq -r '.[0].state | IN("idle","done")')" true
check "agents is read-only: no primary tag written yet" "$([ -e "$TMP/state.json" ] && echo written || echo none)" none

echo "# send x5 (sequential, each after the agent settles)"
SINCE=$(python3 -c 'import time;print(time.time())'); EXP="$TMP/expected.bin"; : > "$EXP"
n=0
send_one() { # relpath start end text
  local rel="$1" s="$2" e="$3" text="$4" ref
  [ "$s" = "$e" ] && ref="$rel:$s" || ref="$rel:$s-$e"
  wait_idle || bad "agent idle before send $((n+1))"
  python3 "$PYC" expect "$EXP" "$REPO" "$rel" "$s" "$e" "$text"
  out=$(tc send --to primary --ref "$ref" --text "$text" 2>"$TMP/err"); rc=$?
  n=$((n+1))
  check "send $n ($ref) exit 0" "$rc" 0
  [ -s "$TMP/err" ] && echo "       stderr: $(cat "$TMP/err")"
  python3 "$PYC" wait "$LOG" "$SINCE" "$n" 20 >/dev/null || bad "send $n arrived at the agent"
}
send_one src/auth/session.ts 142 142 "Why client clock instead of server time here?"
check "first send persisted the primary tag in the state file" "$(jq -r --arg w "$WS" '.workspaces[$w].primary_pane' "$TMP/state.json")" "$PANE"
send_one src/auth/session.ts 10 14 $'Multi-line question\nsecond line: does this │ box char survive?\nthird'
send_one src/auth/session.ts 1 1 "Unicode ✓ — naïve café 日本語 🦀"
send_one src/auth/session.ts 198 200 "Range at the end of the file"
send_one src/auth/session.ts 142 142 "Same line again, 5th in a row"
python3 "$PYC" compare "$LOG" "$SINCE" "$EXP" && ok "agent received all 5 payloads byte-exact (paste markers + CR each)" || bad "byte-exact comparison"
check "exactly 5 submits" "$(python3 - "$LOG" "$SINCE" <<'P'
import json, sys
print(sum(1 for l in open(sys.argv[1]) if (lambda e: e.get("event") == "submit" and e["t"] >= float(sys.argv[2]))(json.loads(l))))
P
)" 5

echo "# send while the agent is working (F2.3): two comments back to back, no waiting"
wait_idle
SINCE2=$(python3 -c 'import time;print(time.time())'); EXP2="$TMP/expected2.bin"; : > "$EXP2"
for q in "burst one" "burst two while working"; do
  python3 "$PYC" expect "$EXP2" "$REPO" src/auth/session.ts 50 50 "$q"
  tc send --ref src/auth/session.ts:50 --text "$q" >/dev/null 2>"$TMP/err"; check "burst send '$q' exit 0" "$?" 0
done
python3 "$PYC" wait "$LOG" "$SINCE2" 2 20 >/dev/null || bad "both burst comments submitted"
python3 "$PYC" compare "$LOG" "$SINCE2" "$EXP2" && ok "both burst comments delivered intact" || bad "burst comparison"

echo "# explicit pane target + json output"
wait_idle
out=$(tc send --to "$PANE" --ref src/auth/session.ts:7 --text "explicit pane" --json); rc=$?
check "explicit pane exit 0" "$rc" 0
check "json ok" "$(echo "$out" | jq -r .ok)" true
check "json pane_id" "$(echo "$out" | jq -r .pane_id)" "$PANE"
check "json tab_index" "$(echo "$out" | jq -r .tab_index)" 1

echo "# blocked agent: warn, and Herdr decides (never falls back to raw keystrokes)"
wait_idle
python3 "$PYC" report "$PANE" blocked && sleep 0.5
BEFORE=$(wc -l < "$LOG")
out=$(tc send --ref src/auth/session.ts:9 --text "while blocked" 2>"$TMP/err"); rc=$?
echo "       exit=$rc stdout=[$out] stderr=[$(tr '\n' '|' < "$TMP/err")]"
if [ "$rc" = 0 ]; then
  grep -q "warning" "$TMP/err" && ok "blocked: delivered with a warning" || bad "blocked: expected a warning"
elif [ "$rc" = 6 ]; then
  grep -q "agent_blocked" "$TMP/err" && ok "blocked: Herdr refused, exit 6, error code shown" || bad "blocked: refusal without agent_blocked" "$(cat "$TMP/err")"
  sleep 0.5; check "blocked: nothing reached the agent" "$(wc -l < "$LOG")" "$BEFORE"
else
  bad "blocked: unexpected exit code $rc"
fi
python3 "$PYC" report "$PANE" idle; sleep 0.5

echo "# errors"
# a body starting with `/` is harmless under the default template ([comment] first) and refused only when the
# template begins with {text}; the refusal case needs its own config so nothing is delivered
printf '[comment]\ntemplate = "{text} [{path}:{line}]"\n' > "$TMP/texttpl.toml"
(cd "$REPO" && env HERDR_WORKSPACE_ID="$WS" HERDR_TAB_ID="$TAB" HERDR_PANE_ID="$PANE" TETHER_STATE="$TMP/state.json" TETHER_CONFIG="$TMP/texttpl.toml" "$TETHER_BIN" send --ref src/auth/session.ts:3 --text "/clear" >/dev/null 2>"$TMP/err"); check "slash body refused when the template starts with {text} (exit 2)" "$?" 2
tc send --ref nonsense --text "x" >/dev/null 2>"$TMP/err"; check "bad --ref rejected (exit 2)" "$?" 2
tc send --to nobody --ref src/auth/session.ts:3 --text "x" >/dev/null 2>"$TMP/err"; check "unknown target (exit 5)" "$?" 5
env -u HERDR_WORKSPACE_ID -u HERDR_PANE_ID -u HERDR_TAB_ID "$TETHER_BIN" send --ref a:1 --text x >/dev/null 2>"$TMP/err"; rc=$?
check "outside Herdr (exit 3)" "$rc" 3
grep -q "not inside a Herdr workspace" "$TMP/err" && ok "outside-Herdr message is clear" || bad "outside-Herdr message" "$(cat "$TMP/err")"

echo
echo "phase1 live test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
