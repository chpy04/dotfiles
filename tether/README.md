# tether

Glue for a personal dev setup: **Herdr + Neovim + Claude Code**. The code is the main way you work with
agents. An agent arranges your Herdr workspace (tabs, splits, Neovim at a line), draws flowcharts and
sequence diagrams that link into the code, and you point at any line or diagram node and send the agent a
question without leaving what you are reading. Opinionated, built for one setup, not portable.

Spec: [`tether-PRD.md`](tether-PRD.md). Build plan and safety rules: [`docs/PLAN.md`](docs/PLAN.md).
Acceptance with a real Claude Code: [`docs/E2E.md`](docs/E2E.md).

## Install

See [`docs/INSTALL.md`](docs/INSTALL.md): `scripts/install.sh` builds the release binary and links the CLI and
the Claude Code skill into `~/.local/bin` and `~/.claude/skills`; the Neovim plugin is one lazy.nvim spec
(`dir = "<checkout>/nvim"`). `scripts/install.sh --dry-run` shows what it would do. In the dotfiles repo,
`./install.sh` at the repo root runs it for you and `nvim/lua/plugins/tether.lua` loads the plugin.

## Commands

All of them work inside a Herdr pane (they read `HERDR_WORKSPACE_ID`) and add `--json` where it helps.

| Command | What it does |
| --- | --- |
| `tether layout` | Tabs, panes, commands, agent states and Neovims of this workspace. Run it before changing anything. |
| `tether tab open <name> [-- cmd]` / `rename` / `focus` / `close` | Manage tabs. Rename and close only touch tabs tether made. |
| `tether split right\|down [--ratio r] [--pane id] [-- cmd]` | Split a pane and run a command in it. |
| `tether code open path:line[-end] [--in tab] [--from pane] [--diff rev]` | Open or reuse Neovim at a line (never a second Neovim in a tab). `--diff` makes that Neovim's gitsigns and neo-tree compare against a git revision, until the next open without it. |
| `tether send [--to primary\|pane\|agent] --ref path:line --text ...` | Send a comment with the line's code to an agent. `--diagram <id> --node <id>` comments on a diagram element. |
| `tether agents` | Agents in this workspace with their states. |
| `tether diagram create spec.json \| - [--star]` | Validate and save a diagram, open it in a new tab. Also `show`, `update`, `export --mermaid`, `render`. |
| `tether diagram list [--all-repos] [--starred] [--json]` | This repo's saved diagrams, newest first: ★, id, type, branch, age, title. |
| `tether diagram open <id\|title words> [--placement tab\|split\|here]` | Reopen a saved diagram; a partial, case-insensitive title is enough (several matches: they are listed, exit 2). |
| `tether diagram star\|unstar <id>`, `rename <id> <title>`, `rm <id>...` (alias `delete`), `path` | Manage saved diagrams. Starred ones are never pruned. |
| `tether diagram prune [--dry-run]` | Remove unstarred diagrams not opened or changed for `[diagrams] keep_days` (30) days. Also runs by itself at most once a day, on `create` and `list`. |
| `tether diagrams` (alias `tether diagram browse`) | Full-screen browser: list on the left, live preview on the right; open, star, rename, delete, filter, copy Mermaid. |
| `tether view <id> [--placement tab\|split\|here]` | Open the interactive viewer for a stored diagram. |
| `tether config show\|path\|init\|get` | Inspect or create the config file. |

Diagram specs are JSON, validated against `schema/flowchart.schema.json` and `schema/sequence.schema.json`,
saved per repo under `~/.local/share/tether/repos/<repo>-<hash>/diagrams/` (`TETHER_DATA` moves the root), shared by
every worktree of a clone, with an `index.json` holding titles, stars, branch and timestamps. Nothing is written
into the repo any more. Diagrams saved by older versions in `<worktree>/.tether/diagrams/` are imported once, on
first use (the old files stay; delete the folder when you like). The agent-facing documentation is `skill/SKILL.md`.

## Keymaps

Neovim (set by the plugin, remappable under `[keys]`):

| Key | Action |
| --- | --- |
| `<leader>ac` | Comment on the current line (or visual range) to the primary agent. Enter sends, Esc cancels, Shift-Enter adds a line. |
| `<leader>aC` | Same, after picking the target agent (`vim.ui.select`). |
| `<leader>ad` (`:TetherDiagrams`) | Pick one of this repo's saved diagrams (`★ Session refresh · flowchart · feat-login · 2d`) and open it in a new tether tab. `keys.diagrams` remaps it. |

Diagram viewer (`tether view`):

