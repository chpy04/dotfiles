# Phase 0 spike findings

Herdr 0.9.5, protocol 22, Claude Code 2.1.325 (`--model haiku`), Neovim 0.12.6, macOS.
Everything below was exercised live in sandbox workspaces (`scripts/sandbox.sh`), never in `w7`.

**Result: `spike/run-all.sh` passed 20/20** (each run = fake-agent delivery checks + real Claude Code
delivery check + 6 Neovim jumps; ~100 s total). Earlier smoke runs (2/3 failed, 3/3) were bugs in the
test harness, listed under Quirks.

## Wire protocol

NDJSON over the unix socket `$HERDR_SOCKET_PATH` (fallback `~/.config/herdr/herdr.sock`).
One request line `{"id":"<any>","method":"<m>","params":{...}}`, one response line
`{"id":..,"result":{"type":"<kind>",...}}` or `{"id":..,"error":{"code":"..","message":".."}}`.
A fresh connection per request works fine (`spike/herdr_rpc.py`, 25 lines of stdlib).
`params` is always required (use `{}`). Source of truth for params: `docs/herdr/api-schema.json`
(`schemas.request.$defs.*Params`). There is **no `pane.run` socket method** (that is CLI sugar for
`pane.send_input` with text + `enter`). Unknown methods return `invalid_request` listing all methods.

Error codes seen: `agent_not_found`, `pane_not_found`, `invalid_request`, `agent_not_ready`
(`"agent w8:p1 is no longer the pane foreground process"` / `"is not an active named agent"`).
`agent_blocked`, `agent_prompt_stalled`, `timeout` are documented for `agent.prompt`.

## 1. Agent delivery

### Chosen: `agent.prompt` (no `wait`), one call per comment

```json
{"id":"1","method":"agent.prompt","params":{"target":"w8:p2","text":"[comment] src/auth/session.ts:142  (worktree: feat-login)\n  142 │ if (token.expiresAt < Date.now()) {\nWhy client clock instead of server time here?"}}
```
Response: `{"result":{"type":"agent_prompted","agent":{ ...AgentInfo, agent_status is the state BEFORE submission... }}}`.

Why: byte-level capture (fake agent, `spike/fake_agent.py`) shows Herdr itself does the right thing:
`ESC[200~` + text + `ESC[201~`, then **~300 ms later** a lone `\r`. Measured gap: 301 ms (paste_end -> CR).
So there is no need for tether to add delays, to split lines, or to know about bracketed paste.
Embedded `\n` stay inside the paste and never submit early. Verified on real Claude Code: the 3-line comment,
including the `│` (U+2502) box char, shows as ONE `❯ [comment] ...` prompt, all 3 lines intact, and the model
answered it once (20/20 runs, exact-once count of each line in the echoed prompt).
`target` may be a pane id (`w8:p2`) or an agent name; names are only needed if the agent was started with
`agent.start`. Pane ids work for auto-detected agents.

Preconditions (otherwise `agent_not_ready` / `agent_blocked`, nothing is sent):
* the agent process must be the pane's **foreground process** (suspended/exited agent -> error),
* agent not sitting in an approval dialog (`blocked`).
tether should surface these errors verbatim ("agent X is blocked, answer it first").

### Alternatives measured (all work, ranked)

| Method | Bytes the agent receives | Verdict |
| --- | --- | --- |
| `agent.prompt` | `ESC[200~ text ESC[201~`, 300 ms pause, `\r` | **use** |
| `pane.send_input` `{"pane_id":..,"text":..,"keys":["enter"]}` | `ESC[200~ text ESC[201~ \r` in one write (no pause). Real Claude submitted it fine (1/1 manual + chars intact) | good fallback, but no foreground/blocked checks and no pause |
| `pane.send_text` + `pane.send_keys ["enter"]` | raw text (NO paste markers, `\n` sent as LF) then `\r` | worked on real Claude (LF inserts a newline, not submit) but relies on Claude treating raw LF as newline; do not use for multi-line |

So `pane.send_text` is *not* bracketed even if the app enabled mode 2004; `pane.send_input` and
`agent.prompt` honour the pane's live bracketed-paste mode (the fake agent was run with mode on; with it off
Herdr docs say they send plain text).

### Sending while the agent is `working` (F2.3)

