# tether — PRD and implementation brief (Herdr + Neovim + Claude Code)

> **For the implementation agent.** This is the product spec for a personal dev environment. "I" and "me" in this document mean the user, who works in Herdr, Neovim, git worktrees and Claude Code, on Ghostty. Build in the order given in **Phasing**. Each phase has an exit criterion; do not start the next phase until it passes. Requirements are numbered (F1.x, F2.x, F3.x) so you can reference them in commits and tests. Where the spec left a choice open, **Implementation defaults** picks one. Follow it unless you find a concrete reason not to, and tell the user when you deviate.

## Overview

A personal dev environment built on Herdr and Neovim where the code is the main way I work with agents. Agents arrange my workspace for me. I can point at any line of code, or any node in a diagram, and send an agent a question without leaving what I'm reading.

**Problem.** Today I switch between two modes that don't connect. Either I read code with little help from the agent, or I read a wall of agent text with no code behind it. Even when there are snippets, I can't go to definition, find references or move around the way I would in an editor. Reviewing agent changes means scrolling a long diff instead of working in the file tree.

**Who it's for.** Me, in my setup. It's opinionated on purpose and isn't meant to be portable.

**v1 goals**

- Agents can create and arrange Herdr tabs and panes to show me code, diagrams and output.
- From any Neovim buffer, one hotkey sends a comment tied to that line to an agent. I don't have to wait for it.
- Agents can make flowcharts and sequence diagrams linked to code. I can move through them with hjkl, open the linked code in Neovim and comment on any node.

**Not in v1:** other editors or multiplexers; a chat UI inside Neovim; showing agent replies in Neovim; sending slash commands, skills or plugin invocations (comments are plain text only); end-to-end workflows. Workflows get built on these tools later.

## Principles

1. **Code is how I understand things.** Agent output should point into the code, not stand in for it. When there's a choice, show me the line instead of describing it.
2. **The agent belongs in the reading loop.** I ask questions where they come up, while I'm reading, and the answers lead back to code I can navigate.
3. **Keep my muscle memory.** Herdr workspaces and tabs, Neovim motions and LSP all keep working as they do today. The tool adds to them and replaces nothing.
4. **The agent drives, I can always step in.** The agent sets up layouts, I can rearrange them whenever I want, and the tool copes with my manual changes.
5. **Sending never blocks reading.** Comments are fire-and-forget. I check the agent's answers when I choose to.
6. **Built for one setup.** Herdr, Neovim, git worktrees and Claude Code. Opinionated over portable.
7. **One config.** Hotkeys, defaults and placement rules all live in one config file.

## Environment and core concepts

Each task (a ticket or feature) is one Herdr workspace, ideally tied to one git worktree. Everything else is addressed relative to that workspace. The tool is called **tether** (placeholder name).

| Concept | Maps to | Notes |
| --- | --- | --- |
| Task | Herdr workspace | One per ticket or feature. Ideally one worktree per workspace. |
| Primary agent | Agent pane in tab 1 | Default target for comments. |
| Agent | Any pane where Herdr detects an agent | Shows up in the target picker along with its state. |
| View tab | Herdr tab an agent creates | Holds a diagram, code, logs, etc. Named after its subject. |
| Code ref | `path:line[-endLine]`, relative to the worktree root | The pointer that every feature passes around. |
| Comment | Text + a code ref or a diagram-node ref | Sent to an agent pane and submitted there. |
| Diagram | Structured spec of nodes, edges, optional code refs and details | Rendered by a terminal viewer in a pane. |

**What Herdr already provides.** Herdr has a CLI and a local socket API that speaks newline-delimited JSON (default socket `~/.config/herdr/herdr.sock`; requests look like `{"id":"req_1","method":"ping","params":{}}`). Relevant methods:

