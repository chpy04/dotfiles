#!/usr/bin/env bash
# End-to-end acceptance with a REAL Claude Code (PRD Feature 1 acceptance + the comment loop).
#   scripts/test-e2e.sh            (E2E_MODEL=sonnet by default; E2E_KEEP=1 keeps the temp project;
#                                   E2E_OUT=dir saves the agent transcripts there)
# What it does, all inside a SANDBOX Herdr workspace (tether-sandbox-*):
#   - a temp git project with a few small TypeScript files, the skill linked PROJECT-LOCALLY
#     (<proj>/.claude/skills/tether -> repo skill/), tether on PATH for that pane only
#   - tab 1: `claude --model sonnet` (permission mode "auto" + a project allowlist: Bash(tether:*), Read, Glob, Grep; no bypass)
#   - a hand-made tab "my notes" (must stay untouched)
#   - prompt 1  "make a flowchart of the session refresh logic"   -> (a)-(e) assertions
#   - prompt 2  "open the line where we check token expiry"       -> Neovim at the expiry check
#   - `tether send --to primary ...` as the comment loop          -> the agent answers it
# Cost: three short real-model turns per run.
# Safety (docs/PLAN.md): input only ever goes to the sandbox's own panes (refuses the inherited
# workspace/pane); tether runs with sandbox ids; the sandbox is closed and claude killed by a trap.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/.." && pwd)"
. "$ROOT/spike/lib.sh"          # rpc()
export PYTHONDONTWRITEBYTECODE=1
INHERITED_WS="${HERDR_WORKSPACE_ID:-}"; INHERITED_PANE="${HERDR_PANE_ID:-}"
MODEL="${E2E_MODEL:-sonnet}"
WAIT_MS="${E2E_WAIT_MS:-360000}"
# Permissions without prompts, narrowest first. "manual" + the allowlist alone blocks as soon as the agent
# greps in a shell loop, so the default is Claude Code's "auto" mode (the user's own default; a classifier
# approves routine commands) ON TOP of the project allowlist. E2E_PERMISSION_MODE=bypassPermissions is the
# fallback where auto mode is unavailable.
PERMS="${E2E_PERMISSION_MODE:-auto}"

source ~/.cargo/env 2>/dev/null
(cd "$ROOT" && cargo build -q -p tether) || { echo "cargo build failed"; exit 1; }
TETHER_BIN="$ROOT/target/debug/tether"
for c in jq nvim claude git dot; do command -v "$c" >/dev/null || { echo "needs $c"; exit 1; }; done

WS=""; PROJ=""
TMP="$(mktemp -d "${TMPDIR:-/tmp}/tether-e2e.XXXXXX")"; TMP="${TMP%/}"
OUT="${E2E_OUT:-$TMP/out}"; mkdir -p "$OUT"
cleanup() {
  set +e
  if [ -n "$WS" ]; then
    # leave claude cleanly, then make sure nothing survives (processes whose cwd is the temp project)
    rpc pane.send_keys "{\"pane_id\":\"$PANE1\",\"keys\":[\"ctrl+c\"]}" >/dev/null 2>&1; sleep 0.4
    rpc pane.send_keys "{\"pane_id\":\"$PANE1\",\"keys\":[\"ctrl+c\"]}" >/dev/null 2>&1; sleep 0.8
    "$ROOT/scripts/sandbox.sh" down "$WS"
  fi
  if [ -n "$PROJ" ]; then
    for pid in $(pgrep -x claude 2>/dev/null) $(pgrep -f 'claude --model' 2>/dev/null); do
      cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')
      case "$cwd" in "$PROJ"*) kill "$pid" 2>/dev/null; sleep 0.3; kill -9 "$pid" 2>/dev/null;; esac
    done
  fi
  if [ -n "${E2E_KEEP:-}" ]; then echo "kept $TMP"; else rm -rf "$TMP"; fi
}
trap cleanup EXIT INT TERM

pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   $1"; }
bad() { fail=$((fail+1)); echo "  FAIL $1${2:+ :: $2}"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "got [$2] want [$3]"; fi; }
wait_for() { local n=$(( $1 * 5 )); shift; local i; for i in $(seq 1 "$n"); do eval "$*" >/dev/null 2>&1 && return 0; sleep 0.2; done; return 1; }