`agent.prompt` is accepted while `working` (no error, no `wait` needed). Real Claude Code queued it:
the UI showed the message under "Press up to edit queued messages" and after the turn finished the queued
comments were answered (two queued multi-line comments were both processed; their replies came merged
in one turn). So tether can rely on Claude's own input queue; it does not need to wait for idle.
Don't pass `wait` for comments (it would block the CLI until the agent settles).
Note: if the agent is already `working`, `wait` can match completion of the *current* turn.

### States / `done`

`agent_status` values: `idle`, `working`, `blocked`, `done`, `unknown`. After a turn finishes while nobody
watches, the status is **`done`** (unseen completion), not `idle`; both mean "ready for input". Any
"is the agent ready" check must accept both (a bug in my first harness).

### Agent detection

* Herdr auto-detects known agents by the pane's foreground process (`claude` -> `"agent":"claude"`), no
  registration needed for real Claude.
* A plain script can register itself: `pane.report_agent`
  `{"pane_id":"w8:p1","source":"tether-fake","agent":"claude","state":"idle"}` -> `{"type":"ok"}` (states
  `idle|working|blocked|unknown`), and `pane.release_agent {pane_id,source,agent}` removes it. After report,
  `agent.list`/`pane.list` show `"agent":"claude"` for the pane. (Reported agents have no `terminal_title`.)
* BUT `agent.prompt` additionally requires the **foreground process name** to be the agent. A python/bash
  script fails with `agent_not_ready ... no longer the pane foreground process`; a `ln -s python claude`
  symlink also fails (macOS resolves it to `Python`). `spike/claude-shim.c` is a ~10-line C launcher
  named `claude` that runs the fake agent as a child; the foreground group then contains `claude` + `Python` and
  `agent.prompt` is accepted. `spike/fake-agent.sh` builds and runs it.

## 2. Neovim RPC

### Socket convention

`<runtime>/tether/<pane-id>.sock` with
`runtime = $XDG_RUNTIME_DIR` else `$TMPDIR` else `/tmp`, **trailing `/` stripped** (macOS `$TMPDIR` ends in
`/`, giving `//tether`), and **`:` in the pane id replaced by `-`**: pane `w8:p2` -> `.../tether/w8-p2.sock`.
The colon must be replaced: `nvim --listen /x/tether/w8:p2.sock` fails with
`Failed to --listen: unknown node or service` (nvim parses `a:b` as host:port) and `--server` likewise.
Length is fine (`/var/folders/gj/<22 chars>/T/tether/w8-p2.sock` is ~70 chars; limit is 104 on macOS).
Create the directory first (`mkdir -p`); nvim creates the socket only after startup (poll for `-S`).
Launch: `nvim --listen "$SOCK" file`. Helpers: `spike/lib.sh` (`tether_sock_path`).

### Jump command (works for open/hidden/new files, any mode)

`spike/nvim-jump.sh <sock> <file> <line>` runs a single synchronous `--remote-expr`
(Rust equivalent: msgpack-rpc `nvim_exec_lua` / `nvim_call_function("luaeval", ..)`):

```lua
(function(a)
  local f = vim.fn.resolve(vim.fn.fnamemodify(a[1], ":p"))
  local b = vim.fn.bufnr(f)
  local w = b > 0 and vim.fn.bufwinid(b) or -1
  if w > 0 then vim.api.nvim_set_current_win(w)          -- already displayed: just focus that window
  else vim.cmd("hide edit " .. vim.fn.fnameescape(f)) end -- open (or switch to hidden buffer)
  local n = math.max(1, math.min(a[2], vim.api.nvim_buf_line_count(0)))
  vim.api.nvim_win_set_cursor(0, { n, 0 })
  vim.cmd("normal! zz")
  return vim.fn.expand("%:p") .. ":" .. n
end)(_A)
```
invoked as `nvim --server SOCK --remote-expr "luaeval('<lua>', ['/abs/file', 142])"`. Returns `path:line`
(path as nvim knows the buffer; `/var/...` may come back as `/private/var/...`, so compare with realpath).

Why this and not the alternatives:
* `nvim_win_set_cursor` + `hide edit` is explicit and synchronous; `:e +N file` via `--remote-send` is
  asynchronous keystrokes (fails in cmdline/insert mode, shows hit-enter prompts).
