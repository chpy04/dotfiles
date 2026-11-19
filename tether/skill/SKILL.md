---
name: tether
description: Arrange the user's Herdr workspace and point them at code. Use when the user asks to show, open or jump to code (a file, a line, a function), to make or show a diagram, or to set up, split, move or tidy panes and tabs. Also use before opening anything in their terminal layout.
---

# tether: layout and code pointers

`tether` drives the user's Herdr workspace (tabs, panes) and the Neovim running in it. It only works inside Herdr (it reads `HERDR_WORKSPACE_ID`). Output is plain text; add `--json` to parse it.

## Rule zero: look first

Run `tether layout` before you create, move or close anything. The user arranges their own tabs and panes; you must not trample them.

```
$ tether layout
workspace w8 "LIN-482 login"
tab 1 "1" w8:t1  [focused, current, yours]
  w8:p1  agent claude/working  ·  primary  ·  focused  ·  <- you
tab 2 "my notes" w8:t2  [yours]
  w8:p2  zsh
tab 3 "session refresh" w8:t5  [tether-made]
  w8:p6  tail -f app.log
  w8:p7  nvim@/repo/src/auth/session.ts:42
```

`[yours]` = the user made it. `[tether-made]` = you (tether) made it. A pane showing `nvim@file:line` is a Neovim you can drive.

## Conventions (defaults)

- **Tab 1 is the primary agent.** Do not put visuals or code there.
- **Visuals get a new tab named after their subject** (`session refresh`, not `diagram` or `output`).
- **Code opens in a split to the right of the visual**, in that same tab. With no `--in`/`--from`, `tether code open` never splits the primary agent's tab: it reuses the Neovim in your most recent tether-made tab, or opens a new tab named after the file (`session.ts`) and starts Neovim there, and says which it did.
- **Never close, rename or rearrange a tab you did not create.** `tether` refuses (it also refuses to close or rename the tab you are running in); do not reach for `--force` unless the user explicitly says to.
- **Stay in your own workspace.** Do not pass `--workspace` unless the user asks you to act elsewhere.
- Do not steal focus. Tabs and splits open in the background; pass `--focus` only when the user should look at it right now.

## The user's words override the conventions