- Workspaces and tabs: `workspace.create/list/get/focus/rename/close`, `tab.create/list/get/focus/rename/close`
- Panes: `pane.split` (direction `right` or `down`, optional `ratio`), `pane.focus_direction`, `pane.resize`, `pane.swap`, `pane.zoom`, `pane.list`, `pane.get`, `pane.current`
- Input and output: `pane.send_text`, `pane.send_keys` (e.g. `enter`), `pane.send_input`, `pane.read`, `pane.wait_for_output`
- Agents: `agent.list`, `agent.get`, `agent.read`, `agent.wait`, `agent.prompt` (submits a prompt and starts a wait in one call). Agent states are `idle`, `working`, `blocked`, `done`, `unknown`.
- Events: `events.subscribe` (e.g. `pane.agent_status_changed`), `events.wait`, `session.snapshot`
- Every managed pane gets `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, `HERDR_SOCKET_PATH` and `HERDR_ENV=1`, so any process can find its own workspace.

Docs: https://herdr.dev/tui/docs/socket-api/ — verify method names and params against the installed Herdr version before relying on them.

**Plugin limits.** A Herdr plugin (`herdr-plugin.toml`) can declare actions, keybindings and event hooks, and can launch commands in panes as an overlay, popup, split, tab or zoomed pane. It can't draw native, non-terminal UI, and it has no storage API (https://herdr.dev/tui/docs/plugins/). So every visual is a terminal program running in a pane, and tether keeps its own state on disk.

## Feature 1 — Agent-controlled workspace layout

Claude gets a small set of layout tools plus a short layout philosophy, and arranges its own workspace with them. Herdr already has every primitive needed. This feature is mostly a thin wrapper and a prompt.

**Requirements**

- **F1.1** The agent can create, rename, focus and close tabs in its own workspace.
- **F1.2** The agent can split a pane right or down at a given ratio and run a command in the new pane.
- **F1.3** The agent can open `file:line` in Neovim. If the target tab already has a Neovim pane it reuses it, and only splits when there isn't one.
- **F1.4** The agent can read the current layout (tabs, panes, what each one is running, agent states) before changing anything, so it doesn't overwrite my manual setup.
- **F1.5** By default the agent only acts inside its own workspace (`HERDR_WORKSPACE_ID`). It touches other workspaces only when I ask.
- **F1.6** Layout conventions live in a Claude Code skill. Tab 1 is the primary agent. Visuals open in a new tab named after their subject. Code opens in a split to the right of the visual. Never close a tab the agent didn't create.
- **F1.7** Any placement can be overridden in my request, e.g. "put it next to the chat".

**Agent-facing commands (draft)**

| Command | What it does | Herdr primitives |
| --- | --- | --- |
| `tether layout` | Prints this workspace's tabs, panes, commands and agent states | `session.snapshot`, `pane.list`, `agent.list` |
| `tether tab open <name> [-- cmd]` | Creates a tab and runs a command in it | `tab.create`, `pane.send_input` |
| `tether split <right\|down> [--ratio] [-- cmd]` | Splits the current or given pane | `pane.split` |
| `tether code open <path:line> [--in <tab>]` | Opens or reuses Neovim at that line | `pane.split` + Neovim RPC |
| `tether view <diagram-id> [--placement tab\|split\|here]` | Shows a diagram (Feature 3) | `tab.create` / `pane.split` |

**Acceptance.** I say "make a flowchart of the session refresh logic". A tab named after it appears with the diagram in it, and I don't touch the layout. Tabs I made by hand stay where they were.

## Feature 2 — Line comments from Neovim to agents

One hotkey opens a floating input under the cursor line. Enter sends the text, along with a reference to that line, to the primary agent and submits it. I keep reading. This is the core loop for asking questions while I read and for reviewing agent changes in the file tree instead of a diff scroll.

**Flow**

1. I press `<leader>ac` (configurable). A bordered float opens anchored just below the current line, titled `→ claude · tab 1`. In visual mode, the comment covers the selected range.
2. I type. Enter sends, Esc cancels, Shift-Enter adds a newline.
3. tether finds the primary agent in the current workspace and delivers the message, which starts the agent.
4. A short notification confirms it was sent. The float closes and the cursor stays where it was.
5. `<leader>aC` opens a picker of every agent in the workspace instead (tab name, pane, state). I pick one, then the same float opens.

**Requirements**

- **F2.1** It works in any Neovim inside a Herdr workspace, including ones an agent opened. Outside Herdr it shows a clear error.
- **F2.2** The input is clearly separate from the code: a floating window with a border and its own highlight, drawn on top of the code. No virtual lines or inline rendering tricks.
- **F2.3** Sending is fire-and-forget. I can send several comments in a row, even while the agent is working. tether sends each one as normal input and relies on Claude Code to queue it, the same way it queues typed text. Do not build a queue in tether unless this proves unreliable.
- **F2.4** The payload includes the line text, so the agent has context without opening the file and the message reads well in the agent's scrollback:

```text
[comment] src/auth/session.ts:142  (worktree: feat-login)
  142 │ if (token.expiresAt < Date.now()) {
Why client clock instead of server time here?
```

- **F2.5** The picker is single-select. It shows each agent's Herdr state so I can avoid one that's `blocked`.
- **F2.6** All hotkeys and the payload template can be set in tether's config. Only plain text is sent: no slash commands, skills or plugin invocations.

**Later (v1.1 candidates, do not build now):** a sign-column marker on commented lines, a review batch mode that collects comments and sends them together, a jump from the agent's answer back to the line.

## Feature 3 — Code-linked diagrams

The agent writes a structured diagram spec. A terminal viewer draws it in a Herdr pane. I move through it with hjkl, press Enter to open a node's linked code in a Neovim split, and use the same comment hotkeys as in Neovim. v1 supports **flowcharts** and **sequence diagrams**.

**Spec: our own JSON, exportable to Mermaid**

Plain Mermaid has nowhere to put code links or expandable details. The agent writes a small JSON spec through a tool call. It's deterministic and easy to validate, and Mermaid can still be exported when needed. Code links are optional on every node and message. The skill tells the agent to add them whenever the code exists.

```json
{
  "type": "flowchart",
  "title": "Session refresh",
  "nodes": [
    { "id": "check", "label": "Token expired?", "shape": "decision",
      "code": { "path": "src/auth/session.ts", "line": 142, "anchor": "token.expiresAt <" },
      "details": [{ "title": "Edge case", "body": "Uses client clock; skew > 30s causes early refresh" }] },
    { "id": "refresh", "label": "Call /token/refresh",
      "code": { "path": "src/auth/refresh.ts", "line": 18 } },
    { "id": "ok", "label": "Continue request" }
  ],
  "edges": [
    { "from": "check", "to": "refresh", "label": "yes" },
    { "from": "check", "to": "ok", "label": "no" }
  ]
}
```

Sequence diagrams use the same idea: `participants`, ordered `messages` (each can have a `code` link and `details`), `notes`, and `alt`/`opt`/`loop` groups. `details` holds data models, edge cases and side paths. It shows as a collapsed `+N` badge until expanded. Publish a JSON Schema for both diagram types and validate every spec against it.

**Navigation**

| Key | Flowchart | Sequence diagram |
| --- | --- | --- |
| `j` / `k` | Follow an outgoing edge / go back to the parent | Next / previous message |
| `h` / `l` | Move between siblings, or pick a branch at a decision | Previous / next participant (lifeline) |
| `Enter` | Open linked code in the split on the right | Same |
| `Tab` | Expand or collapse the node's details | Same, for a message or note |
| `<leader>ac` / `<leader>aC` | Comment on the node to the primary agent / to a picked agent | Same |
| `/`, `n` | Search labels | Same |
| `q` | Close the viewer | Same |

**Requirements**

- **F3.1** By default a diagram opens in a new tab named after its title. The agent can place it as a split next to the chat when I ask.
- **F3.2** The first `Enter` on a linked node splits the tab vertically and opens Neovim at that line. Each later `Enter` reuses the same Neovim. It never opens a second one.
- **F3.3** A comment from a node includes the diagram title, the node label and the code ref if there is one.
- **F3.4** Code links store an `anchor` snippet as well as a line number, so the link can be re-found after the code shifts.
- **F3.5** It has to look polished, not like plain ASCII: rounded borders, true-color fills, icons, colors that follow my theme, a clearly highlighted focused node, edge labels, and no overlapping lines at typical sizes (around 30 nodes).
- **F3.6** Diagrams are saved and can be reopened (`tether view <id>`) for as long as the workspace exists.

**Rendering: a viewer where every node is an object (no images)**

The viewer is an interactive terminal app, not a picture. Every node, edge, message, note and details panel is an object with an id, a position, a focus state, an expanded state and an optional code ref. hjkl, comments and expansion all act on those objects directly. Do not render diagrams as images (no Kitty graphics, Sixel or rasterized SVG).

1. **Layout.** A graph layout engine places objects on a grid of terminal cells. Flowcharts use a layered layout with orthogonal edge routing (Graphviz `dot -Tplain` for positions to start, a native Rust layout later). Sequence diagrams use a simple fixed layout: one column per participant, one row per message.
2. **Drawing.** Each object draws itself with cells: rounded box-drawing borders, true-color fills, bold labels, Nerd Font icons for shapes and badges, and colored edges with arrowheads.
3. **Focus.** The focused object gets the accent fill and a heavy border. Other objects dim slightly so the focused path stands out. The viewport scrolls to keep the focus in view.
4. **Expansion.** Tab opens a details panel attached to the object. The layout runs again and nearby objects move to make room.
5. **Comments.** The comment hotkey opens a float styled like the Neovim one, anchored under the focused object, and sends through tether.

**Framework: Rust + ratatui (decided).** The viewer, tether and anything added later share one Rust codebase, which keeps future integrations simple. ratatui redraws the whole frame each time, so the viewer owns its object model (ids, positions, focus, expansion) and draws every frame from it.

## Reference projects

Nothing existing does the whole loop. Use these as references; do not add them as dependencies unless noted.

| Need | Project | How to use it |
| --- | --- | --- |
| Send line context to an agent | [sidekick.nvim](https://github.com/folke/sidekick.nvim) — sends the current line or selection to AI CLIs, with tmux and zellij backends | Borrow its context formatting for our Neovim plugin. |
| Send line context to an agent | [claudecode.nvim](https://github.com/coder/claudecode.nvim) — implements Claude Code's IDE WebSocket protocol | Built around one Neovim and the one Claude it manages. Reference only. |
| Send line context to an agent | [send-to-agent.nvim](https://github.com/code-inflation/send-to-agent.nvim) — sends `@file#L1-5` references to agents in tmux panes | Close in spirit to F2. Reference only. |
| Agent-made diagrams in a TUI | [Nereid](https://github.com/bnomei/nereid) — ratatui TUI plus an MCP server for flowcharts and sequence diagrams | Closest prior art for Feature 3. **Noncommercial source-available license: study its navigation and MCP tool design, do not copy code or fork it.** |
| Drawing diagrams in terminal cells | [merman](https://github.com/kitlangton/merman) (TypeScript, MIT) | Model for rounded, themed diagram drawing. Reference only. |

## Architecture

A single **tether CLI** sits at the center. The Neovim plugin, the diagram viewer and Claude all call it, and only it talks to Herdr's socket and to Neovim's RPC sockets. That keeps the integration logic in one place and lets each piece be tested from a shell.

```
  Neovim plugin        Diagram viewer         Claude Code
  (comment float,      (hjkl, Enter,          (skill; calls the
   picker)              comments)              CLI via Bash)
        \                   |                     /
         \                  |                    /
          +---------> tether CLI (Rust) <-------+
                 resolve targets · send ·
                 open code · diagrams
          /                 |                    \
   Herdr socket API     Neovim RPC          Diagram store
   (tabs, panes,        (one --listen       (validated JSON
    agents)              socket per pane)    specs)
```

The callers never reach Herdr or Neovim directly. Swapping a back end, or working around a Herdr API change, only touches tether.

**Components**

| Component | Role | Stack |
| --- | --- | --- |
| tether CLI | Resolves workspace, agents and Neovim panes. Sends comments, opens code, stores and validates diagrams | Rust, single binary |
| Neovim plugin | Hotkeys, floating input, agent picker. Shells out to `tether send` | Lua |
| Diagram viewer | Terminal UI that lays out and draws a spec and handles keys. Calls `tether code open` and `tether send` | Rust, ratatui with custom widgets (can be a `tether view` subcommand) |
| Claude integration | A skill documenting tether's commands and layout conventions. Claude runs them through Bash | `SKILL.md` |
| Herdr plugin (optional) | Herdr-level keybindings and event hooks, e.g. set up tab 1 when a workspace is created | `herdr-plugin.toml` |

**Key flows**

1. **Comment.** Neovim → `tether send --to primary --ref src/x.ts:42 --text …` → read `HERDR_WORKSPACE_ID` → `agent.list` → deliver to the agent's pane (`agent.prompt`, or `pane.send_text` + Enter).
2. **Open code.** Viewer → `tether code open src/x.ts:42` → look for a registered Neovim in this tab → if there is one, tell it over RPC to jump to the line; if not, `pane.split right` running `nvim +42 src/x.ts`, then register it.
3. **Diagram.** Claude → `tether diagram create spec.json` → validate and store → `tab.create` running `tether view <id>`.

**Technical notes**

- **Delivering text into Claude Code.** Send as normal input. Try `agent.prompt` first; fall back to `pane.send_text` followed by a separate Enter. Multi-line text can trip over bracketed paste and submit timing, which is why Phase 0 tests it.
- **Finding Neovim instances.** Launch Neovim through a wrapper that adds `--listen $XDG_RUNTIME_DIR/tether/$HERDR_PANE_ID.sock`. Pane to Neovim lookup is then deterministic and needs no process scraping.
- **Primary agent.** Tabs can be reordered. Tag the first agent pane tether sees in a workspace as primary, and fall back to the agent in tab 1.
- **Flowchart layout** is the hardest piece. Start with Graphviz (`dot -Tplain`) for node positions and draw boxes and orthogonal edges ourselves; replace with a native Rust layered layout later if needed. Re-run layout whenever a node expands.
- **Keeping links fresh.** Re-resolve each link by searching for its `anchor` near the stored line. If not found, mark the link stale in the viewer and do not jump to the wrong line.
- **Moving target.** Herdr is young and has announced a separate desktop app on a new runtime. Pin a Herdr version and build against the terminal app's socket API.

## Mockups (described)

The PRD doc has drawn versions of these. They're rough sketches, not final designs.

**A — Task workspace.** Herdr sidebar on the left lists workspaces (`LIN-482 login` active) and the agents in the current one with states (`tab 1 claude · idle`, `tab 3 claude · working`). The tab bar shows `1 claude`, `2 session refresh` (active), `3 tests`, `4 shell`. Tab 2 is split vertically: the diagram viewer on the left shows `Request arrives → ◇ Token expired? → (yes) Refresh token / (no) Continue`. The decision node is focused (accent fill, heavy border) with a `+1` details badge. A dashed accent link runs from it to the right pane, which is Neovim on `src/auth/session.ts` with line 142 highlighted. The viewer's footer shows key hints: `hjkl move · ⏎ code · tab details · ␣ac ask`. Moving to another node retargets the same Neovim.

**B — Comment float and agent picker.** In Neovim, line 142 is followed by a bordered accent float, drawn on top of the code (lines below are covered, not pushed down). It's titled `→ claude · tab 1`, contains the typed question with a caret, and has the footer `⏎ send · esc cancel`. The `<leader>aC` picker is a bordered list titled `Send comment to` with rows `tab 1 · claude — idle` (highlighted), `tab 3 · claude — working`, `tab 5 · codex — blocked`, with each state in its own color (green, amber, red).

**C — Sequence diagram with expanded details.** Participants `Browser`, `API`, `AuthSvc`, `DB` sit in rounded boxes with dashed lifelines. Messages: `POST /login`, then `verify(email, pw)` (focused: accent arrow, bold label, `⏎ code` hint), then `SELECT user`, then dashed returns `session` and `200 + cookie`. Under the focused message an accent-bordered details panel shows `DATA MODEL User { id, email, pw_hash, mfa }`, `EDGE CASE locked accounts return 423, not 401`, `CODE src/auth/verify.ts:31`. Lifelines break around the panel and later messages move down. Footer: `j/k message · h/l participant · ⏎ code · tab details · ␣ac ask`.

## Implementation defaults

These were left open in the PRD. Use these unless you have a concrete reason to change them, and tell the user if you do.

| Question | Default |
| --- | --- |
| Primary agent identity | Tagged pane (first agent tether sees in the workspace), falling back to tab 1. |
| Diagram spec format | Our own JSON with a published JSON Schema, plus a Mermaid export. |
| Claude interface | CLI + `SKILL.md` only. No MCP server in v1. |
| Diagram storage | Inside the worktree at `.tether/diagrams/` (add `.tether/` to `.gitignore`). |
| Neovim picker | `vim.ui.select`, so it uses whatever picker the user's config has. |
| Neovim plugin | Our own small plugin (not a sidekick.nvim backend), borrowing sidekick's context formatting. |
| Task bootstrap command | Not in v1. |
| Config file | One TOML file at `~/.config/tether/config.toml` (hotkeys, payload template, placement defaults). The Neovim plugin reads hotkeys from it. |
| Suggested repo layout | A Cargo workspace: `crates/tether-core` (Herdr client, Neovim RPC, spec types, config), `crates/tether` (CLI + `view` subcommand). Plus `nvim/` (Lua plugin), `skill/SKILL.md`, and optionally `herdr-plugin/`. |

## Phasing

Build the comment loop first. It's the smallest piece, useful on its own, and proves the Herdr and Neovim plumbing everything else uses.

1. **Phase 0 — Spike.** From a shell script, send a multi-line string into the tab-1 Claude pane over the Herdr socket and submit it. Also launch Neovim with `--listen` and jump it to a line over RPC. *Exit: both work reliably 20 times in a row.*
2. **Phase 1 — Comments.** `tether send`, the Neovim float, the agent picker and the config. *Exit: I review a real agent PR entirely in Neovim using comments.*
3. **Phase 2 — Layout tools.** `tether layout / tab / split / code open` plus the Claude skill. *Exit: the agent opens code at a line in a split when I ask, without touching my tabs.*
4. **Phase 3 — Flowcharts.** Spec and validation, layout, object rendering, focus and expansion, viewer, hjkl navigation, code links and comments from nodes. *Exit: I navigate an agent-made flowchart of real code and jump to every linked node.*
5. **Phase 4 — Sequence diagrams and details.** Sequence layout, expandable details, link re-resolution, visual polish.
6. **Phase 5 — Workflows.** Build on the tools: review flow, task bootstrap, "explain this module".

**Out of scope:** other editors and terminals, chat inside Neovim, showing agent replies in Neovim, slash commands or skills from comments, sharing with a team, and diagram types beyond flowcharts and sequence diagrams.

## Sources

- [Herdr socket API](https://herdr.dev/tui/docs/socket-api/)
- [Herdr plugins](https://herdr.dev/tui/docs/plugins/)
- [Herdr docs](https://herdr.dev/docs/)
- [A deep dive into Herdr — Flavio Copes](https://flaviocopes.com/herdr/)
- [sidekick.nvim](https://github.com/folke/sidekick.nvim)
- [claudecode.nvim](https://github.com/coder/claudecode.nvim)
- [send-to-agent.nvim](https://github.com/code-inflation/send-to-agent.nvim)
- [Nereid](https://github.com/bnomei/nereid)
- [merman](https://github.com/kitlangton/merman)
