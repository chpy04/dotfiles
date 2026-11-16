-- tether lives in this dotfiles repo at tether/ (next to nvim/). ~/.config/nvim is a
-- symlink to dotfiles/nvim, so resolve it to find the repo root.
local dir = vim.fs.normalize(vim.fn.resolve(vim.fn.stdpath("config")) .. "/../tether/nvim")

return {
	----------------------------------------------------------------------
	-- tether: line comments to agents in Herdr (<leader>ac / <leader>aC),
	-- saved diagrams (<leader>ad).
	-- Not lazy-loaded: it registers this Neovim's RPC socket at startup so
	-- `tether code open` can reuse it.
	----------------------------------------------------------------------
	{
		dir = dir,
		name = "tether",
		lazy = false,
		cond = vim.fn.isdirectory(dir) == 1,
		config = function()
			require("tether").setup()
		end,
	},
}
