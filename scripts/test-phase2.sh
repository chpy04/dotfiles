#!/usr/bin/env bash
# Phase 2 live test (F1.1-F1.5): layout / tab / split / code open in a SANDBOX Herdr workspace.
#   scripts/test-phase2.sh        (TETHER_BIN=path to override; default target/debug/tether, built first)
# Safety (docs/PLAN.md): talks only to sandbox workspaces it creates (labeled tether-sandbox-*); tether
# runs with HERDR_WORKSPACE_ID/TAB_ID/PANE_ID overridden to sandbox ids; refuses to run if those equal the
# inherited ones; no input is ever sent to an inherited pane; both sandboxes are closed by a trap.
# Neovim runs as `nvim --clean -n` (via a tiny wrapper around bin/tether-nvim), sockets live in $TMP/run.
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
command -v nvim >/dev/null && command -v jq >/dev/null || { echo "needs nvim and jq"; exit 1; }

WS=""; WS2=""; TMP="$(mktemp -d "${TMPDIR:-/tmp}/tether-p2.XXXXXX")"; TMP="${TMP%/}"
cleanup() {
  set +e
  [ -n "$WS" ] && "$ROOT/scripts/sandbox.sh" down "$WS"
  [ -n "$WS2" ] && "$ROOT/scripts/sandbox.sh" down "$WS2"
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1${2:+ :: $2}"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "got [$2] want [$3]"; fi; }

# ---- a git worktree with two files ----------------------------------------------------------------
REPO="$TMP/repo"; mkdir -p "$REPO/src/auth" "$TMP/run"
git -C "$REPO" init -q
seq 1 200 | sed 's/^/const line_/;s/$/ = 1;/' > "$REPO/src/auth/session.ts"
seq 1 50 | sed 's/^/other /' > "$REPO/src/other.ts"
REPO="$(cd "$REPO" && pwd -P)"   # /private/var/... (what Neovim reports)
SESSION="$REPO/src/auth/session.ts"; OTHER="$REPO/src/other.ts"

# wrapper used inside the new pane: isolated socket dir, config-free Neovim, real bin/tether-nvim
cat > "$TMP/nvim-wrapper.sh" <<W
#!/bin/sh
XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM="nvim --clean -n" exec "$ROOT/bin/tether-nvim" "\$@"
W
chmod +x "$TMP/nvim-wrapper.sh"

# ---- sandbox workspaces ------------------------------------------------------------------------------------
eval "$(SANDBOX_CWD="$REPO" "$ROOT/scripts/sandbox.sh" up "phase2-$$")" || { echo "sandbox up failed"; exit 1; }
WS="$SANDBOX_WORKSPACE_ID"; TAB1="$SANDBOX_TAB_ID"; PANE1="$SANDBOX_PANE_ID"
if [ "$WS" = "w7" ] || [ "$WS" = "$INHERITED_WS" ] || [ "$PANE1" = "$INHERITED_PANE" ]; then echo "refusing: sandbox resolved to an inherited workspace/pane ($WS/$PANE1)"; exit 1; fi
eval "$(SANDBOX_CWD="$REPO" "$ROOT/scripts/sandbox.sh" up "phase2b-$$" | sed 's/SANDBOX_/OTHER_/')" || { echo "second sandbox failed"; exit 1; }
WS2="$OTHER_WORKSPACE_ID"; OTHER_TAB="$OTHER_TAB_ID"
echo "sandbox workspace $WS (tab $TAB1 pane $PANE1), second sandbox $WS2 (tab $OTHER_TAB); inherited ws=$INHERITED_WS pane=$INHERITED_PANE"

# tether always runs with the sandbox ids (never the inherited ones), state and sockets in TMP
t() { env HERDR_WORKSPACE_ID="$WS" HERDR_TAB_ID="$TAB1" HERDR_PANE_ID="$PANE1" TETHER_STATE="$TMP/state.json" \
        TETHER_CONFIG="$TMP/none.toml" XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM_WRAPPER="$TMP/nvim-wrapper.sh" "$TETHER_BIN" "$@"; }
tc() { (cd "$REPO" && t "$@"); }
tabs_json() { rpc tab.list "{\"workspace_id\":\"$WS\"}" | jq -c '.result.tabs | map({tab_id,number,label})'; }
pane_count() { rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq '.result.panes | length'; }
nvim_pos() { nvim --server "$1" --remote-expr 'expand("%:p").":".line(".")' 2>/dev/null; }
sockfor() { echo "$TMP/run/tether/${1//:/-}.sock"; }

echo "# setup: a hand-made tab (stands in for the user's own)"
HM=$(rpc tab.create "{\"workspace_id\":\"$WS\",\"label\":\"my notes\",\"focus\":false}")
HM_ID=$(echo "$HM" | jq -r .result.tab.tab_id); HM_NUM=$(echo "$HM" | jq -r .result.tab.number)
check "hand-made tab is tab 2" "$HM_NUM" 2
BEFORE_HM=$(tabs_json | jq -c --arg id "$HM_ID" '.[] | select(.tab_id==$id)')

echo "# layout"
out=$(tc layout --json); rc=$?
check "layout exit 0" "$rc" 0
check "two tabs" "$(echo "$out" | jq '.tabs | length')" 2
check "tab 1 is yours" "$(echo "$out" | jq -r '.tabs[0].created_by_tether')" false
check "hand-made tab label" "$(echo "$out" | jq -r '.tabs[1].label')" "my notes"
check "hand-made tab is not tether-made" "$(echo "$out" | jq -r '.tabs[1].created_by_tether')" false
check "tab 1 pane is the sandbox pane, current" "$(echo "$out" | jq -r '.tabs[0].panes[0] | "\(.pane_id) \(.current)"')" "$PANE1 true"
check "foreground command reported" "$(echo "$out" | jq -r '.tabs[0].panes[0].command | length > 0')" true
echo "       text form:"; tc layout | sed 's/^/       | /'

echo "# tab open"
out=$(tc tab open "session refresh" --json -- sh -c 'echo hello-from-tab; sleep 600'); rc=$?
check "tab open exit 0" "$rc" 0
TAB2=$(echo "$out" | jq -r .tab_id); TAB2_PANE=$(echo "$out" | jq -r .pane_id)
check "new tab is index 3" "$(echo "$out" | jq -r .index)" 3
check "new tab label" "$(echo "$out" | jq -r .label)" "session refresh"
check "state file records it under the workspace" "$(jq -r --arg w "$WS" '.workspaces[$w].tabs | map(.tab_id) | join(",")' "$TMP/state.json")" "$TAB2"
sleep 1.5
rpc pane.read "{\"pane_id\":\"$TAB2_PANE\",\"source\":\"recent\"}" | jq -r .result.read.text | grep -q hello-from-tab && ok "command ran in the new tab's pane" || bad "command output not seen in $TAB2_PANE"
check "focus not stolen (active tab still tab 1)" "$(rpc workspace.get "{\"workspace_id\":\"$WS\"}" | jq -r .result.workspace.active_tab_id)" "$TAB1"
out=$(tc layout --json)
check "layout marks it tether-made" "$(echo "$out" | jq -r '.tabs[2].created_by_tether')" true

echo "# tab rename / focus; rename refused on the hand-made tab"
tc tab rename "session refresh" "session refresh" >/dev/null; check "rename (same name) exit 0" "$?" 0
tc tab rename "my notes" "renamed!" >/dev/null 2>"$TMP/err"; check "rename of hand-made tab refused (exit 6)" "$?" 6
check "hand-made label untouched by refused rename" "$(tabs_json | jq -r --arg id "$HM_ID" '.[] | select(.tab_id==$id) | .label')" "my notes"

out=$(tc tab focus "session refresh" --json); rc=$?
check "tab focus exit 0" "$rc" 0
check "  active tab is now the tether-made one" "$(rpc workspace.get "{\"workspace_id\":\"$WS\"}" | jq -r .result.workspace.active_tab_id)" "$TAB2"
tc tab focus 1 >/dev/null; check "  focus back to tab 1 by index" "$(rpc workspace.get "{\"workspace_id\":\"$WS\"}" | jq -r .result.workspace.active_tab_id)" "$TAB1"

echo "# split right (in the new tab, next to its first pane)"
N0=$(pane_count)
out=$(env HERDR_TAB_ID="$TAB2" HERDR_PANE_ID="$TAB2_PANE" HERDR_WORKSPACE_ID="$WS" TETHER_STATE="$TMP/state.json" TETHER_CONFIG="$TMP/none.toml" "$TETHER_BIN" split right --ratio 0.6 --json -- sh -c 'echo split-ok; sleep 600'); rc=$?
check "split exit 0" "$rc" 0
SPLIT_PANE=$(echo "$out" | jq -r .pane_id)
check "split printed a new pane id in the tab" "$(rpc pane.get "{\"pane_id\":\"$SPLIT_PANE\"}" | jq -r .result.pane.tab_id)" "$TAB2"
check "one more pane" "$(pane_count)" $((N0+1))
sleep 1.5
rpc pane.read "{\"pane_id\":\"$SPLIT_PANE\",\"source\":\"recent\"}" | jq -r .result.read.text | grep -q split-ok && ok "command ran in the split pane" || bad "split command output not seen"

echo "# code open: no Neovim in the tab yet -> split + start, jump to line 42"
N1=$(pane_count)
out=$(tc code open src/auth/session.ts:42 --in "session refresh" --from "$TAB2_PANE" --json); rc=$?
echo "       $out"
check "code open exit 0" "$rc" 0
NV_PANE=$(echo "$out" | jq -r .pane_id)
check "not reused (new split)" "$(echo "$out" | jq -r .reused)" false
check "split from the reference pane" "$(echo "$out" | jq -r .split_from)" "$TAB2_PANE"
check "new pane is in the target tab" "$(rpc pane.get "{\"pane_id\":\"$NV_PANE\"}" | jq -r .result.pane.tab_id)" "$TAB2"
check "exactly one pane was added" "$(pane_count)" $((N1+1))
SOCK="$(sockfor "$NV_PANE")"
[ -S "$SOCK" ] && ok "registered socket exists: $SOCK" || bad "no socket at $SOCK"
check "Neovim cursor is session.ts:42 (queried over RPC)" "$(nvim_pos "$SOCK")" "$SESSION:42"

echo "# code open again at another line: same pane reused, cursor moved, no new pane"
N2=$(pane_count)
out=$(tc code open src/auth/session.ts:120 --in "session refresh" --json); rc=$?
check "second code open exit 0" "$rc" 0
check "reused" "$(echo "$out" | jq -r .reused)" true
check "same pane" "$(echo "$out" | jq -r .pane_id)" "$NV_PANE"
check "no new pane" "$(pane_count)" "$N2"
check "cursor moved to 120" "$(nvim_pos "$SOCK")" "$SESSION:120"
# default tab = the caller's tab; range ref; another file
out=$(env HERDR_TAB_ID="$TAB2" HERDR_PANE_ID="$TAB2_PANE" HERDR_WORKSPACE_ID="$WS" TETHER_STATE="$TMP/state.json" TETHER_CONFIG="$TMP/none.toml" XDG_RUNTIME_DIR="$TMP/run" "$TETHER_BIN" code open "$OTHER:5-8" --json); rc=$?
check "code open in the current tab (no --in), absolute path, range: exit 0" "$rc" 0
check "  reused the same pane" "$(echo "$out" | jq -r '"\(.reused) \(.pane_id)"')" "true $NV_PANE"
check "  other.ts:5 (range start)" "$(nvim_pos "$SOCK")" "$OTHER:5"
check "  end_line reported" "$(echo "$out" | jq -r .end_line)" 8
check "  still no extra pane" "$(pane_count)" "$N2"
check "tab holds exactly 3 panes (first, split, the one Neovim)" "$(rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq -r --arg t "$TAB2" '[.result.panes[] | select(.tab_id==$t) | .pane_id] | length')" 3
tc code open src/auth/session.ts:3 --in 99 >/dev/null 2>"$TMP/err"; check "unknown tab rejected (exit 5)" "$?" 5
tc code open src/missing.ts:3 --in 3 >/dev/null 2>"$TMP/err"; check "missing file rejected (exit 2)" "$?" 2
check "  and nothing was created" "$(pane_count)" "$N2"

echo "# layout sees the registered Neovim"
out=$(tc layout --json)
check "nvim reported in its pane" "$(echo "$out" | jq -r --arg p "$NV_PANE" '[.tabs[].panes[] | select(.pane_id==$p) | .nvim.at] | .[0]')" "$OTHER:5"
tc layout | grep "$NV_PANE" | sed 's/^/       | /'

echo "# tab close: hand-made refused, tether-made allowed"
tc tab close "my notes" >/dev/null 2>"$TMP/err"; rc=$?
check "close hand-made tab by label refused (exit 6)" "$rc" 6
grep -q "tether did not create it" "$TMP/err" && ok "refusal says why" || bad "refusal message" "$(cat "$TMP/err")"
tc tab close 2 >/dev/null 2>&1; check "close hand-made tab by index refused (exit 6)" "$?" 6
tc tab close "$HM_ID" >/dev/null 2>&1; check "close hand-made tab by id refused (exit 6)" "$?" 6
check "hand-made tab still exists" "$(tabs_json | jq -r --arg id "$HM_ID" '[.[] | select(.tab_id==$id)] | length')" 1
# another workspace's tab: refused even though it exists (and even with --force)
tc tab close "$OTHER_TAB" --force >/dev/null 2>"$TMP/err"; rc=$?
check "tab of another workspace refused (exit 6)" "$rc" 6
grep -q -- "--workspace $WS2" "$TMP/err" && ok "message points at --workspace" || bad "scope message" "$(cat "$TMP/err")"
check "other workspace's tab untouched" "$(rpc tab.list "{\"workspace_id\":\"$WS2\"}" | jq '.result.tabs | length')" 1
tc view foo >/dev/null 2>"$TMP/err"; check "view of an unknown diagram is a usage error (exit 2)" "$?" 2

out=$(tc tab close "session refresh" --json); rc=$?
check "close tether-made tab exit 0" "$rc" 0
check "  it reported created_by_tether" "$(echo "$out" | jq -r .created_by_tether)" true
check "  tab is gone" "$(tabs_json | jq -r --arg id "$TAB2" '[.[] | select(.tab_id==$id)] | length')" 0
check "  state record removed" "$(jq -r --arg w "$WS" '.workspaces[$w].tabs // [] | length' "$TMP/state.json")" 0

echo "# plain nvim (no tether-nvim wrapper) registers itself via the plugin; code open reuses it (finding 1a)"
cat > "$TMP/plain-init.lua" <<L
-- plugin-only init: nothing but the tether plugin on the runtimepath
vim.opt.rtp:prepend("$ROOT/nvim")
vim.opt.swapfile = false
vim.opt.shadafile = "NONE"
L
out=$(tc tab open "plain nvim" --json -- sh -c 'sleep 600'); rc=$?
check "tab for the plain nvim opened" "$rc" 0
PTAB=$(echo "$out" | jq -r .tab_id); PPANE=$(echo "$out" | jq -r .pane_id)
sleep 1
# the pane's shell is the sandbox pane's own (Herdr sets HERDR_ENV/HERDR_PANE_ID there); sleep is replaced by our nvim
rpc pane.send_keys "{\"pane_id\":\"$PPANE\",\"keys\":[\"ctrl+c\"]}" >/dev/null
sleep 0.5
rpc pane.send_input "$(jq -nc --arg p "$PPANE" --arg t "XDG_RUNTIME_DIR=$TMP/run exec nvim --clean -n -i NONE -u $TMP/plain-init.lua $OTHER" '{pane_id:$p,text:$t,keys:["enter"]}')" >/dev/null
PSOCK="$(sockfor "$PPANE")"
for i in $(seq 1 60); do [ -S "$PSOCK" ] && nvim_pos "$PSOCK" >/dev/null 2>&1 && break; sleep 0.25; done
[ -S "$PSOCK" ] && ok "plain nvim opened the pane socket $PSOCK" || bad "plain nvim did not register" "no socket at $PSOCK"
NP=$(pane_count)
out=$(tc code open src/auth/session.ts:77 --in "plain nvim" --json); rc=$?
check "code open exit 0" "$rc" 0
check "  reused the plain nvim's pane" "$(echo "$out" | jq -r '"\(.reused) \(.pane_id)"')" "true $PPANE"
check "  cursor is session.ts:77" "$(nvim_pos "$PSOCK")" "$SESSION:77"
check "  no pane was created" "$(pane_count)" "$NP"
tc tab close "plain nvim" >/dev/null 2>&1; check "plain-nvim tab closed (tether-made)" "$?" 0

echo "# code open default placement away from the primary agent's tab (finding 7)"
FLOG="$TMP/fake.log"; : > "$FLOG"
rpc pane.send_input "$(jq -nc --arg p "$PANE1" --arg t "FAKE_LOG=$FLOG FAKE_WORK_SECS=1 $ROOT/spike/fake-agent.sh" '{pane_id:$p,text:$t,keys:["enter"]}')" >/dev/null
for i in $(seq 1 100); do grep -q '"event": *"ready"' "$FLOG" 2>/dev/null && break; sleep 0.2; done
grep -q '"event": *"ready"' "$FLOG" && ok "fake agent running in the sandbox's tab 1 pane" || bad "fake agent did not start"
sleep 1
check "agent is the primary (tab 1)" "$(tc agents --json | jq -r '.[0] | "\(.pane_id) \(.is_primary)"')" "$PANE1 true"
T1_PANES=$(rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq --arg t "$TAB1" '[.result.panes[] | select(.tab_id==$t)] | length')
NT=$(tabs_json | jq length)
out=$(tc code open src/other.ts:9 --json); rc=$?   # no --in, no --from: the current tab is the agent's
echo "       $out"
check "code open (default target) exit 0" "$rc" 0
check "  new tab created" "$(echo "$out" | jq -r .tab_created)" true
check "  tab named after the file" "$(echo "$out" | jq -r .tab_label)" "other.ts"
NEWTAB=$(echo "$out" | jq -r .tab_id); NEWPANE=$(echo "$out" | jq -r .pane_id)
check "  one more tab" "$(tabs_json | jq length)" $((NT+1))
check "  agent's tab was not split" "$(rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq --arg t "$TAB1" '[.result.panes[] | select(.tab_id==$t)] | length')" "$T1_PANES"
check "  new tab is tether-made" "$(tc layout --json | jq -r --arg t "$NEWTAB" '.tabs[] | select(.tab_id==$t) | .created_by_tether')" true
check "  active tab still the agent's (no focus stolen)" "$(rpc workspace.get "{\"workspace_id\":\"$WS\"}" | jq -r .result.workspace.active_tab_id)" "$TAB1"
check "  Neovim is at other.ts:9" "$(nvim_pos "$(sockfor "$NEWPANE")")" "$OTHER:9"
out=$(tc code open src/auth/session.ts:30 --json); rc=$?
check "second default code open exit 0" "$rc" 0
check "  reused the Neovim in the tether tab" "$(echo "$out" | jq -r '"\(.reused) \(.pane_id) \(.tab_created)"')" "true $NEWPANE false"
check "  no extra tab" "$(tabs_json | jq length)" $((NT+1))
check "  cursor session.ts:30" "$(nvim_pos "$(sockfor "$NEWPANE")")" "$SESSION:30"
tc tab close "$NEWTAB" >/dev/null 2>&1; check "other.ts tab closed" "$?" 0
echo "# explicit --in 1 still splits next to the chat (F1.7)"
out=$(tc code open src/auth/session.ts:5 --in 1 --json); rc=$?
check "code open --in 1 exit 0" "$rc" 0
check "  split from the agent pane, no new tab" "$(echo "$out" | jq -r '"\(.reused) \(.split_from) \(.tab_created)"')" "false $PANE1 false"
check "  agent's tab now has one more pane" "$(rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq --arg t "$TAB1" '[.result.panes[] | select(.tab_id==$t)] | length')" $((T1_PANES+1))

echo "# own tab: close / rename refused without --force (finding 14)"
tc tab rename 1 "oops" >/dev/null 2>"$TMP/err"; check "rename own tab refused (exit 6)" "$?" 6
tc tab close 1 >/dev/null 2>"$TMP/err"; check "close own tab refused (exit 6)" "$?" 6
grep -q "your own tab" "$TMP/err" && ok "refusal names the reason" || bad "own-tab message" "$(cat "$TMP/err")"
check "own tab label is not 'oops'" "$([ "$(tabs_json | jq -r --arg id "$TAB1" '.[] | select(.tab_id==$id) | .label')" != oops ] && echo yes)" yes

echo "# hand-made tab unchanged at the end"
AFTER_HM=$(tabs_json | jq -c --arg id "$HM_ID" '.[] | select(.tab_id==$id)')
check "hand-made tab id/position/label identical" "$AFTER_HM" "$BEFORE_HM"
check "tab count back to 2" "$(tabs_json | jq length)" 2
check "tab 1 still first" "$(tabs_json | jq -r '.[0].tab_id')" "$TAB1"

echo
echo "phase2 live test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
