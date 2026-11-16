return {
	----------------------------------------------------------------------
	-- tether: line comments to agents in Herdr (<leader>ac / <leader>aC)
	-- Not lazy-loaded: it registers this Neovim's RPC socket at startup so
	-- `tether code open` can reuse it.
	----------------------------------------------------------------------
	{
		dir = vim.fn.expand("~/devex/nvim"),
		name = "tether",
		lazy = false,
		config = function()
			require("tether").setup()
		end,
	},
}