* `hide edit` works even with a **modified** current buffer (plain `:edit` fails with E37).
* Verified: jump while the buffer was modified in insert mode, and while the cmdline was open
  (`:set` typed): RPC still ran and moved the cursor; mode is preserved (stays in insert/cmdline), so the
  Lua plugin may want to `vim.cmd("stopinsert")` first.
* New (nonexistent) file: opens an empty buffer, line clamps to 1. Line beyond EOF clamps to last line.
* `bufnr()` matches buffers across symlinked paths (`/var` vs `/private/var`); hence the `resolve()`.
* Read-back for tests: `nvim --server SOCK --remote-expr 'expand("%:p").":".line(".")'`.

Gotcha: a stale swap file makes nvim wait at the ATTENTION prompt while its RPC server already answers,
with an empty buffer (jump looked like it landed on line 1 of an empty buffer). `run-all.sh` uses a fresh
temp dir per run. tether should not worry about it, but tests must not reuse files across crashed nvims.

## Quirks / gotchas list

1. `pane.read`/`agent.read` with `lines: N` returns the **bottom N rows of the screen**, which are blank on
   a mostly empty 75-row Claude screen. Omit `lines` (or use a large N) and filter blank lines.
   `source` values are snake_case in JSON: `visible`, `recent`, `recent_unwrapped`, `detection`.
2. `recent`/`visible`/`detection` were identical for a Claude pane; screen includes the whole transcript so far, so
   assertions must look only at the latest `❯ [comment]` prompt (earlier runs stay in scrollback).
3. `agent_status` after a turn is `done`, not `idle` (see above).
4. `agent.send_keys` needs a *named* agent ("not an active named agent" for a pane id of an unnamed one);
   `pane.send_keys` with the pane id works for anything (used for ctrl+c).
5. Pane ids after workspace 9 are letters in the observed run: workspace ids went `w8, w9, wA, wB, wC`
   (hex-like). Treat ids as opaque strings.
6. `pane.split` `ratio` is the fraction given to the **existing** pane (ratio 0.3, direction down -> original
   30 % / new 70 %; layout showed heights 23 / 52 of 75).
7. `focus` defaults to false on `workspace.create`, `tab.create`, `pane.split`: no focus stealing, just
   don't pass `true`. The CLI's `--no-focus` is the same thing.
8. `pane.send_input` into a sandbox shell races shell startup only if sent in the first ~second;
   keystrokes are buffered by the tty so it still worked in practice.
9. Paste->CR 300 ms pause is Herdr's; tether does not add its own delay.

## Layout discovery for `tether layout`

* **Tab order/index:** `tab.list {"workspace_id":"w8"}` returns tabs in display order with `number` (1-based
  index) and `label` (defaults to the number as a string, e.g. `"1"`). `tab.create {label}` appends.
* **Panes in a tab + geometry:** `session.snapshot` -> `snapshot.layouts[]` (per tab: `area`, `panes[].rect`,
  `splits[]` with `direction`/`ratio`), also `pane.layout {"pane_id":..}`.
* **What a pane is running:** `pane.list` does NOT include a command. Use
  `pane.process_info {"pane_id":..}` -> `foreground_processes[]` (`name`, `argv`, `cmdline`, `cwd`);
  e.g. zsh idle shows `[{"name":"zsh"...}]`, nvim shows `nvim` with its argv (the `--listen` path is in
  `argv`, which also identifies tether-managed Neovim panes: find `--listen` and the file argument).
  For recognized agents `pane.list`/`agent.list` carry `agent` + `agent_status`.
* Cheapest full picture: one `session.snapshot` (workspaces, tabs, panes, agents, layouts) + one
  `pane.process_info` per non-agent pane.

## Response field reference (all verified from real responses)

Common: every object id is an opaque string. `type` discriminates the result.

### PaneInfo (inside `pane_info`, `pane_list.panes[]`, `tab_created.root_pane`, `workspace_created.root_pane`, `session_snapshot.panes[]`)
| field | type | notes |
| --- | --- | --- |
| `pane_id` | string | `w8:p1` |
| `terminal_id` | string | `term_...` |
| `workspace_id`, `tab_id` | string | |
| `focused` | bool | |
| `cwd`, `foreground_cwd` | string | `/tmp` shows as `/private/tmp` |
| `agent` | string, optional | absent when no agent (e.g. `"claude"`) |
| `terminal_title`, `terminal_title_stripped` | string, optional | only auto-detected agents with a title |
| `agent_status` | string | `idle|working|blocked|done|unknown` |
| `scroll` | `{offset_from_bottom,max_offset_from_bottom,viewport_rows}` | |
| `revision` | int | |

