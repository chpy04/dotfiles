-- Minimal init for headless plugin tests: nvim --headless -u nvim/tests/minimal_init.lua
-- When TETHER_TEST_FILE is set, that Lua file is run once Neovim is fully started.
local here = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h")
local root = vim.fn.fnamemodify(here, ":h") -- nvim/
vim.opt.runtimepath:prepend(root)
vim.opt.swapfile = false
vim.opt.shadafile = "NONE"
vim.g.mapleader = " "
vim.g.tether_auto_setup = false
vim.o.more = false
vim.o.cmdheight = 3

local test_file = vim.env.TETHER_TEST_FILE
if test_file and test_file ~= "" then
  vim.api.nvim_create_autocmd("VimEnter", {
    once = true,
    callback = function()
      vim.schedule(function()
        local ok, err = xpcall(dofile, debug.traceback, test_file)
        if not ok then
          io.stderr:write("TEST HARNESS ERROR: " .. tostring(err) .. "\n")
          vim.cmd("cquit 2")
        end
      end)
    end,
  })
end