# ---- the temp project: a git repo with small real sources ------------------------------------------------
PROJ="$TMP/webapp"; mkdir -p "$PROJ/src/auth" "$TMP/run"
cat > "$PROJ/src/auth/refresh.ts" <<'EOF'
// Talks to the auth server to swap a refresh token for a new token pair.
export interface TokenPair {
  accessToken: string;
  refreshToken: string;
  expiresAt: number; // epoch ms, taken from the server's expires_in plus the client clock
}

export class SessionExpiredError extends Error {}

export async function refreshToken(refreshTokenValue: string): Promise<TokenPair> {
  const res = await fetch("/token/refresh", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ refresh_token: refreshTokenValue }),
  });
  if (res.status === 401) {
    throw new SessionExpiredError("refresh token rejected");
  }
  if (!res.ok) {
    throw new Error(`refresh failed: ${res.status}`);
  }
  const body = await res.json();
  return {
    accessToken: body.access_token,
    refreshToken: body.refresh_token,
    expiresAt: Date.now() + body.expires_in * 1000,
  };
}
EOF
cat > "$PROJ/src/auth/session.ts" <<'EOF'
import { refreshToken, SessionExpiredError, TokenPair } from "./refresh";

const SKEW_MS = 30_000;
const MAX_ATTEMPTS = 3;

export interface Session {
  userId: string;
  tokens: TokenPair;
}

let current: Session | null = null;

export function setSession(s: Session | null): void {
  current = s;
}

// Returns a session whose access token is valid, refreshing it when it is about to expire.
export async function ensureFreshSession(): Promise<Session> {
  if (!current) {
    throw new Error("not signed in");
  }
  const { tokens } = current;
  if (tokens.expiresAt < Date.now() + SKEW_MS) {
    const next = await refreshWithRetry(tokens.refreshToken);
    current = { ...current, tokens: next };
  }
  return current;
}

async function refreshWithRetry(rt: string): Promise<TokenPair> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await refreshToken(rt);
    } catch (e) {
      if (e instanceof SessionExpiredError || attempt >= MAX_ATTEMPTS) {
        current = null; // force a fresh login
        throw e;
      }
      await new Promise((r) => setTimeout(r, 200 * 2 ** attempt));
    }
  }
}
EOF
EXPIRY_LINE=$(grep -n 'expiresAt < Date.now' "$PROJ/src/auth/session.ts" | cut -d: -f1)
git -C "$PROJ" init -q
git -C "$PROJ" add -A
git -C "$PROJ" -c user.name=e2e -c user.email=e2e@example.invalid commit -q -m "initial"
PROJ="$(cd "$PROJ" && pwd -P)"
echo "project $PROJ ($(cat "$PROJ"/src/auth/*.ts | wc -l | tr -d ' ') lines of source, expiry check at session.ts:$EXPIRY_LINE)"

# the skill is available to THIS project only, plus a narrow permission allowlist (no bypass mode)
mkdir -p "$PROJ/.claude/skills"
ln -s "$ROOT/skill" "$PROJ/.claude/skills/tether"
cat > "$PROJ/.claude/settings.json" <<'EOF'
{
  "permissions": {
    "allow": ["Bash(tether:*)", "Read", "Glob", "Grep"]
  }
}
EOF
check "skill is project-local (symlink to the repo skill/)" "$(readlink "$PROJ/.claude/skills/tether")" "$ROOT/skill"
[ -e "$HOME/.claude/skills/tether" ] && echo "  note: ~/.claude/skills/tether also exists (installed); the project-local copy is the same dir"

# nvim in new splits: plain `nvim --clean`, sockets under the temp dir
cat > "$TMP/nvim-wrapper.sh" <<W
#!/bin/sh
XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM="nvim --clean -n" exec "$ROOT/bin/tether-nvim" "\$@"
W
chmod +x "$TMP/nvim-wrapper.sh"
# what the claude pane runs: tether (target/debug) and bin/ on PATH, state/sockets in the temp dir
cat > "$TMP/launch.sh" <<L
#!/bin/sh
cd "$PROJ" || exit 1
export PATH="$ROOT/target/debug:$ROOT/bin:\$PATH"
export TETHER_STATE="$TMP/state.json" TETHER_CONFIG="$TMP/none.toml" XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM_WRAPPER="$TMP/nvim-wrapper.sh"
export TETHER_DATA="$TMP/data"   # the diagram library stays in the temp dir
unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT
exec claude --model $MODEL --permission-mode $PERMS
L
chmod +x "$TMP/launch.sh"

