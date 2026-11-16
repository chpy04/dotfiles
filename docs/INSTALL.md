# Installing tether

tether has four parts. The first three install with one script; the Neovim plugin needs one
line in your Neovim config.

| Part | Where it ends up |
|---|---|
| `tether` CLI (release build) | `~/.local/bin/tether` (symlink to `target/release/tether`) |
| `tether-nvim` wrapper (optional, see below) | `~/.local/bin/tether-nvim` (symlink to `bin/tether-nvim`) |
| Claude Code skill | `~/.claude/skills/tether` (symlink to `skill/`) |
| Neovim plugin | loaded from this checkout by your plugin manager (`nvim/`) |

Requirements: Rust (`source ~/.cargo/env`), Herdr 0.9+, Neovim 0.10+ (developed on 0.12), Claude Code
running inside a Herdr pane. `~/.local/bin` must be on your `PATH`.

## 1. CLI, wrapper and skill

```sh
cd ~/devex
scripts/install.sh --dry-run     # show what would happen
scripts/install.sh               # cargo build --release, then symlink
```

The script is idempotent (run it again after `git pull`; it rebuilds and leaves correct links alone).
It never overwrites a real file or directory: if something is in the way it prints what and changes
nothing. `scripts/install.sh --uninstall` removes only the symlinks it points into this checkout
(your config and state files stay). Start a new Claude Code session afterwards so it sees the skill.

Check it:

```sh
tether --version && tether config show      # from any shell
# inside a Herdr pane:
tether layout
```

## 2. Neovim plugin

lazy.nvim spec (for the dotfiles setup this is a new file `nvim/lua/plugins/tether.lua`, picked up by
`{ import = "plugins" }`):

```lua
return {
  {
    dir = "~/devex/nvim",
    name = "tether",
    config = function()
      require("tether").setup()
    end,
  },
}
```

Do not lazy-load it (`keys=`/`cmd=`/`event=`): `plugin/tether.lua` has to run at startup so the Neovim
registers itself with tether (next section).

`setup()` accepts options; all are optional:

```lua
require("tether").setup({
  bin = "/path/to/tether",             -- default: the `tether` on PATH, else <checkout>/target/release|debug/tether
  keys = { comment = "<leader>ac", comment_pick = "<leader>aC" },
  float = { width = 64, max_height = 10, border = "rounded" },
})
```

Startup is not blocked: the keys are mapped immediately from these defaults plus your options, and only
inside Herdr does the plugin read `tether config show --json` in the background (remapping if your tether
config defines other keys).

### How `tether code open` finds your Neovim

Inside Herdr (`HERDR_ENV=1`, `HERDR_PANE_ID` set) the plugin makes the running Neovim listen on
`<runtime>/tether/<pane id>.sock` (runtime = `$XDG_RUNTIME_DIR`, else `$TMPDIR`, else `/tmp`). So a plain
`nvim` you start by hand is found and reused by `tether code open`, with no wrapper. Details:

- A stale socket file left by a crashed Neovim is removed first; a socket that a live Neovim already
  owns is never taken over.
- If macOS purges `$TMPDIR` while Neovim is running, the plugin listens again on the next
  `FocusGained`/`VimResume`.
- Opt out with `vim.g.tether_auto_register = false` before the plugin loads.
- `bin/tether-nvim` still works (it passes `--listen` itself) and is what tether uses when it has to start
  a Neovim in a new split.

## 3. Keys

Defaults: `<leader>ac` comments on the current line or visual range to the primary agent, `<leader>aC`
picks the agent first. If your Neovim config already uses `<leader>ac…` as a prefix (the dotfiles
Copilot keys `<leader>act`, `<leader>acd`, `<leader>ace` do), Neovim waits `timeoutlen` after
`<leader>ac` before it fires the comment. Remap tether in `~/.config/tether/config.toml`:

```toml
[keys]
comment = "<leader>ak"
comment_pick = "<leader>aK"
```

(or pass `keys = { ... }` to `setup()`, which wins over the file). Run `tether config init` to write a
commented config.

## 4. Where tether keeps things

| What | Where |
|---|---|
| Config | `~/.config/tether/config.toml` (`TETHER_CONFIG` overrides) |
| State (primary agent tag, tabs tether made) | `$XDG_STATE_HOME/tether/state.json`, default `~/.local/state/tether/state.json` (`TETHER_STATE` overrides). Updates are serialized with a lock file next to it. A corrupt file is backed up to `state.json.corrupt-<time>`. |
| Neovim sockets | `<runtime>/tether/<pane>.sock` (not state: they may be purged) |

State used to live in the runtime dir; the old file is not migrated (tether just re-learns the primary
agent from the first send).

## 5. Dotfiles integration

`~/dotfiles` uses lazy.nvim with one file per area under `nvim/lua/plugins/`, `~/.local/bin` already on
`PATH` (`zsh/zshrc`), and `nvim-cmp`, `nvim-autopairs` and `copilot.lua`. To integrate (done by hand; tether's
installer never touches the dotfiles):

1. Add `nvim/lua/plugins/tether.lua` with the spec above, so every machine that checks out the dotfiles
   and `~/devex` gets the plugin.
2. Optionally add to `dotfiles/install.sh` a call to `~/devex/scripts/install.sh` (it is safe to repeat),
   or run it once by hand per machine.
3. The comment box (filetype `tetherinput`) switches off nvim-cmp, blink.cmp and Copilot for itself, and
   re-claims `<CR>` on `InsertEnter`, so autopairs and completion keymaps do not swallow Enter.
4. Add the `[keys]` remap above if `<leader>ac` is to stay Copilot's prefix.
5. Optionally list the tether keys in `dotfiles/nvim/keymaps.md` (`<leader>ac`/`<leader>aC`, or your
   remapped ones).