| Key | Flowchart | Sequence diagram |
| --- | --- | --- |
| along the flow: `j` / `k` (`TB`), `l` / `h` (`LR`) | step forward along the highlighted outgoing edge (the first branch by position, or the last one you chose at that node) / step back along the edge you arrived by (else the first parent) | `j`/`k` next / previous message, `h`/`l` previous / next participant |
| across the flow: `l` / `h` (`TB`), `j` / `k` (`LR`) | switch to the next / previous sibling branch (the other children of the node a step back returns to, by position, wrapping; roots switch between roots). The parent remembers the choice, so stepping back then forward comes back to it | see the row above |
| `H` `J` `K` `L` (and the arrow keys) | move on screen to the nearest node left / down / up / right (`J`/`K` keep their column), so every node is reachable by position | arrow keys, as `h` `j` `k` `l` above |
| `Ctrl-o` / `Ctrl-i` | jump list: back / forward over where the focus has been (up to 100 positions; `Ctrl-i` needs a terminal with the kitty keyboard protocol, others send it as `Tab`) | same |
| `Enter` | open the linked code in Neovim (reuses the same one; stale links refuse to jump) and focus that pane (`[view] focus_code = false` keeps focus on the viewer) | same |
| `Tab` | expand / collapse details | same, for a message or note |
| `<leader>ac` / `<leader>aC` | comment on the node to the primary / a picked agent | same |
| `/`, `n`, `N` | search labels, next, previous | same |
| `g` / `gg` / `G`, `Ctrl-d` / `Ctrl-u`, `?`, `q` | first / last, half page, help, quit | same |
| `s` | star / unstar this diagram (shown as ★ in the header; `view_star` remaps it) | same |

Flowchart keys follow the chart's `direction`: the key pair along the flow steps through it and the pair across it switches branch. A top-to-bottom (`TB`, the default) chart steps with `j`/`k` and switches branch with `h`/`l`; a left-to-right (`LR`) chart is the other way round. The footer shows the current pair. The highlighted outgoing edge of the focused node (where a forward step goes) is drawn bold in the accent color. Moving the focus never retargets Neovim; only `Enter` does.
The viewer keys are remappable under `[keys]`: `view_down`/`view_up`/`view_left`/`view_right` (sequence diagrams),
`view_flow_back`/`view_flow_forward`/`view_flow_branch_next`/`view_flow_branch_prev` (named for `LR` charts; a `TB` chart swaps the two pairs) and `view_flow_left`/`view_flow_down`/`view_flow_up`/`view_flow_right` (flowcharts), `view_jump_back`, `view_jump_forward`.

Diagram browser (`tether diagrams`):

| Key | Action |
| --- | --- |
| `j` / `k` (or arrows), `g` / `G` | move through the list |
| `Enter` | open in the full viewer here; `q` there returns to the browser |
| `t` | open in a new tether tab (`tether view <id> --placement tab`) and stay |
| `s` | star / unstar |
| `r` | rename (inline; the id stays) |
| `d` | delete, after `y/n` |
| `/` | filter by title or branch as you type (`Enter` keeps it, `Esc` clears it) |
| `a` | toggle all repos; `S` toggle starred only |
| `Ctrl-d` / `Ctrl-u` (`<` / `>`) | scroll (pan) the preview |
| `e` | copy the Mermaid source to the clipboard (`pbcopy`) |
| `?`, `q` | help, quit |

## Config

`~/.config/tether/config.toml` (`TETHER_CONFIG` overrides the path). A missing file means defaults;
`tether config init` writes a commented one, `tether config show` prints the effective values. Sections:
`[keys]`, `[comment]` (payload template, `max_code_lines`, `max_code_line_width`), `[placement]`, `[diagrams]` (`keep_days`), `[view]`
(icons, mouse, focus_code), `[theme]` (colors, Catppuccin Mocha by default), `[nvim]`. State (primary agent, tabs tether
made) lives in `~/.local/state/tether/state.json` (`TETHER_STATE`).

## Tests

```sh
source ~/.cargo/env
scripts/test-all.sh            # cargo test, clippy -D warnings, nvim/tests/run.sh, live phase 1-3 tests
scripts/test-all.sh --e2e      # ... plus the real-Claude end-to-end (a few model turns, a few cents)
```

The live tests (`scripts/test-phase{1,2,3}.sh`, `scripts/test-e2e.sh`) need Herdr running. They only touch
sandbox workspaces named `tether-sandbox-*` that they create and close themselves; they refuse to run
against the inherited workspace or pane. Individual gates: `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `nvim/tests/run.sh`.
