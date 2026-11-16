#!/usr/bin/env bash
# Phase 0 exit gate: N runs (default 20) of {fake-agent delivery, real Claude delivery, nvim RPC jump}.
#   RUNS=20 SKIP_CLAUDE=1 spike/run-all.sh
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/.."
. "$HERE/lib.sh"
RUNS="${RUNS:-20}"; SKIP_CLAUDE="${SKIP_CLAUDE:-0}"
WS=""
cleanup() {
  set +e
  if [ -n "$WS" ]; then
    for p in "$P_CLAUDE" "$P_NVIM"; do :; done
    "$ROOT/scripts/sandbox.sh" down "$WS"
  fi
  rm -f "$(tether_sock_path "${P_NVIM:-none}")"; rm -rf "${TETHER_SPIKE_DIR:-/nonexistent}"
}
trap cleanup EXIT INT TERM

eval "$("$ROOT/scripts/sandbox.sh" up "spike-$$")" || { echo "sandbox up failed"; exit 1; }
WS="$SANDBOX_WORKSPACE_ID"; P_FAKE="$SANDBOX_PANE_ID"
# safety: never operate outside the sandbox workspace
[ "$WS" != "w7" ] || { echo "refusing: sandbox resolved to w7"; exit 1; }
P_CLAUDE=$(rpc pane.split "{\"target_pane_id\":\"$P_FAKE\",\"direction\":\"right\",\"focus\":false}" | jq -r .result.pane.pane_id)
P_NVIM=$(rpc pane.split "{\"target_pane_id\":\"$P_FAKE\",\"direction\":\"down\",\"focus\":false}" | jq -r .result.pane.pane_id)
echo "sandbox $WS fake=$P_FAKE claude=$P_CLAUDE nvim=$P_NVIM"

LOG="${TMPDIR:-/tmp}"; LOG="${LOG%/}/tether-spike-fake.log"; : > "$LOG"
send_line() { rpc pane.send_input "$(jq -nc --arg p "$1" --arg t "$2" '{pane_id:$p,text:$t,keys:["enter"]}')" >/dev/null; }

# start the three processes
send_line "$P_FAKE" "FAKE_LOG=$LOG FAKE_WORK_SECS=1 $HERE/fake-agent.sh"
SOCK="$(tether_sock_path "$P_NVIM")"; mkdir -p "$(dirname "$SOCK")"; rm -f "$SOCK"
D="${TMPDIR:-/tmp}"; D="${D%/}/tether-spike-$$"; mkdir -p "$D"; export TETHER_SPIKE_DIR="$D"; seq 1 300 | sed 's/^/line /' > "$D/a.txt"; seq 1 50 | sed 's/^/other /' > "$D/b.txt"
send_line "$P_NVIM" "nvim --listen '$SOCK' '$D/a.txt'"
[ "$SKIP_CLAUDE" = 1 ] || send_line "$P_CLAUDE" "claude --model haiku"
for i in $(seq 1 100); do [ -S "$SOCK" ] && break; sleep 0.2; done
[ -S "$SOCK" ] || { echo "nvim socket never appeared at $SOCK"; exit 1; }

pass=0; fails=0
for i in $(seq 1 "$RUNS"); do
  ok=1; out=""
  r=$(python3 "$HERE/check_delivery.py" fake "$P_FAKE" "$LOG" 2>&1) || ok=0; out+="  fake:   $r"$'\n'
  if [ "$SKIP_CLAUDE" != 1 ]; then r=$(python3 "$HERE/check_delivery.py" claude "$P_CLAUDE" 2>&1) || ok=0; out+="  claude: $r"$'\n'; fi
  r=$("$HERE/check-nvim.sh" "$P_NVIM" 2>&1) || ok=0; out+="  nvim:   $r"$'\n'
  if [ $ok = 1 ]; then pass=$((pass+1)); echo "run $i: PASS"; else fails=$((fails+1)); echo "run $i: FAIL"; printf '%s' "$out"; fi
  [ "$ok" = 1 ] || true
done
echo "RESULT: $pass/$RUNS passed"
# graceful exits (sandbox is torn down by the trap regardless)
rpc pane.send_keys "{\"pane_id\":\"$P_FAKE\",\"keys\":[\"ctrl+c\"]}" >/dev/null
[ "$SKIP_CLAUDE" = 1 ] || send_line "$P_CLAUDE" "/exit"
send_line "$P_NVIM" ":qa!"
sleep 1
[ "$pass" = "$RUNS" ]
