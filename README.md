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
(`dir = "~/devex/nvim"`). `scripts/install.sh --dry-run` shows what it would do.

## Commands

All of them work inside a Herdr pane (they read `HERDR_WORKSPACE_ID`) and add `--json` where it helps.

| Command | What it does |
| --- | --- |
| `tether layout` | Tabs, panes, commands, agent states and Neovims of this workspace. Run it before changing anything. |
| `tether tab open <name> [-- cmd]` / `rename` / `focus` / `close` | Manage tabs. Rename and close only touch tabs tether made. |
| `tether split right\|down [--ratio r] [--pane id] [-- cmd]` | Split a pane and run a command in it. |
| `tether code open path:line[-end] [--in tab] [--from pane]` | Open or reuse Neovim at a line (never a second Neovim in a tab). |
| `tether send [--to primary\|pane\|agent] --ref path:line --text ...` | Send a comment with the line's code to an agent. `--diagram <id> --node <id>` comments on a diagram element. |
| `tether agents` | Agents in this workspace with their states. |
| `tether diagram create spec.json \| -` | Validate and store a diagram, open it in a new tab. Also `list`, `show`, `update`, `export --mermaid`, `render`, `delete`. |
| `tether view <id> [--placement tab\|split\|here]` | Open the interactive viewer for a stored diagram. |
| `tether config show\|path\|init\|get` | Inspect or create the config file. |

Diagram specs are JSON, validated against `schema/flowchart.schema.json` and `schema/sequence.schema.json`,
stored under `.tether/diagrams/` in the worktree. The agent-facing documentation is `skill/SKILL.md`.

## Keymaps

Neovim (set by the plugin, remappable under `[keys]`):

| Key | Action |
| --- | --- |
| `<leader>ac` | Comment on the current line (or visual range) to the primary agent. Enter sends, Esc cancels, Shift-Enter adds a line. |
| `<leader>aC` | Same, after picking the target agent (`vim.ui.select`). |

Diagram viewer (`tether view`):

| Key | Flowchart | Sequence diagram |
| --- | --- | --- |
| `j` / `k` | follow an edge / back to the parent | next / previous message |
| `h` / `l` | previous / next sibling or branch | previous / next participant |
| `Enter` | open the linked code in Neovim (reuses the same one; stale links refuse to jump) | same |
| `Tab` | expand / collapse details | same, for a message or note |
| `<leader>ac` / `<leader>aC` | comment on the node to the primary / a picked agent | same |
| `/`, `n`, `N` | search labels, next, previous | same |
| `g` / `G`, `Ctrl-d` / `Ctrl-u`, `?`, `q` | first / last, half page, help, quit | same |

Moving the focus never retargets Neovim; only `Enter` does.

## Config

`~/.config/tether/config.toml` (`TETHER_CONFIG` overrides the path). A missing file means defaults;
`tether config init` writes a commented one, `tether config show` prints the effective values. Sections:
`[keys]`, `[comment]` (payload template, `max_code_lines`, `max_code_line_width`), `[placement]`, `[view]`
(icons, mouse), `[theme]` (colors, Catppuccin Mocha by default), `[nvim]`. State (primary agent, tabs tether
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