# ---- sandbox workspace ------------------------------------------------------------------------------------
eval "$(SANDBOX_CWD="$PROJ" "$ROOT/scripts/sandbox.sh" up "e2e-$$")" || { echo "sandbox up failed"; exit 1; }
WS="$SANDBOX_WORKSPACE_ID"; TAB1="$SANDBOX_TAB_ID"; PANE1="$SANDBOX_PANE_ID"
if [ "$WS" = "w7" ] || [ "$WS" = "$INHERITED_WS" ] || [ "$PANE1" = "$INHERITED_PANE" ]; then echo "refusing: sandbox resolved to an inherited workspace/pane ($WS/$PANE1)"; WS=""; exit 1; fi
echo "sandbox workspace $WS tab $TAB1 pane $PANE1 (inherited ws=$INHERITED_WS pane=$INHERITED_PANE)"

t() { env HERDR_WORKSPACE_ID="$WS" HERDR_TAB_ID="$TAB1" HERDR_PANE_ID="$PANE1" TETHER_STATE="$TMP/state.json" TETHER_DATA="$TMP/data" \
        TETHER_CONFIG="$TMP/none.toml" XDG_RUNTIME_DIR="$TMP/run" TETHER_NVIM_WRAPPER="$TMP/nvim-wrapper.sh" "$TETHER_BIN" "$@"; }
tc() { (cd "$PROJ" && t "$@"); }
tabs_json() { rpc tab.list "{\"workspace_id\":\"$WS\"}" | jq -c '.result.tabs | map({tab_id,number,label})'; }
panes_json() { rpc pane.list "{\"workspace_id\":\"$WS\"}" | jq -c '.result.panes'; }
pane_text() { rpc pane.read "{\"pane_id\":\"$1\",\"source\":\"${2:-visible}\"}" | jq -r .result.read.text; }
agent_state() { rpc agent.get "{\"target\":\"$PANE1\"}" | jq -r '.result.agent.agent_status // "none"'; }
nvim_expr() { nvim --server "$1" --remote-expr "$2" 2>/dev/null; }
# blocking agent calls need a longer socket timeout than rpc()'s 30s
rpc_long() { python3 -c '
import json, sys
sys.path.insert(0, sys.argv[1])
from herdr_rpc import call
r = call(sys.argv[2], json.loads(sys.argv[3]), timeout=float(sys.argv[4]))
print(json.dumps(r))' "$ROOT/spike" "$1" "$2" "$(( WAIT_MS / 1000 + 30 ))"; }
agent_read() { rpc agent.read "{\"target\":\"$PANE1\",\"source\":\"recent_unwrapped\",\"lines\":${1:-250}}" | jq -r .result.read.text; }

# ---- a hand-made tab (stands in for the user's own) -------------------------------------------------------
HM=$(rpc tab.create "{\"workspace_id\":\"$WS\",\"label\":\"my notes\",\"focus\":false}")
HM_ID=$(echo "$HM" | jq -r .result.tab.tab_id); HM_NUM=$(echo "$HM" | jq -r .result.tab.number)
check "hand-made tab 'my notes' is tab 2" "$HM_NUM" 2
HM_PANE=$(panes_json | jq -r --arg t "$HM_ID" '.[] | select(.tab_id==$t) | .pane_id' | head -1)
rpc pane.send_input "{\"pane_id\":\"$HM_PANE\",\"text\":\"echo my-private-notes\",\"keys\":[\"enter\"]}" >/dev/null
sleep 1
BEFORE_HM_TAB=$(tabs_json | jq -c --arg id "$HM_ID" '.[] | select(.tab_id==$id)')
BEFORE_HM_PANES=$(panes_json | jq -c --arg t "$HM_ID" '[.[] | select(.tab_id==$t) | .pane_id]')
BEFORE_HM_TEXT=$(pane_text "$HM_PANE" recent)

# ---- start claude in tab 1 --------------------------------------------------------------------------------
echo "# starting claude --model $MODEL in the sandbox's tab 1 (project-local skill, allowlist permissions)"
rpc pane.send_input "$(jq -nc --arg p "$PANE1" --arg t "$TMP/launch.sh" '{pane_id:$p,text:$t,keys:["enter"]}')" >/dev/null
ready=0
for i in $(seq 1 150); do
  txt=$(pane_text "$PANE1")
  if echo "$txt" | grep -q "Yes, I trust this folder"; then
    # the dialog highlights "No, exit"; move to "Yes" first (only ever the sandbox pane)
    rpc pane.send_keys "{\"pane_id\":\"$PANE1\",\"keys\":[\"down\",\"enter\"]}" >/dev/null; sleep 2; continue
  fi
  if [ "$(agent_state)" = "idle" ] && echo "$txt" | grep -q "Claude Code"; then ready=1; break; fi
  sleep 1
done
[ "$ready" = 1 ] && ok "claude is up and idle in the sandbox's tab 1" || { bad "claude did not come up" "$(pane_text "$PANE1" | tail -8)"; exit 1; }

send_prompt() { # <text> -> blocks until the agent settles; prints the final state
  local r st
  r=$(rpc_long agent.prompt "$(jq -nc --arg t "$PANE1" --arg x "$1" --argjson ms "$WAIT_MS" '{target:$t,text:$x,wait:{timeout_ms:$ms}}')")
  st=$(echo "$r" | jq -r '.result.agent.agent_status // .error.code // "?"')
  echo "       agent.prompt -> $st"
  [ -n "$(echo "$r" | jq -r '.error // empty')" ] && echo "       $r" | head -c 600
  sleep 2
  agent_state
}

# =============================================================================================================
echo "# prompt 1: make a flowchart of the session refresh logic"
t0=$(date +%s)
st=$(send_prompt "make a flowchart of the session refresh logic" | tail -1)
echo "       agent settled as: $st after $(( $(date +%s) - t0 ))s"
agent_read 300 > "$OUT/transcript-1.txt"
case "$st" in idle|done) ok "agent finished prompt 1 without blocking (state $st)";; *) bad "agent did not settle idle/done" "state=$st :: $(pane_text "$PANE1" | tail -12)";; esac
sleep 3

