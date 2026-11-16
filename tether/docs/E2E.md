# End-to-end acceptance with a real Claude Code

`scripts/test-e2e.sh` is the automated stand-in for the PRD's Feature 1 acceptance ("I say *make a flowchart of
the session refresh logic*; a tab named after it appears with the diagram in it; I don't touch the layout; tabs I
made by hand stay where they were") plus the comment loop (Feature 2). It drives a **real** `claude --model sonnet`
(Claude Code 2.1.325, Sonnet 5.5) in a sandbox Herdr workspace. Cost: three short model turns per run, about two
minutes of wall time.

## What it sets up

- A temp git project `webapp/` with `src/auth/session.ts` (token-refresh logic, retry with backoff, an expiry check)
  and `src/auth/refresh.ts`: 70 lines of TypeScript.
- The skill **project-locally only**: `webapp/.claude/skills/tether` is a symlink to the repo's `skill/` directory.
  Nothing under `~/.claude` or `~/dotfiles` is touched.
- A pane environment with `target/debug` and `bin/` on `PATH`; `TETHER_STATE`, `TETHER_CONFIG`,
  `XDG_RUNTIME_DIR` and a `tether-nvim` wrapper (`nvim --clean -n`) point into the temp dir.
- Permissions without prompts: `.claude/settings.json` allows `Bash(tether:*)`, `Read`, `Glob`, `Grep`, and claude
  runs with `--permission-mode auto` (the user's own default mode; see "Permission mode" below). No bypass mode.
- A sandbox workspace `tether-sandbox-e2e-<pid>`; tab 1 runs claude; a hand-made tab `my notes` (a plain shell with
  one line typed into it) is created **before** prompting. Input is only ever sent to panes of that sandbox
  (the script refuses the inherited workspace/pane, `w7`); a trap kills claude, closes the sandbox and removes the
  temp dir (`E2E_KEEP=1` keeps it, `E2E_OUT=dir` keeps the transcripts).

## Steps and assertions

| Step | Assertions |
| --- | --- |
| Prompt 1 `make a flowchart of the session refresh logic` (via `agent.prompt` with a 6 minute wait) | agent settles `idle`/`done`, never `blocked`; **(a)** a new tab whose label relates to session refresh exists and is recorded as tether-made in `TETHER_STATE`; **(b)** its pane runs `tether view <id>`; **(c)** `.tether/diagrams/` holds a valid flowchart (`tether diagram show`), at least 3 nodes carry `code` refs, every ref has an `anchor` that resolves (on the stated line = exact, elsewhere in the file = moved, absent = stale = failure), and `tether diagram render` shows no stale link; **(d)** `my notes` has the same id, position, label, panes and screen text; **(e)** tab 1 holds exactly one pane, the agent; focus was not stolen |
| Prompt 2 `open the line where we check token expiry` | agent settles; a Neovim socket answers with `session.ts:<expiry line>` and that line's text contains `expiresAt <`; the Neovim is not in tab 1; tab 1 and `my notes` still untouched |
| Comment loop `tether send --to primary --ref src/auth/session.ts:<line> --text "why client clock here? answer in one sentence"` (sandbox env, as the Neovim hotkey would) | exit 0; after the agent settles, `agent.read` shows an answer to the comment that mentions the client clock; agent settled again |

## Results

| Run | Result | Notes |
| --- | --- | --- |
| 1 | 25/27 | Test too strict, not a tether bug: one node's `line` was off by one (anchor on the next line), and the final state was `done` where the test wanted `idle`. The anchor rule became exact/moved/stale (the viewer follows moved links), and `idle`/`done` both count as settled. Skill edit 1 below. |
| 2 | blocked | `--permission-mode manual` plus the allowlist blocks the moment the agent greps in a shell `for` loop ("a variable in this command can't be checked"). Default changed to `auto` plus the allowlist. |
| 3 | 27/27 | |
| 4 | 27/27 | The agent put a stray `cat > /tmp/x` (waits on stdin) in front of `tether diagram create`, hung for about four minutes, then recovered by itself. Skill edit 2 below. |
| 5 | 27/27 | Final skill text. |
| 6 | 27/27 | Final skill text. Output below. |

Runs 5 and 6 were back to back and each took about two minutes (prompt 1 about 20 s of model time). 6 runs in total,
so the cost cap was reached; runs 1 and 2 were test-script fixes, run 4 the only agent-side flake.

### Skill edits (`skill/SKILL.md`), and why

1. **Line numbers from `grep -n`.** "never guess or count them by eye: run `grep -n "<anchor>" <file>` and use the
   line it prints, so `line` is exactly the line that contains the anchor text". Run 1 had one off-by-one line
   number; after the edit every ref in runs 3 to 6 was exact (13, 13, 12 and 14 refs).
2. **One command with the spec inline.** "`tether diagram create - <<'EOF'` ... `EOF`; never run a bare `cat` or
   any other command that waits on stdin before it". Run 4's hang.

Nothing else needed changing: in every run the agent loaded the skill, ran `tether layout` and `tether diagram
create`, left tab 1 and the hand-made tab alone, and opened code with `tether code open` into the diagram's tab
(split beside the viewer) rather than the primary agent's tab.

### Permission mode

The user's own Claude Code default is auto mode. The narrowest thing that runs without any prompt is therefore the
allowlist (always allows tether) plus `--permission-mode auto`. A pure allowlist works only if the agent sticks to
single commands; set `E2E_PERMISSION_MODE=bypassPermissions` on a machine without auto mode.

### Caveats

- Neovim runs as `nvim --clean -n` inside the test, so the plugin from `nvim/` is not loaded; sockets come from
  `tether-nvim --listen`. The real-config path is covered by `nvim/tests/`.
- It cannot judge looks: the viewer's rendering is covered by the golden tests and `docs/screens/`, and the
  "navigate it and jump to every linked node" half of the Phase 3 exit is for you to try by hand.
- Model nondeterminism: wording differs run to run; the assertions only look at the layout, the stored spec, the
  Neovim position and the word "clock" in the answer.

## Final run (run 6), `scripts/test-e2e.sh` output

(Temp paths shortened, long viewer lines cut at 170 columns.)

```
project /tmp/tether-e2e.sjCBOl/webapp (70 lines of source, expiry check at session.ts:23)
  ok   skill is project-local (symlink to the repo skill/)
sandbox workspace w1E tab w1E:t1 pane w1E:p1 (inherited ws=w7 pane=w7:p1)
  ok   hand-made tab 'my notes' is tab 2
# starting claude --model sonnet in the sandbox's tab 1 (project-local skill, allowlist permissions)
  ok   claude is up and idle in the sandbox's tab 1
# prompt 1: make a flowchart of the session refresh logic
       agent settled as: done after 22s
  ok   agent finished prompt 1 without blocking (state done)
       {"n":1,"label":"1","id":"w1E:t1","tether":false,"panes":[{"pane_id":"w1E:p1","command":"caffeinate -i -t 300"}]}
       {"n":2,"label":"my notes","id":"w1E:t2","tether":false,"panes":[{"pane_id":"w1E:p2","command":"zsh"}]}
       {"n":3,"label":"Session refresh","id":"w1E:t3","tether":true,"panes":[{"pane_id":"w1E:p3","command":"tether view session-refresh-e2237c"}]}
  ok   (a) a new tab about session refresh exists: w1E:t3 "Session refresh"
  ok   (a) it is recorded as tether-made in TETHER_STATE
       viewer pane command: tether view session-refresh-e2237c
  ok   (b) the new tab's pane runs `tether view <id>`
  ok   (c) .tether/diagrams/ has 1 diagram(s)
  ok   (c) the diagram is a flowchart
  ok   (c) the viewer id matches a stored diagram
       diagram "Session refresh": 13 nodes, 14 edges, 13 with code
   code refs: 13, exact 13, moved 0
  ok   (c) every node's code ref has an anchor that resolves to a real line
  ok   (c) render shows no stale links
  ok   (d) hand-made tab: same id, position and label
  ok   (d) hand-made tab: same panes, nothing typed into it
  ok   (d) hand-made tab: same screen text
  ok   (d) tab 1 is still the first tab
  ok   (e) tab 1 holds exactly one pane
  ok   (e) ... and it is the agent pane
  ok   focus not stolen (active tab is still tab 1)
       ---- the viewer ----
       |   Session refresh  flowchart · 13 nodes · 14 edges
       |
       |                                                                                                                                                      ┏━━━━━━━━━━━
       |                                                                                                                                                      ┃  ensureFr
       |                                                                                                                                                      ┗━━━━━━━━━━━
       |
       |
       |
       |                                                                                                                                                           ╭──────
       |                                                                                                                                                           │  Sig
       |                                                                                                                                                           ╰──────
       |                                                                                                                                                                │n
       |                                                                                                                                                  ╭─────────────╯
       |                                                                                                                                                  ▼
       |                                                                                                                                    ╭───────────────────────────╮
       |                                                                                                                                    │  Throw 'not signed in'  │
       |                                                                                                                                    ╰───────────────────────────╯
       |
       |
       |
       |                                                                                                                                                             ╭────
       |                                                                                                                                                             ││ r
       |                                                                                                                                                             ╰────
       |
       |                                                                                                                                                                 ╭
       |                                                                                                                                                                 ▼
# prompt 2: open the line where we check token expiry
  ok   agent finished prompt 2 (state done)
       nvim w1E:p4 in w1E:t3: /tmp/tether-e2e.sjCBOl/webapp/src/auth/session.ts:23  |   if (tokens.expiresAt < Date.now() + SKEW_MS) {
  ok   Neovim is open at session.ts:23 on the expiry check
  ok     ... not in tab 1 (w1E:t3)
  ok   tab 1 still holds only the agent pane
  ok   hand-made tab still untouched
# comment loop: tether send --to primary (as the Neovim hotkey would)
       tether send -> exit 0: sent to claude · tab 1 (w1E:p1, done)
  ok   tether send exit 0
       agent.wait -> done
       ---- agent's answer ----
       |
       | ⏺ The server only sends a relative expires_in, so refresh.ts:26 turns it into an absolute expiresAt using the client's Date.now(), and comparing against the same
       |   cost is that a clock that changes after the token was issued can make the refresh fire too early or too late.
       |
       | ✻ Cogitated for 2s · done 8:01 PM
       |
       |
       |
       |
       |
       |
       |
       |
       |
  ok   the agent's reply addresses the comment (mentions the client clock)
  ok   agent settled again (idle/done)

# teardown happens in the trap (claude killed, sandbox closed)
e2e: 27 passed, 0 failed  (model sonnet, transcripts in /tmp/e2e-f2)
closed w1E
```

## The agent's transcript (`agent.read`, run 6)

Taken after the comment loop, so it holds all three turns (the ghost suggestion claude shows in its empty input
box at the end is left out). The spec it generated has 13 nodes and 14 edges, all 13 nodes with exact code refs.

```
❯ make a flowchart of the session refresh logic
⏺ Skill(tether)
  ⎿  Successfully loaded skill
  Ran 3 shell commands
⏺ I made a flowchart of the session refresh logic. It opened in a new background tab called "Session refresh", and I left your tabs alone.
  In the viewer, j/k follow the edges and Enter on a node opens its code. Tab expands the extra notes.
  The flow:
  - Entry: ensureFreshSession() (src/auth/session.ts:18) throws "not signed in" if there is no session.
  - Expiry check: if the token expires within 30s (SKEW_MS), it calls refreshWithRetry(). Otherwise it returns the current session.
  - Refresh call: POST /token/refresh (src/auth/refresh.ts). A 200 builds a new TokenPair, which is stored and returned.
  - Failure: a 401 or any other error goes to a give-up check.
    - Give up: a 401 (SessionExpiredError) or the third attempt clears the session and rethrows, which forces a fresh login.
    - Retry: otherwise it waits 200ms * 2^attempt and calls the endpoint again.
✻ Sautéed for 19s · done 8:01 PM
❯ open the line where we check token expiry
  Ran 1 shell command
⏺ I opened src/auth/session.ts:23 in a split next to the diagram, in the "Session refresh" tab.
  The line is if (tokens.expiresAt < Date.now() + SKEW_MS). It refreshes the token once it is within 30 seconds of expiring, using the client clock.
✻ Baked for 4s · done 8:01 PM
❯ [comment] src/auth/session.ts:23  (worktree: webapp)
    23 │   if (tokens.expiresAt < Date.now() + SKEW_MS) {
  why client clock here? answer in one sentence
⏺ The server only sends a relative expires_in, so refresh.ts:26 turns it into an absolute expiresAt using the client's Date.now(), and comparing against the same clock keeps the check self-consistent 
  cost is that a clock that changes after the token was issued can make the refresh fire too early or too late.
✻ Cogitated for 2s · done 8:01 PM
```
