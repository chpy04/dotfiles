# tether Claude Code skill

`SKILL.md` teaches Claude Code the layout conventions and the `tether` commands
(`layout`, `tab`, `split`, `code open`). The skill is not installed automatically.

## Install

Symlink this directory into your personal skills directory so edits here take effect immediately:

```sh
mkdir -p ~/.claude/skills
ln -s "$(pwd)/skill" ~/.claude/skills/tether
```

Run it from the repository root. Start a new Claude Code session afterwards. To uninstall, remove the
symlink (`rm ~/.claude/skills/tether`); the repository files are not touched.

## Requirements

- `tether` on `PATH` (for example `cargo install --path crates/tether`, or symlink `target/release/tether`).
- Claude Code running inside a Herdr pane, so `HERDR_WORKSPACE_ID` is set.
- `bin/tether-nvim` is found next to the repository the binary was built from. Otherwise put it on `PATH`,
  or point `TETHER_NVIM_WRAPPER` at it.
