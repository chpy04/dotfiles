# Keymaps Reference

Leader: `Space` | Local leader: `\`

---

## Essential Vim Motions & Commands

### Movement

| Key | Description |
|-----|-------------|
| `h` `j` `k` `l` | Left, down, up, right |
| `w` `b` | Next word / previous word |
| `e` `ge` | End of word / end of previous word |
| `W` `B` `E` | Same as above but by WORD (whitespace-delimited) |
| `0` `$` | Start / end of line |
| `^` | First non-blank character |
| `f{c}` `F{c}` | Jump to next / previous char `{c}` on line |
| `t{c}` `T{c}` | Jump to before next / after previous char `{c}` |
| `;` `,` | Repeat last `f`/`t` forward / backward |
| `gg` `G` | Top / bottom of file |
| `{` `}` | Previous / next blank line (paragraph) |
| `%` | Jump to matching bracket |
| `H` `M` `L` | Top / middle / bottom of screen |
| `Ctrl-d` `Ctrl-u` | Half-page down / up (centered via neoscroll) |
| `Ctrl-f` `Ctrl-b` | Full page down / up |
| `Ctrl-o` `Ctrl-i` | Jump list back / forward |
| `gd` | Go to local definition |
| `gD` | Go to global declaration |

### Editing

| Key | Description |
|-----|-------------|
| `i` `a` | Insert before / after cursor |
| `I` `A` | Insert at start / end of line |
| `o` `O` | New line below / above |
| `r{c}` | Replace single character with `{c}` |
| `R` | Enter replace mode |
| `x` | Delete character under cursor |
| `dd` `yy` `cc` | Delete / yank / change entire line |
| `D` `C` | Delete / change to end of line |
| `J` | Join line below to current |
| `~` | Toggle case of character |
| `gu` `gU` | Lowercase / uppercase (with motion) |
| `.` | Repeat last change |
| `u` `Ctrl-r` | Undo / redo |
| `p` `P` | Paste after / before cursor |
| `"_d` | Delete without yanking (black hole register) |
| `"+y` `"+p` | Yank to / paste from system clipboard |

### Text Objects (use with `d`, `c`, `y`, `v`, etc.)

| Key | Description |
|-----|-------------|
| `iw` `aw` | Inner word / a word (includes space) |
| `i"` `a"` | Inner quotes / around quotes |
| `i(` `a(` | Inner parens / around parens |
| `i{` `a{` | Inner braces / around braces |
| `i[` `a[` | Inner brackets / around brackets |
| `it` `at` | Inner tag / around tag (HTML/XML) |
| `ip` `ap` | Inner paragraph / around paragraph |
| `is` `as` | Inner sentence / around sentence |

### Visual Mode

| Key | Description |
|-----|-------------|
| `v` | Character-wise visual |
| `V` | Line-wise visual |
| `Ctrl-v` | Block visual |
| `gv` | Reselect last visual selection |
| `>` `<` | Indent / unindent selection (reselects) |
| `=` | Auto-indent selection |

### Search & Substitute

| Key | Description |
|-----|-------------|
| `/pattern` `?pattern` | Search forward / backward |
| `n` `N` | Next / previous match (centered) |
| `*` `#` | Search word under cursor forward / backward |
| `:s/old/new/g` | Substitute in current line |
| `:%s/old/new/gc` | Substitute in file with confirm |

### Marks & Registers