### workspace.create
Request: `{"label":"tether-sandbox-x","focus":false,"cwd":"/abs"}` (also `env`, `source_workspace_id`).
```json
{"result":{"type":"workspace_created",
 "workspace":{"workspace_id":"wC","number":3,"label":"tether-sandbox-shape","focused":false,"pane_count":1,"tab_count":1,"active_tab_id":"wC:t1","agent_status":"unknown"},
 "tab":{"tab_id":"wC:t1","workspace_id":"wC","number":1,"label":"1","focused":false,"pane_count":1,"agent_status":"unknown"},
 "root_pane":{ ...PaneInfo... }}}
```
Workspace fields: `workspace_id, number, label, focused, pane_count, tab_count, active_tab_id, agent_status`.
`workspace.list` -> `{"type":"workspace_list","workspaces":[Workspace...]}`; `workspace.get {workspace_id}` ->
`{"type":"workspace_info","workspace":{...}}`; `workspace.close {workspace_id}` -> `{"type":"ok"}`.

### tab.create
Request: `{"workspace_id":"w8","label":"second","focus":false,"cwd":"/abs"}`.
```json
{"result":{"type":"tab_created","tab":{"tab_id":"w8:t2","workspace_id":"w8","number":2,"label":"second","focused":false,"pane_count":1,"agent_status":"unknown"},
 "root_pane":{ ...PaneInfo, "pane_id":"w8:p3","tab_id":"w8:t2" }}}
```
`tab.list {workspace_id}` -> `{"type":"tab_list","tabs":[Tab...]}`; `tab.close {tab_id}` -> ok. Tab fields:
`tab_id, workspace_id, number, label, focused, pane_count, agent_status`.

### pane.split
Request: `{"target_pane_id":"w8:p1","direction":"right"|"down","ratio":0.3,"cwd":"/abs","focus":false}`
(`workspace_id` optional; omit target -> focused pane, so ALWAYS pass `target_pane_id`).
```json
{"result":{"type":"pane_info","pane":{"pane_id":"w8:p2","terminal_id":"term_66059400b77162a","workspace_id":"w8","tab_id":"w8:t1","focused":false,"cwd":"/Users/chrispyle/devex","foreground_cwd":"/Users/chrispyle/devex","agent_status":"unknown","scroll":{"offset_from_bottom":0,"max_offset_from_bottom":0,"viewport_rows":73},"revision":0}}}
```
New pane id: `.result.pane.pane_id`. `pane.close {pane_id}` -> ok. No `command` param: start a command with
`pane.send_input {pane_id,text,keys:["enter"]}`.

### pane.list
Request: `{"workspace_id":"w8"}` (optional filter). Response `{"type":"pane_list","panes":[PaneInfo...]}`:
```json
{"pane_id":"w8:p1","terminal_id":"term_660593b17fc8229","workspace_id":"w8","tab_id":"w8:t1","focused":false,"cwd":"/Users/chrispyle/devex","foreground_cwd":"/Users/chrispyle/devex","agent":"claude","agent_status":"idle","scroll":{"offset_from_bottom":0,"max_offset_from_bottom":0,"viewport_rows":75},"revision":0}
```
Other: `pane.get {pane_id}` -> `pane_info`; `pane.read {pane_id,source,format?,lines?,strip_ansi?}` ->
`{"type":"pane_read","read":{"pane_id","workspace_id","tab_id","source","format","text","revision","truncated"}}`;
`pane.layout {pane_id}` -> `{"type":"pane_layout","layout":{workspace_id,tab_id,zoomed,area{x,y,width,height},focused_pane_id,panes[{pane_id,focused,rect}],splits[{id,direction,ratio,rect}]}}`;
`pane.process_info {pane_id}` -> `{"type":"pane_process_info","process_info":{pane_id,shell_pid,foreground_process_group_id,foreground_processes[{pid,name,argv0,argv,cmdline,cwd}]}}`.

