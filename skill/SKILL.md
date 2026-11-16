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
- **Code opens in a split to the right of the visual**, in that same tab.
- **Never close, rename or rearrange a tab you did not create.** `tether` refuses; do not reach for `--force` unless the user explicitly says to.
- **Stay in your own workspace.** Do not pass `--workspace` unless the user asks you to act elsewhere.
- Do not steal focus. Tabs and splits open in the background; pass `--focus` only when the user should look at it right now.

## The user's words override the conventions

Explicit placement wins. "Put it next to the chat" means split beside the agent pane in tab 1 (`tether split right --pane <agent pane>` or `tether code open ... --in 1`). "Open it in the tests tab" means `--in tests`. After an override, go back to the defaults for the next request.

## Commands

`<tab>` is a 1-based index, a tab id (`w8:t5`) or a label. Panes are ids like `w8:p6`.

```
tether layout [--json]                         # tabs, panes, commands, agents, Neovims
tether tab open "session refresh" [--focus] [-- cmd args...]
tether tab rename <tab> "new name"             # tether-made tabs only
tether tab focus <tab>
tether tab close <tab>                         # tether-made tabs only (--force overrides; don't)
tether split right|down [--ratio 0.5] [--pane <id>] [-- cmd args...]   # prints the new pane id
tether code open path:line[-end] [--in <tab>] [--from <pane>] [--focus]
```

- `tether tab open` prints the tab id and its first pane id. Put long-running commands after `--`.
- `tether split` splits your own pane unless `--pane` is given. `--ratio` is the share the existing pane keeps.
- `tether code open src/auth/session.ts:142` resolves paths against the worktree root. It jumps the Neovim already in the target tab (default: your tab; `--in` picks another), and only splits a new one, to the right of `--from` (default your pane), when the tab has none. It never opens a second Neovim in a tab. Output says which pane and whether it was reused. A range (`:142-150`) highlights those lines for a few seconds.
- Refusals exit with code 6 and say why; read the message instead of retrying with `--force`.

Typical flows:

```
# "show me where we refresh the token"
tether layout
tether tab open "token refresh"                 # only if a visual tab is wanted
tether code open src/auth/session.ts:142 --in "token refresh"

# "open that next to the chat"
tether layout                                    # find the agent pane in tab 1
tether code open src/auth/session.ts:142 --in 1 --from w8:p1
```

## Point at code, don't describe it

The user understands things by reading code, with go-to-definition and references at hand. When you mention a function, a bug or a change, open it: `tether code open path:line` instead of pasting a long snippet or narrating the lines. Say one sentence about why that line matters, and let the code carry the rest. When several places matter, open the most important first and list the rest as `path:line` references the user can ask you to open. Keep refs relative to the worktree root.

## Diagrams

(Placeholder, arrives with Phase 3.) `tether diagram create` will validate a spec and `tether view` will show it in a tab named after its subject, following the conventions above. Until then, do not try to draw diagrams in tabs; explain in text and point at code.