| Key | Description |
|-----|-------------|
| `ma` | Set mark `a` |
| `'a` `` `a `` | Jump to mark `a` (line / exact) |
| `''` | Jump to last position before jump |
| `"ay` `"ap` | Yank into / paste from register `a` |
| `:reg` | View all registers |

### Macros

| Key | Description |
|-----|-------------|
| `qa` | Record macro into register `a` |
| `q` | Stop recording |
| `@a` | Play macro `a` |
| `@@` | Replay last macro |

### Windows & Tabs

| Key | Description |
|-----|-------------|
| `Ctrl-w s` | Horizontal split |
| `Ctrl-w v` | Vertical split |
| `Ctrl-w h/j/k/l` | Navigate between splits |
| `Ctrl-w =` | Equalize split sizes |
| `Ctrl-w q` | Close split |
| `:tabnew` | New tab |
| `gt` `gT` | Next / previous tab |

---

## Custom Keymaps

### General (`config/keymaps.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>sp` | n | Toggle spell check |
| `<leader>y` | v | Yank to system clipboard |
| `<leader>x` | v | Cut to system clipboard |
| `<leader>/` | n | Clear search highlight |
| `<leader>t` | n | Open terminal (below) |
| `<leader>sj` | n | Toggle j/k vs gj/gk (visual lines) |
| `<leader>st` | n | Theme picker |
| `<leader>sr` | n | Search & replace (global, prompts) |
| `<leader>sq` | n | Search & replace (quickfix list) |
| `Esc Esc` | t | Exit terminal insert mode |
| `Ctrl-h` | n | Move to left split |
| `Ctrl-j` | n | Move to below split |
| `Ctrl-k` | n | Move to above split |
| `Ctrl-l` | n | Move to right split |

### Neo-tree (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>nn` | n | Reveal file in Neo-tree |
| `<leader>nt` | n | Toggle Neo-tree |
| `O` | n | Open file externally (inside Neo-tree) |

### Todo Comments (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `]t` | n | Jump to next TODO comment |
| `[t` | n | Jump to prev TODO comment |
| `<leader>fT` | n | Find TODO comments (Telescope) |

### Treesitter Text Objects (`plugins/treesitter.lua`)

#### Select (use with `v`, `d`, `c`, `y`)

| Key | Description |
|-----|-------------|
| `af` / `if` | Outer / inner function |
| `ac` / `ic` | Outer / inner class |
| `aa` / `ia` | Outer / inner argument |
| `ai` / `ii` | Outer / inner conditional |
| `al` / `il` | Outer / inner loop |

#### Move

| Key | Description |
|-----|-------------|
| `]m` / `[m` | Next / prev function start |
| `]M` / `[M` | Next / prev function end |
| `]]` / `[[` | Next / prev class start |
| `][` / `[]` | Next / prev class end |
| `]a` / `[a` | Next / prev argument |

#### Swap

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>ma` | n | Swap argument with next |
| `<leader>mA` | n | Swap argument with prev |

### Telescope (`plugins/telescope.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>ff` | n | Find files |
| `<leader>fg` | n | Live grep |
| `<leader>fb` | n | Buffers |
| `<leader>fo` | n | Old files (recent) |
| `<leader>fw` | n | Grep word under cursor |
| `<leader>fh` | n | Help tags |
| `<leader>fk` | n | Keymaps |
| `<leader>fc` | n | Commands |
| `<leader>fC` | n | Command history |
| `<leader>fs` | n | Fuzzy find in current buffer |
| `<leader>fm` | n | Marks |
| `<leader>fj` | n | Jumplist |
| `<leader>fr` | n | Registers |
| `<leader>ft` | n | Treesitter symbols |
| `<leader>f/` | n | Search history |
| `<leader>fq` | n | Quickfix list |
| `<leader>fl` | n | Location list |
| `<leader>fa` | n | Find files across project dirs |
| `<leader>gtc` | n | Git commits (Telescope) |
| `<leader>gts` | n | Git status (Telescope) |
| `<leader>gtB` | n | Git branches (Telescope) |

### LSP (`plugins/lsp.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>lf` | n | Format file (conform) |
| `<leader>ltc` | n | Toggle completion on/off |
| `<leader>lef` | n | Diagnostics float |
| `<leader>lD` | n | Diagnostics list (Telescope) |

### Lint (`plugins/lint.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>ll` | n | Run linters |

### Completion (nvim-cmp, insert mode)

| Key | Mode | Description |
|-----|------|-------------|
| `Ctrl-p` | i | Select previous item |
| `Ctrl-n` | i | Select next item |
| `Ctrl-y` | i | Confirm selection |
| `Ctrl-Space` | i | Trigger completion |

### Git (`plugins/git.lua`)

#### Gitsigns

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>gp` | n | Preview hunk |
| `<leader>gP` | n | Preview hunk inline |
| `]g` | n | Next hunk |
| `[g` | n | Previous hunk |
| `<leader>gs` | n | Stage hunk |
| `<leader>gs` | v | Stage hunk (visual) |
| `<leader>gr` | n | Reset hunk |
| `<leader>gr` | v | Reset hunk (visual) |
| `<leader>gS` | n | Stage buffer |
| `<leader>gR` | n | Reset buffer |
| `<leader>gu` | n | Undo stage hunk |
| `<leader>gb` | n | Toggle line blame |
| `<leader>gT` | n | Toggle git signs |

