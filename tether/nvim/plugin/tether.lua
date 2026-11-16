-- tether Neovim plugin entry: user commands + automatic setup() (disable with
-- `vim.g.tether_auto_setup = false` and call require("tether").setup(opts) yourself) + socket
-- registration inside Herdr (disable with `vim.g.tether_auto_register = false`), so a plain `nvim`
-- is found by `tether code open` and need not be started through bin/tether-nvim.
if vim.g.loaded_tether then
  return
end
vim.g.loaded_tether = true

vim.api.nvim_create_user_command("TetherComment", function(o)
  local range = o.range > 0 and { o.line1, o.line2 } or nil
  require("tether").comment({ range = range })
end, { range = true, desc = "tether: comment on this line/range to the primary agent" })

vim.api.nvim_create_user_command("TetherCommentPick", function(o)
  local range = o.range > 0 and { o.line1, o.line2 } or nil
  require("tether").comment_pick({ range = range })
end, { range = true, desc = "tether: comment on this line/range to a picked agent" })

vim.api.nvim_create_user_command("TetherDiagrams", function()
  require("tether").diagrams()
end, { desc = "tether: pick a saved diagram of this repo and open it in a Herdr tab" })

if vim.g.tether_auto_register ~= false then
  require("tether.server").setup()
end

if vim.g.tether_auto_setup ~= false then
  -- scheduled so an explicit require("tether").setup(opts) in the user's config wins
  vim.schedule(function()
    local t = require("tether")
    if not t._setup_done then
      t.setup()
    end
  end)
end