### agent.list
Request `{}`. Response `{"type":"agent_list","agents":[AgentInfo...]}`; `agent.get {target}` -> `{"type":"agent_info","agent":AgentInfo}`.
```json
{"terminal_id":"term_660585f7af03f27","agent":"claude","terminal_title":"◐ PRD orchestration with Sonnet subagents","terminal_title_stripped":"PRD orchestration with Sonnet subagents","agent_status":"working","workspace_id":"w7","tab_id":"w7:t1","pane_id":"w7:p1","focused":true,"state_change_seq":153,"cwd":"/Users/chrispyle/devex","foreground_cwd":"/Users/chrispyle/devex","revision":2}
```
AgentInfo fields: `terminal_id, agent, agent_status, workspace_id, tab_id, pane_id, focused, state_change_seq,
cwd, foreground_cwd, revision` + optional `terminal_title, terminal_title_stripped, completion_seq` (present after
a completed turn) and `name` (agents started with `agent.start`/renamed; not observed unnamed).
`agent.list` is global (all workspaces): filter by `workspace_id`. "Primary agent" = first/only
`agent` in the tab/workspace; tagging is tether's own state.
`agent.read {target,source,lines?}` -> same `read` object as pane.read.

### agent.prompt
Request: `{"target":"w8:p2","text":"...","wait":{"timeout_ms":60000,"until":["idle"]}}` (`wait` optional).
```json
{"result":{"type":"agent_prompted","agent":{"terminal_id":"term_66059400b77162a","agent":"claude","terminal_title":"✳ Claude Code","terminal_title_stripped":"Claude Code","agent_status":"done","workspace_id":"w8","tab_id":"w8:t1","pane_id":"w8:p2","focused":false,"state_change_seq":174,"completion_seq":174,"cwd":"/Users/chrispyle/devex","foreground_cwd":"/Users/chrispyle/devex","revision":1}}}
```
Without `wait`, returns once text+Enter are written (status in the reply may be pre-turn). With `wait` it returns
after the first settled state (`agent_status` then reflects it: `done` above, 1.7 s for a trivial haiku turn).
Errors: see top.

### session.snapshot
Request `{}` -> `{"type":"session_snapshot","snapshot":{...}}` with keys:
`version` ("0.9.5"), `protocol` (22), `focused_workspace_id`, `focused_tab_id`, `focused_pane_id`,
`workspaces[]` (Workspace), `tabs[]` (Tab), `panes[]` (PaneInfo), `agents[]` (AgentInfo),
`layouts[]` (per tab: `{workspace_id,tab_id,zoomed,area,focused_pane_id,panes[{pane_id,focused,rect{x,y,width,height}}],splits[{id,direction,ratio,rect}]}`).
The whole thing is global (all workspaces, including other people's): filter by `workspace_id`.
Example layout entry (down split at ratio 0.3):
`{"workspace_id":"wC","tab_id":"wC:t1","zoomed":false,"area":{"x":0,"y":0,"width":279,"height":75},"focused_pane_id":"wC:p1","panes":[{"pane_id":"wC:p1","focused":true,"rect":{"x":0,"y":0,"width":279,"height":23}},{"pane_id":"wC:p2","focused":false,"rect":{"x":0,"y":23,"width":279,"height":52}}],"splits":[{"id":"split_0_root","direction":"down","ratio":0.3,"rect":{"x":0,"y":0,"width":279,"height":75}}]}`

### Others used
`pane.send_text {pane_id,text}`, `pane.send_keys {pane_id,keys:["enter"|"esc"|"ctrl+c"...]}`,
`pane.send_input {pane_id,text?,keys?}`, `pane.report_agent`, `pane.release_agent`,
`workspace.close`, `tab.close`, `pane.close`: all return `{"result":{"type":"ok"}}`.

## Files

* `scripts/sandbox.sh up [suffix] | down <ws-id> | list` (`up` prints `export SANDBOX_WORKSPACE_ID/TAB_ID/PANE_ID`;
  `down` refuses workspaces not labeled `tether-sandbox-*`; never focuses)
* `spike/herdr_rpc.py` raw socket client (CLI + importable)
* `spike/fake-agent.sh`, `fake_agent.py`, `claude-shim.c` fake agent (logs bytes JSONL, reports states)
* `spike/nvim-jump.sh`, `check-nvim.sh`, `check_delivery.py`, `lib.sh`
* `spike/run-all.sh` (`RUNS=20`, `SKIP_CLAUDE=1` to skip the real-Claude check; trap tears sandbox down)