#### Copilot

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>act` | n | Toggle Copilot |
| `<leader>acd` | n | Disable Copilot |
| `<leader>ace` | n | Enable Copilot |
| `Alt-y` | i | Accept suggestion |
| `Alt-]` | i | Next suggestion |
| `Alt-[` | i | Previous suggestion |
| `Ctrl-]` | i | Dismiss suggestion |

#### LazyGit

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>gg` | n | Open LazyGit |

#### Diffview

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>gdo` | n | Open Diffview |
| `<leader>gdc` | n | Close Diffview |
| `<leader>gdh` | n | File history (all) |
| `<leader>gdf` | n | File history (current file) |
| `<leader>gdt` | n | Toggle file panel |
| `<leader>dco` | n | Conflict: choose ours |
| `<leader>dct` | n | Conflict: choose theirs |
| `<leader>dcb` | n | Conflict: choose base |
| `<leader>dca` | n | Conflict: choose all |
| `dx` | n | Conflict: delete region |

### Harpoon (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>ha` | n | Add file |
| `<leader>hd` | n | Remove file |
| `<leader>hh` | n | Quick menu |
| `<leader>h1-4` | n | Jump to file 1-4 |
| `<leader>hp` | n | Previous file |
| `<leader>hn` | n | Next file |

### Surround (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>ps` | n | Add surround |
| `<leader>pS` | n | Add surround (line) |
| `<leader>pd` | n | Delete surround |
| `<leader>pc` | n | Change surround |
| `<leader>pC` | n | Change surround (surrounding) |
| `<leader>ps` | v | Add surround (visual) |

### Windows (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>wm` | n | Maximize window |

### Markdown (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>mm` | n | Toggle Markview (in-editor markdown) |
| `<leader>mf` | n | Mkdnflow: follow link |
| `<leader>mF` | n | Mkdnflow: destroy link |
| `<leader>mc` | n/v | Mkdnflow: create link |
| `<leader>mC` | n/v | Mkdnflow: link from clipboard |
| `<leader>mn` | n | Mkdnflow: next link |
| `<leader>mp` | n | Mkdnflow: prev link |
| `<leader>mb` | n | Mkdnflow: back |
| `<leader>mB` | n | Mkdnflow: forward |
| `<leader>mt` | n/v | Mkdnflow: toggle todo |

### Other (`plugins/tools.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `<leader>u` | n | Toggle undotree |
| `<leader>?l` | n | Which-key: buffer local keymaps |
| `<leader>?g` | n | Which-key: global keymaps |

### Snippets (`plugins/snippets.lua`)

| Key | Mode | Description |
|-----|------|-------------|
| `Ctrl-j` | i/s | Expand or jump forward |
| `Ctrl-k` | i/s | Jump backward |

---

## Which-key Groups

| Prefix | Group |
|--------|-------|
| `<leader>a` | AI|
| `<leader>f` | Find |
| `<leader>g` | Git |
| `<leader>h` | Harpoon |
| `<leader>l` | LSP |
| `<leader>m` | Markdown |
| `<leader>n` | Neo-tree |
| `<leader>p` | Pairs/Surround |
| `<leader>s` | Settings |
| `<leader>w` | Windows |
| `<leader>?` | Whichkey |
