# dotfiles

macOS dev environment: AeroSpace, Karabiner, Ghostty, herdr, Neovim, zsh, and Homebrew.
Vi keys everywhere, tiling windows, terminal-first.

| Tool | Config | Linked to |
|------|--------|-----------|
| zsh + Powerlevel10k | [zsh/zshrc](zsh/zshrc), [zsh/zprofile](zsh/zprofile), [zsh/p10k.zsh](zsh/p10k.zsh) | `~/.zshrc`, `~/.zprofile`, `~/.p10k.zsh` |
| Git | [git/gitconfig](git/gitconfig) | `~/.gitconfig` |
| Neovim | [nvim/](nvim) ([keymaps](nvim/keymaps.md)) | `~/.config/nvim` |
| Ghostty | [ghostty/config](ghostty/config) | `~/.config/ghostty/config` |
| herdr | [herdr/config.toml](herdr/config.toml) | `~/.config/herdr/config.toml` |
| AeroSpace | [aerospace/aerospace.toml](aerospace/aerospace.toml) | `~/.aerospace.toml` |
| Karabiner-Elements | [karabiner/](karabiner) | `~/.config/karabiner` (whole dir) |
| Homebrew | [brew/Brewfile](brew/Brewfile) | — |

## New machine

```bash
# 1. Homebrew
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# 2. This repo
git clone git@github.com:chpy04/dotfiles.git ~/dotfiles
cd ~/dotfiles

# 3. Packages + symlinks (existing files are moved to *.backup.<timestamp>)
./install.sh --dry-run   # preview
./install.sh --brew
```

Then:

- **herdr** isn't in Homebrew; install it separately (binary goes in `~/.local/bin`).
- **Powerlevel10k**: prompt config is tracked in `zsh/p10k.zsh`; run `p10k configure` to change it.
- **Neovim**: first launch installs plugins at the versions pinned in `nvim/lazy-lock.json` (lazy.nvim) and LSP servers/formatters (Mason).
- **AeroSpace / Karabiner**: grant Accessibility / Input Monitoring permissions when macOS asks.

## Keys at a glance

- **Caps Lock**: Escape on tap, Control on hold (Karabiner)
- **AeroSpace** (`alt`): `h/j/k/l` focus, `shift-h/j/k/l` move, `1-9`/`a-z` workspace,
  `shift-<ws>` send window, `tab` last workspace, `shift-;` service mode
- **herdr**: prefix `ctrl-s`
- **zsh**: vi mode (cursor changes shape), `ctrl-r` fzf history, `ctrl-t` fzf files, `alt-y` accept suggestion
- **Neovim**: leader `space`; see [nvim/keymaps.md](nvim/keymaps.md)

## Adding things

- New brew package: add it to `brew/Brewfile`, then `brew bundle --file=~/dotfiles/brew/Brewfile`.
- New config: put it in a folder here and add a `link_file` line to `install_links` in `install.sh`.

Originally forked from [ekremarmagankarakas/dotfiles](https://github.com/ekremarmagankarakas/dotfiles).
