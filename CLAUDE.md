# Dotfiles

This repo is the source of truth for this Mac's dev setup. Live config files are symlinks into it.

## Rule: change it here

- Edit config in this repo, never at the link target, and never in a copy outside the repo.
- If a tool writes config somewhere else (app settings, a new file in `~/.config`), move it in here and add a `link_file` line to `install_links` in `install.sh`.
- New packages go in `brew/Brewfile`, not ad-hoc `brew install`.
- If something truly can't live here (secrets, runtime state, non-brew installs), say so and note it in `README.md`.
- After changing links, run `./install.sh --dry-run`, then `./install.sh`. Commit when done.

## Stack

| Tool | Repo path |
|------|-----------|
| zsh + Powerlevel10k (no oh-my-zsh) | `zsh/` |
| Git | `git/gitconfig` |
| Neovim (lazy.nvim, Mason; plugins pinned in `lazy-lock.json`) | `nvim/` |
| Ghostty | `ghostty/config` |
| herdr (prefix `ctrl-s`, replaces tmux) | `herdr/config.toml` |
| AeroSpace | `aerospace/aerospace.toml` |
| Karabiner (caps: esc tap / ctrl hold) | `karabiner/` (whole dir linked) |
| Homebrew | `brew/Brewfile` |

## Setup notes

- `./install.sh --brew` installs the Brewfile, then symlinks everything. Existing files are moved to `*.backup.<timestamp>`.
- Karabiner rewrites its config in place, so link its directory, not the file.
- herdr isn't in Homebrew; its binary lives in `~/.local/bin`.
- Never commit secrets: `~/.ssh`, `~/.config/gh/hosts.yml`, tokens.
- macOS only.
