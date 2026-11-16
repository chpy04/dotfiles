# tether — build plan and rules for implementing agents

Source spec: `tether-PRD.md` (repo root). Requirement IDs (F1.x, F2.x, F3.x) come from there.
Reference them in commit messages and test names.

## Hard safety rules (every agent must follow)

1. **Never send input to any pane in workspace `w7`**, and never to the pane in `$HERDR_PANE_ID` that you
   inherited from the orchestrator. `w7:p1` is the orchestrating Claude session; typing into it corrupts
   the build. All live Herdr testing happens in a **sandbox workspace** you create yourself, labeled
   `tether-sandbox-<something>`, and you close it when done (`workspace.close`).
2. When tether resolves "the current workspace" from env vars during tests, **override the env**
   (`HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`, `HERDR_PANE_ID`) to point at sandbox ids. Never run
   `tether send` with the inherited env.
3. Never close or rename workspaces, tabs or panes you did not create.
4. Don't edit `~/dotfiles`, `~/.config/nvim`, `~/.config/herdr/config.toml`, or anything outside this repo,
   except under `$XDG_RUNTIME_DIR`/`$TMPDIR` for sockets and scratch.
5. Don't `git commit`. The orchestrator commits per phase.
6. Rust toolchain: run `source ~/.cargo/env` before cargo. Graphviz `dot` is installed (16.1).

## Environment facts

- Tests must never touch the real diagram library: every test that can reach `DiagramStore` sets `TETHER_DATA` to a temp dir
  (core tests use `DiagramStore::open_at`). The data dir is `$TETHER_DATA`, else `$XDG_DATA_HOME/tether`, else `~/.local/share/tether`.

- Herdr 0.9.5, protocol 22. Full socket API schema: `docs/herdr/api-schema.json`
  (generated with `herdr api schema --json`). Herdr's own agent guide: `docs/herdr/herdr-skill.md`.
  The CLI (`herdr pane --help`, `herdr agent --help`, …) wraps the same API and returns JSON. It's a
  good way to explore, but tether itself talks to the socket directly (NDJSON at `$HERDR_SOCKET_PATH`).
- Neovim 0.12.6. Claude Code 2.1.x. macOS (BSD userland). `XDG_RUNTIME_DIR` may be unset, so fall back to
  `$TMPDIR`.

## Repo layout

```
Cargo.toml                    workspace
crates/tether-core/src/
  herdr.rs                    Herdr socket client (NDJSON request/response, typed helpers)
  nvim.rs                     Neovim RPC (msgpack-rpc over --listen socket) + socket path convention
  config.rs                   ~/.config/tether/config.toml (TETHER_CONFIG env overrides path)
  workspace.rs                resolve current workspace/tab/pane, agents, primary-agent tagging
  send.rs                     comment payload formatting + delivery
  diagram/                    spec types, JSON Schema, validation, store (per-repo library under $TETHER_DATA, index.json), mermaid export, anchors
  layout/                     flowchart (dot -Tplain) + sequence layout -> cell grid geometry (pure)
crates/tether/src/
  main.rs, cli/…              clap CLI
  view/…                      ratatui viewer (`tether view <id>`)
  browse/…                    ratatui diagram browser (`tether diagrams`): list + live preview via view's drawing
schema/                       published JSON Schemas (flowchart, sequence)
nvim/                         Lua plugin (plugin/tether.lua, lua/tether/*.lua)
bin/tether-nvim               Neovim wrapper adding --listen <runtime>/tether/<pane>.sock
skill/SKILL.md                Claude Code skill
spike/                        Phase 0 scripts + findings
scripts/sandbox.sh            create/teardown a sandbox Herdr workspace for live tests
herdr-plugin/                 optional
```

## Phases (PRD order) and automated stand-in exit gates

| Phase | Stand-in gate (automated) | Needs the user to try by hand |
|---|---|---|
| 0 Spike | multi-line delivery to an agent pane + nvim RPC jump, 20/20 in a sandbox | — |
| 1 Comments | `cargo test` passes; live: `tether send` delivers to the sandbox agent; headless nvim test of float/picker keymaps calls tether with the correct args | Review a real PR via comments |
| 2 Layout | live sandbox: layout/tab/split/code open, nvim reuse, no foreign tabs touched | Ask the agent to open code |
| 3 Flowcharts | schema validation tests, layout snapshot tests (~30 nodes, no overlaps), viewer render tests with ratatui TestBackend, live: Enter opens/reuses nvim | Navigate a real flowchart |
| 4 Sequence + details | same, for sequence diagrams; anchor re-resolution tests | Visual polish judgment |
