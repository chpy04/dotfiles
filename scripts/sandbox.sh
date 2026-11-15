#!/usr/bin/env bash
# Sandbox Herdr workspace for live tests.
#   sandbox.sh up [suffix]   -> creates workspace "tether-sandbox-<suffix>" (no focus), prints env exports
#   sandbox.sh down <ws-id>  -> closes that workspace (refuses anything not labeled tether-sandbox-*)
#   sandbox.sh list          -> lists tether-sandbox-* workspaces
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
RPC="$HERE/../spike/herdr_rpc.py"
cmd="${1:-}"; shift || true
case "$cmd" in
  up)
    suffix="${1:-$$}"
    out=$(python3 "$RPC" workspace.create "{\"label\":\"tether-sandbox-$suffix\",\"focus\":false,\"cwd\":\"${SANDBOX_CWD:-$PWD}\"}")
    echo "$out" | python3 -c '
import json,sys
r=json.load(sys.stdin)["result"]
print("export SANDBOX_WORKSPACE_ID=%s"%r["workspace"]["workspace_id"])
print("export SANDBOX_TAB_ID=%s"%r["tab"]["tab_id"])
print("export SANDBOX_PANE_ID=%s"%r["root_pane"]["pane_id"])'
    ;;
  down)
    ws="${1:?workspace id}"
    label=$(python3 "$RPC" workspace.get "{\"workspace_id\":\"$ws\"}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["result"]["workspace"]["label"])')
    case "$label" in tether-sandbox-*) ;; *) echo "refusing to close $ws (label: $label)" >&2; exit 1;; esac
    python3 "$RPC" workspace.close "{\"workspace_id\":\"$ws\"}" >/dev/null && echo "closed $ws"
    ;;
  list)
    python3 "$RPC" workspace.list | python3 -c '
import json,sys
for w in json.load(sys.stdin)["result"]["workspaces"]:
    if w.get("label","").startswith("tether-sandbox-"): print(w["workspace_id"],w["label"])'
    ;;
  *) echo "usage: $0 up [suffix] | down <ws-id> | list" >&2; exit 2;;
esac