Explicit placement wins. "Put it next to the chat" means split beside the agent pane in tab 1 (`tether split right --pane <agent pane>` or `tether code open ... --in 1`; an explicit `--in` or `--from` is always honored, even for the primary agent's tab). "Open it in the tests tab" means `--in tests`. After an override, go back to the defaults for the next request.

## Commands

`<tab>` is a 1-based index, a tab id (`w8:t5`) or a label. Panes are ids like `w8:p6`.

```
tether layout [--json]                         # tabs, panes, commands, agents, Neovims
tether tab open "session refresh" [--focus] [-- cmd args...]
tether tab rename <tab> "new name"             # tether-made tabs only
tether tab focus <tab>
tether tab close <tab>                         # tether-made tabs only (--force overrides; don't)
tether split right|down [--ratio 0.5] [--pane <id>] [-- cmd args...]   # prints the new pane id
tether code open path:line[-end] [--in <tab>] [--from <pane>] [--focus] [--diff <rev>]
```

- `tether tab open` prints the tab id and its first pane id. Put long-running commands after `--`.
- `tether split` splits your own pane unless `--pane` is given. `--ratio` is the share the existing pane keeps.
- `tether code open src/auth/session.ts:142` resolves paths against the worktree root. It jumps the Neovim already in the target tab and only splits a new one, to the right of `--from` (default your pane), when the tab has none. It never opens a second Neovim in a tab. Default target tab (no `--in`, no `--from`): your tab, unless that is the primary agent's tab; then a Neovim in the most recent tether-made tab is reused, else a new tab named after the file is opened (tether-made, no focus stolen). `--in <tab>` or `--from <pane>` picks the tab explicitly and is never redirected. The output line says which pane, whether it was reused or created, and why. A range (`:142-150`) highlights those lines for a few seconds.
- `--diff <rev>` (optional, off by default) makes that Neovim compare against a git revision instead of the index: a branch (`main`, `origin/main`), tag, commit or `HEAD~1`. No extra windows: gitsigns signs and line highlights, hunk preview/navigation/reset (the user's `<leader>g` keys) and the neo-tree file tree's git colours all show changes relative to that rev, for every file the user opens in that Neovim, and a notification says which base is active. It lives only in that Neovim's memory (no config is touched) and lasts until the next `code open` without `--diff`, which restores the default, or one with another rev. Use it only when the user asks to see changes against something ("what changed vs main", reviewing a branch); otherwise open code normally, which also clears a base you set earlier. To compare against where a branch forked (PR-style), pass the merge base: `--diff $(git merge-base main HEAD)`. A bad rev exits with code 2 before any pane is touched.
- Refusals exit with code 6 and say why; read the message instead of retrying with `--force`.

Typical flows:

```
# "show me where we refresh the token"
tether layout
tether tab open "token refresh"                 # only if a visual tab is wanted
tether code open src/auth/session.ts:142 --in "token refresh"

# "show me how session.ts changed against main"
tether code open src/auth/session.ts:142 --diff main

# "open that next to the chat"
tether layout                                    # find the agent pane in tab 1
tether code open src/auth/session.ts:142 --in 1 --from w8:p1
```

## Point at code, don't describe it

The user understands things by reading code, with go-to-definition and references at hand. When you mention a function, a bug or a change, open it: `tether code open path:line` instead of pasting a long snippet or narrating the lines. Say one sentence about why that line matters, and let the code carry the rest. When several places matter, open the most important first and list the rest as `path:line` references the user can ask you to open. Keep refs relative to the worktree root.

## Diagrams

Make a diagram when the user wants to *understand how something works* and the answer is a shape (a branching decision flow, a conversation between components). Write a small JSON spec, hand it to `tether diagram create`, and a tab named after the diagram opens with an interactive viewer. The user moves through it with hjkl, presses Enter on a node to open its linked code in Neovim, and comments on any node straight to you. A diagram is a map into the code, not a picture, so **every node or message that corresponds to code gets a `code` ref**.

### Which type

- **flowchart**: control flow and decisions inside one piece of logic: "what happens when the token expires", a retry policy, a state machine, a request pipeline with branches. Nodes are steps, edges are transitions, `decision` nodes branch (label the edges `yes`/`no`).
- **sequence**: who talks to whom, in what order, across components or services: a login request through API, auth service and database; a webhook handshake; a cache fill. Use `participants`, ordered `messages`, `note`s, and `alt` / `opt` / `loop` / `par` groups for branches and repeats.
- If you are unsure: one function or module's internal logic is a flowchart; several actors exchanging messages is a sequence diagram. Prefer two small diagrams over one with 40 nodes (about 30 nodes is the comfortable maximum for a flowchart).
- Not a diagram: a list of files (use `tether code open` for each), a plain explanation, anything without structure.

### Always link the code, with an anchor

Every node, message or note that maps to code gets `"code": {"path", "line", "anchor"}`. `path` is relative to the worktree root, `line` is 1-based (optional `end_line` for a range). **Always add `anchor`**: a short, distinctive snippet copied from that line (`"token.expiresAt <"`, `"router.post('/login'"`). The viewer re-finds the line by its anchor when the code shifts, and marks the link *stale* (and refuses to jump to a wrong line) when the snippet is gone. Read the file first so that line numbers and anchors are real; never guess or count them by eye: run `grep -n "<anchor>" <file>` and use the line it prints, so `line` is exactly the line that contains the anchor text (for a multi-line statement, the line where the anchor text is). Only leave `code` out for elements that really have no code (an external system, "user clicks submit").

Optionally a link can carry `"diff": "<rev>"` (e.g. `"code": {"path": "src/auth/session.ts", "line": 142, "anchor": "token.expiresAt <", "diff": "main"}`): pressing Enter on it then opens the file with that git base, exactly like `tether code open --diff`. Add it only when the diagram is about a change (a PR walkthrough, "what this branch changed") or the user asks for it, and then put the same rev on **every** link of the diagram: the base applies to the whole Neovim and a link without `diff` resets it to the default. Leave it out otherwise. Prefer a stable rev (a branch name or a commit sha from `git merge-base`) over a relative one like `HEAD~1`, since a stored diagram is reopened later. Comments from such a node carry a `[diff vs <rev>]` marker after the ref.

### Put the detail in `details`

Keep labels short (a few words). Everything else goes into `details`: `[{"title": "Edge case", "body": "..."}]`. Good titles: `Edge case`, `Data model`, `Invariant`, `Failure mode`, `Perf`, `Why`. The viewer shows a collapsed `+N` badge; the user expands it with Tab, and the layout makes room. A linked element with `details` also shows its `CODE` ref in the panel. Edge cases, data shapes, side paths and caveats belong here, not in extra nodes.

### Schemas

JSON Schemas are published in the repo: `schema/flowchart.schema.json` and `schema/sequence.schema.json`. Every spec is validated against them plus semantic checks (unknown ids, duplicate ids, `end_line` before `line`, absolute or `..` paths). An invalid spec exits with code 2 and prints one `- /json/pointer: what is wrong` line per problem; fix those and run again. Unknown fields are rejected, so do not invent keys.

### Flowchart example

```json
{
  "type": "flowchart",
  "title": "Session refresh",
  "direction": "TB",
  "nodes": [
    { "id": "check", "label": "Token expired?", "shape": "decision",
      "code": { "path": "src/auth/session.ts", "line": 142, "anchor": "token.expiresAt <" },
      "details": [{ "title": "Edge case", "body": "Uses client clock; skew > 30s causes early refresh" }] },
    { "id": "refresh", "label": "Call /token/refresh", "shape": "io",
      "code": { "path": "src/auth/refresh.ts", "line": 18, "anchor": "export async function refresh" } },
    { "id": "ok", "label": "Continue request", "shape": "terminal" }
  ],
  "edges": [
    { "from": "check", "to": "refresh", "label": "yes" },
    { "from": "check", "to": "ok", "label": "no" },
    { "from": "refresh", "to": "ok", "style": "dashed" }
  ]
}
```

`shape`: `process` (default), `decision`, `terminal` (start/end), `io`, `subroutine`, `database`. `direction`: `TB` (top to bottom) or `LR` (left to right). **Always set it explicitly** and pick it on purpose, because the viewer's keys follow it (see below). Use `TB` for most flows, and for anything with several decisions or long chains (branches spread across the wide terminal, steps go down). Use `LR` for a short, mostly linear pipeline of stages (about 6 or fewer steps deep) with few branches, read left to right. When unsure, use `TB`. `style`: `solid` (default) or `dashed` (use for async, optional or fallback paths). Edge `label` is optional but use it on every branch out of a decision. Node `id`s are yours: short, stable, unique, and only letters, digits, `_`, `.` and `-` (sequence participant, message and note ids share one namespace).

### Sequence example

```json
{
  "type": "sequence",
  "title": "Login request",
  "participants": [
    { "id": "browser", "label": "Browser", "kind": "actor" },
    { "id": "api", "label": "API", "kind": "service" },
    { "id": "db", "label": "DB", "kind": "database" }
  ],
  "items": [
    { "type": "message", "from": "browser", "to": "api", "label": "POST /login",
      "code": { "path": "src/api/login.ts", "line": 14, "anchor": "router.post('/login'" } },
    { "type": "message", "from": "api", "to": "db", "label": "SELECT user",
      "code": { "path": "src/auth/user_repo.ts", "line": 56, "anchor": "SELECT * FROM users" },
      "details": [{ "title": "Data model", "body": "User { id, email, pw_hash, mfa }" }] },
    { "type": "message", "from": "db", "to": "api", "label": "user row", "kind": "return" },
    { "type": "note", "over": ["api"], "text": "bcrypt compare, cost 12" },
    { "type": "group", "kind": "alt", "label": "Credentials",
      "branches": [
        { "label": "valid", "items": [
          { "type": "message", "from": "api", "to": "browser", "label": "200 + cookie", "kind": "return" } ] },
        { "label": "invalid", "items": [
          { "type": "message", "from": "api", "to": "browser", "label": "401", "kind": "return" } ] }
      ] },
    { "type": "group", "kind": "opt", "label": "MFA enabled", "items": [
      { "type": "message", "from": "api", "to": "browser", "label": "302 /mfa" } ] }
  ]
}
```

`participants[].kind`: `actor`, `service`, `database`, `queue`, `external` (controls the icon and color). Message `kind`: `sync` (default), `async`, `return` (drawn dashed). `items` are in time order and are `message`, `note` (`over` lists one or more participant ids) or `group`. `alt` and `par` groups take `branches` (each with an optional `label` and `items`); `opt` and `loop` take `items`. Messages and notes get ids `m1`, `m2`, ... and `n1`, ... in document order unless you give an `id` (give one to anything you may want to refer to later).

### Commands

```
tether diagram list [--json] [--starred] [--all-repos]   # this repo's saved diagrams, newest first
tether diagram create spec.json [--open tab|split|here|none] [--pane <id>] [--focus] [--star] [--json]
tether diagram create - < spec.json            # spec on stdin
tether diagram open <id|title words> [--placement tab|split|here]   # reopen a saved diagram
tether diagram star <id> | unstar <id>         # starred diagrams are never pruned
tether diagram rename <id> <new title>         # the id stays
tether diagram show <id>                       # the stored spec as JSON
tether diagram update <id> spec.json           # replace the spec (validated), same id
tether diagram export <id> --mermaid           # Mermaid source on stdout
tether diagram export <id> --mermaid --compat  # plain `participant` lines for Mermaid before v11
tether diagram rm <id>...                      # (`delete` is an alias)
tether diagram path | prune [--dry-run]        # the repo's diagram folder | remove stale unstarred ones
tether view <id> [--placement tab|split|here]  # (re)open the viewer
```

- **Diagrams persist per repo**, not per worktree: they live outside the repo (`~/.local/share/tether/repos/<repo>-<hash>/`), shared by every worktree of the clone, and nothing is written into the repo or its `.gitignore`. Unstarred diagrams not opened or changed for 30 days (`[diagrams] keep_days`) are pruned automatically; starred ones never are. The user can browse them all with `tether diagrams` (a list with a live preview) or Space-a-d in Neovim.
- **Before creating a diagram, run `tether diagram list`.** If one on the same subject already exists, do not make a duplicate: revise it with `tether diagram update <id> spec.json` (then tell the user to reopen it), or, if the user just wants to see it, open it with `tether diagram open "<title words>" --placement tab` (an ambiguous title exits 2 and lists the candidates; use the id then).
- When the user asks to keep, save or pin a diagram ("keep this one", "save that for later"), `tether diagram star <id>` it, or pass `--star` to `create`.
- Simplest and most reliable: one command with the spec inline, no temp file: `tether diagram create - <<'EOF'` then the JSON, then `EOF` (quote the `'EOF'`). Never run a bare `cat` or any other command that waits on stdin before it: the Bash tool would hang.
- `create` validates, stores the spec in the repo's diagram library (see above) and prints the id. By default (`--open tab`, or the user's `[placement] diagram` config) it opens a **new tab named after the title** running the viewer, in the background; the tab is tether-made, so you may close or rename it later. The viewer pane closes when the user presses `q`.
- `--open split` puts the viewer in a split to the right of your pane (or `--pane <id>`): use it when the user says "next to the chat". `--open none` only stores it. `--open here` runs the viewer in your own pane: never do this from the primary agent's pane.
- Run `tether layout` first, like for any tab. Do not pass `--focus` unless the user should look right now.
- To revise a diagram the user is looking at, `tether diagram update <id> spec.json`, then tell them to reopen it (`tether view <id> --placement tab`); the open viewer shows the version it started with.
- If you only need the user to see an element, say its label and let them use the viewer; do not paste the spec back to them.

### What the user does in the viewer (so you can answer their comments)

In a flowchart the keys follow its `direction`: in `TB` `j`/`k` step forward/back along the arrows and `h`/`l` switch branch; in `LR` `l`/`h` step forward/back and `j`/`k` switch branch. `HJKL` move on screen either way. When you show a flowchart, tell the user its direction in one short line (e.g. "top-to-bottom: j/k follow the flow, h/l switch branch"); in a sequence diagram `j`/`k` move between messages and `h`/`l` between participants. Enter opens the linked code in the Neovim split to the right of the viewer (the same Neovim every time) and focuses it, Tab expands `details`, `/` searches, Space-a-c comments on the focused node to you. A comment from a node arrives as:

```
[comment] diagram "Session refresh" › node "Token expired?"  src/auth/session.ts:142  (worktree: feat-login)
  142 │ if (token.expiresAt < Date.now()) {
Why client clock instead of server time here?
```

It names the diagram, the element, the live code ref and the line. Answer about that element; open related code with `tether code open`. A `[stale link]` marker after the ref means the anchor no longer matches: the code moved or changed since you drew the diagram, so re-read it, answer from the current code and offer to `tether diagram update` the spec.
