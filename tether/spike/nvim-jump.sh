#!/usr/bin/env bash
# Jump a --listen'ed Neovim to file:line from outside.   nvim-jump.sh <sock> <file> <line>
# Uses --remote-expr + luaeval (synchronous, returns the resulting "file:line" so callers can verify).
set -euo pipefail
sock="$1"; file="$2"; line="$3"
case "$file" in /*) ;; *) file="$PWD/$file";; esac
vfile=${file//\'/\'\'}   # vimscript single-quote escaping
lua='(function(a)
  local f = vim.fn.resolve(vim.fn.fnamemodify(a[1], ":p"))
  local b = vim.fn.bufnr(f)
  local w = b > 0 and vim.fn.bufwinid(b) or -1
  if w > 0 then vim.api.nvim_set_current_win(w)
  else vim.cmd("hide edit " .. vim.fn.fnameescape(f)) end
  local n = math.max(1, math.min(a[2], vim.api.nvim_buf_line_count(0)))
  vim.api.nvim_win_set_cursor(0, { n, 0 })
  vim.cmd("normal! zz")
  return vim.fn.expand("%:p") .. ":" .. n
end)(_A)'
nvim --server "$sock" --remote-expr "luaeval('${lua//\'/\'\'}', ['$vfile', $line])"