LAY=$(tc layout --json)
echo "$LAY" | jq -c '.tabs[] | {n:(.index // .number), label, id:(.tab_id // .id), tether:.created_by_tether, panes:[.panes[] | {pane_id, command}]}' | sed 's/^/       /'
# (a) a new tab relating to session refresh, recorded as tether-made
NEWTAB=$(tabs_json | jq -r --arg a "$TAB1" --arg b "$HM_ID" '.[] | select(.tab_id!=$a and .tab_id!=$b) | select(.label | test("session|refresh|token"; "i")) | .tab_id' | head -1)
[ -n "$NEWTAB" ] && ok "(a) a new tab about session refresh exists: $NEWTAB \"$(tabs_json | jq -r --arg t "$NEWTAB" '.[] | select(.tab_id==$t) | .label')\"" || bad "(a) no new tab with a session/refresh/token label" "$(tabs_json)"
check "(a) it is recorded as tether-made in TETHER_STATE" "$(jq -r --arg w "$WS" --arg t "${NEWTAB:-none}" '[.workspaces[$w].tabs[]? | select(.tab_id==$t)] | length' "$TMP/state.json" 2>/dev/null)" 1
# (b) its pane runs `tether view <id>`
DIAG_DIR="$(tc diagram path)"
NDIAG=$(ls "$DIAG_DIR"/*.json 2>/dev/null | wc -l | tr -d ' ')
VIEW_CMD=$(echo "$LAY" | jq -r --arg t "${NEWTAB:-none}" '.tabs[] | select((.tab_id // .id)==$t) | .panes[] | .command' | head -3)
echo "       viewer pane command: $VIEW_CMD"
echo "$VIEW_CMD" | grep -Eq 'tether( |.*[ /])view [A-Za-z0-9_.-]+' && ok "(b) the new tab's pane runs \`tether view <id>\`" || bad "(b) pane command is not tether view" "$VIEW_CMD"
VIEW_ID=$(echo "$VIEW_CMD" | grep -Eo 'view [A-Za-z0-9_.-]+' | head -1 | cut -d' ' -f2)
# (c) a valid flowchart, with anchors that point at real lines
if [ "$NDIAG" -ge 1 ]; then ok "(c) the per-repo library ($DIAG_DIR) has $NDIAG diagram(s)"; else bad "(c) no diagram stored"; fi
ID="${VIEW_ID:-$(ls "$DIAG_DIR" 2>/dev/null | head -1 | sed 's/\.json$//')}"
SPEC=$(tc diagram show "$ID" 2>"$TMP/show.err"); [ -n "$SPEC" ] || bad "(c) tether diagram show $ID failed" "$(cat "$TMP/show.err")"
echo "$SPEC" > "$OUT/diagram.json"
check "(c) the diagram is a flowchart" "$(echo "$SPEC" | jq -r .type)" flowchart
check "(c) the viewer id matches a stored diagram" "$([ -f "$DIAG_DIR/$ID.json" ] && echo yes)" yes
echo "       diagram \"$(echo "$SPEC" | jq -r .title)\": $(echo "$SPEC" | jq '.nodes | length') nodes, $(echo "$SPEC" | jq '.edges | length') edges, $(echo "$SPEC" | jq '[.nodes[] | select(.code)] | length') with code"
python3 - "$PROJ" "$OUT/diagram.json" <<'P' && ok "(c) every node's code ref has an anchor that resolves to a real line" || bad "(c) some code refs do not resolve"
import json, sys, os
# same rule as tether_core::diagram::resolve: anchor on the stated line = exact; found elsewhere = moved
# (the viewer follows it); not found = stale (a failure here)
root, spec = sys.argv[1], json.load(open(sys.argv[2]))
refs = [(n["id"], n["code"]) for n in spec.get("nodes", []) if n.get("code")]
errs, exact, moved = [], 0, []
if len(refs) < 3: errs.append("only %d nodes have code refs" % len(refs))
for nid, c in refs:
    p = os.path.join(root, c["path"])
    if not os.path.isfile(p): errs.append("%s: no file %s" % (nid, c["path"])); continue
    lines = open(p).read().split("\n")
    a = c.get("anchor")
    if not a: errs.append("%s: no anchor" % nid); continue
    ln = c["line"]
    if 1 <= ln <= len(lines) and a in lines[ln - 1]: exact += 1; continue
    hits = [i + 1 for i, l in enumerate(lines) if a in l]
    if not hits: errs.append("%s: anchor %r not found in %s (stale)" % (nid, a, c["path"]))
    else: moved.append("%s: %s:%d -> %d" % (nid, c["path"], ln, hits[0]))
print("   code refs: %d, exact %d, moved %d" % (len(refs), exact, len(moved)))
for m in moved: print("   ~ moved", m)
for e in errs: print("   -", e)
sys.exit(1 if errs else 0)
P
# the static render re-resolves links the way the viewer does: no stale badge
tc diagram render "$ID" --no-icons >"$OUT/render.txt" 2>&1; grep -q 'stale' "$OUT/render.txt" && bad "(c) render reports stale links" || ok "(c) render shows no stale links"
# (d) the hand-made tab is unchanged
check "(d) hand-made tab: same id, position and label" "$(tabs_json | jq -c --arg id "$HM_ID" '.[] | select(.tab_id==$id)')" "$BEFORE_HM_TAB"
check "(d) hand-made tab: same panes, nothing typed into it" "$(panes_json | jq -c --arg t "$HM_ID" '[.[] | select(.tab_id==$t) | .pane_id]')" "$BEFORE_HM_PANES"
check "(d) hand-made tab: same screen text" "$(pane_text "$HM_PANE" recent)" "$BEFORE_HM_TEXT"
check "(d) tab 1 is still the first tab" "$(tabs_json | jq -r '.[0].tab_id')" "$TAB1"
# (e) tab 1 only holds the agent pane
check "(e) tab 1 holds exactly one pane" "$(panes_json | jq --arg t "$TAB1" '[.[] | select(.tab_id==$t)] | length')" 1
check "(e) ... and it is the agent pane" "$(panes_json | jq -r --arg t "$TAB1" '.[] | select(.tab_id==$t) | .pane_id')" "$PANE1"
check "focus not stolen (active tab is still tab 1)" "$(rpc workspace.get "{\"workspace_id\":\"$WS\"}" | jq -r .result.workspace.active_tab_id)" "$TAB1"
echo "       ---- the viewer ----"; VP=$(panes_json | jq -r --arg t "${NEWTAB:-none}" '.[] | select(.tab_id==$t) | .pane_id' | head -1); [ -n "$VP" ] && pane_text "$VP" | sed 's/^/       | /' | head -26

# =============================================================================================================
echo "# prompt 2: open the line where we check token expiry"
st=$(send_prompt "open the line where we check token expiry" | tail -1)
agent_read 300 > "$OUT/transcript-2.txt"
case "$st" in idle|done) ok "agent finished prompt 2 (state $st)";; *) bad "agent did not settle after prompt 2" "state=$st :: $(pane_text "$PANE1" | tail -12)";; esac
sleep 3
NVIM_OK=0
for sock in "$TMP"/run/tether/*.sock; do
  [ -S "$sock" ] || continue
  pos=$(nvim_expr "$sock" 'expand("%:p").":".line(".")'); line=$(nvim_expr "$sock" 'getline(".")')
  pane="$(basename "$sock" .sock)"; pane="${pane/-/:}"
  ptab=$(panes_json | jq -r --arg p "$pane" '.[] | select(.pane_id==$p) | .tab_id')
  echo "       nvim $pane in ${ptab:-?}: $pos  | $line"
  if [ "$pos" = "$PROJ/src/auth/session.ts:$EXPIRY_LINE" ] && echo "$line" | grep -q 'expiresAt <'; then
    NVIM_OK=1; NVIM_TAB="$ptab"
  fi
done
[ "$NVIM_OK" = 1 ] && ok "Neovim is open at session.ts:$EXPIRY_LINE on the expiry check" || bad "no Neovim at session.ts:$EXPIRY_LINE" "sockets: $(ls "$TMP"/run/tether 2>/dev/null | tr '\n' ' ')"
[ "$NVIM_OK" = 1 ] && { [ "$NVIM_TAB" != "$TAB1" ] && ok "  ... not in tab 1 ($NVIM_TAB)" || bad "  ... it opened in tab 1 (the primary agent's tab)"; }
check "tab 1 still holds only the agent pane" "$(panes_json | jq --arg t "$TAB1" '[.[] | select(.tab_id==$t)] | length')" 1
check "hand-made tab still untouched" "$(tabs_json | jq -c --arg id "$HM_ID" '.[] | select(.tab_id==$id)')" "$BEFORE_HM_TAB"

# =============================================================================================================
echo "# comment loop: tether send --to primary (as the Neovim hotkey would)"
MARK="why client clock here? answer in one sentence"
out=$(tc send --to primary --ref "src/auth/session.ts:$EXPIRY_LINE" --text "$MARK" 2>&1); rc=$?
echo "       tether send -> exit $rc: $(echo "$out" | head -2 | tr '\n' ' ')"
check "tether send exit 0" "$rc" 0
# the agent starts working on the comment; wait for that, then for it to settle again
wait_for 30 '[ "$(agent_state)" = working ]' || echo "       (agent never showed 'working'; it may have answered already)"
r=$(rpc_long agent.wait "$(jq -nc --arg t "$PANE1" --argjson ms "$WAIT_MS" '{target:$t,timeout_ms:$ms}')")
echo "       agent.wait -> $(echo "$r" | jq -r '.result.agent.agent_status // .error.code // "?"')"
sleep 3
agent_read 300 > "$OUT/transcript-3.txt"
# everything after the echoed comment is the answer
ANSWER=$(awk -v m="answer in one sentence" 'index($0,m){found=NR} {line[NR]=$0} END{for(i=found+1;i<=NR;i++) print line[i]}' "$OUT/transcript-3.txt")
echo "       ---- agent's answer ----"; echo "$ANSWER" | sed 's/^/       | /' | head -14
echo "$ANSWER" | grep -qi 'clock' && ok "the agent's reply addresses the comment (mentions the client clock)" || bad "reply after the comment does not mention the client clock"
case "$(agent_state)" in idle|done) ok "agent settled again (idle/done)";; *) bad "agent not settled" "$(agent_state)";; esac

echo
echo "# teardown happens in the trap (claude killed, sandbox closed)"
echo "e2e: $pass passed, $fail failed  (model $MODEL, transcripts in $OUT)"
[ "$fail" -eq 0 ]
