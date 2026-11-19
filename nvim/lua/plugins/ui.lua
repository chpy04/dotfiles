return {
	----------------------------------------------------------------------
	-- Install Themes
	----------------------------------------------------------------------
	{ "catppuccin/nvim", name = "catppuccin", lazy = true, priority = 1000 },
	{ "EdenEast/nightfox.nvim", lazy = true, priority = 1000 },
	{ "rebelot/kanagawa.nvim", lazy = true, priority = 1000 },
	{ "projekt0n/github-nvim-theme", name = "github-theme", lazy = true, priority = 1000 },
	{ "olimorris/onedarkpro.nvim", lazy = true, priority = 1000 },
	{ "Mofiqul/dracula.nvim", lazy = true, priority = 1000 },
	{ "folke/tokyonight.nvim", lazy = true, priority = 1000 },
	{ "Mofiqul/adwaita.nvim", lazy = true, priority = 1000 },
	{ "rose-pine/neovim", lazy = true, priority = 1000 },
	{ "sainnhe/gruvbox-material", lazy = true, priority = 1000 },
	{ "sainnhe/everforest", lazy = true, priority = 1000 },
	{ "scottmckendry/cyberdream.nvim", lazy = true, priority = 1000 },
	{ "Mofiqul/vscode.nvim", lazy = true, priority = 1000 },

	----------------------------------------------------------------------
	--  Neoscroll
	----------------------------------------------------------------------
	{
		"karb94/neoscroll.nvim",
		event = "VeryLazy",
		opts = {
			mappings = { "<C-u>", "<C-d>", "<C-b>", "<C-f>", "zt", "zz", "zb" },
			hide_cursor = true,
			stop_eof = true,
			respect_scrolloff = false,
			cursor_scrolls_alone = true,
		},
	},

	----------------------------------------------------------------------
	-- Lualine
	----------------------------------------------------------------------
	{
		"nvim-lualine/lualine.nvim",
		dependencies = { "nvim-tree/nvim-web-devicons" },
		opts = {
			options = {
				theme = "auto",
				globalstatus = true,
				component_separators = { left = "", right = "" },
				section_separators = { left = "", right = "" },
			},
			sections = {
				lualine_a = { "mode" },
				lualine_b = { "branch", "diff", "diagnostics" },
				lualine_c = {
					{ "filename", path = 1 },
				},
				lualine_x = {
					-- Diff base (<leader>gB); hidden on the default unstaged view
					{
						function()
							return require("config.diffbase").status()
						end,
						color = { fg = "#e0af68" },
					},
					-- Noice macro recording indicator
					{
						function()
							return require("noice").api.status.mode.get()
						end,
						cond = function()
							return package.loaded["noice"] and require("noice").api.status.mode.has()
						end,
						color = { fg = "#ff9e64" },
					},
					-- Noice search count
					{
						function()
							return require("noice").api.status.search.get()
						end,
						cond = function()
							return package.loaded["noice"] and require("noice").api.status.search.has()
						end,
						color = { fg = "#ff9e64" },
					},
					-- Copilot status
					{
						function()
							local ok, copilot = pcall(require, "copilot.api")
							if not ok then
								return ""
							end
							local status = copilot.status.data
							if status.status == "InProgress" then
								return " loading"
							elseif status.status == "Warning" then
								return " warning"
							end
							return ""
						end,
						cond = function()
							return package.loaded["copilot"] ~= nil
						end,
					},
					-- Active LSP servers
					{
						function()
							local clients = vim.lsp.get_clients({ bufnr = 0 })
							if #clients == 0 then
								return ""
							end
							local names = {}
							for _, client in ipairs(clients) do
								table.insert(names, client.name)
							end
							return " " .. table.concat(names, ", ")
						end,
						cond = function()
							return #vim.lsp.get_clients({ bufnr = 0 }) > 0
						end,
					},
				},
				lualine_y = { "filetype" },
				lualine_z = { "progress", "location" },
			},
		},
	},

	----------------------------------------------------------------------
	-- Indent Blankline
	----------------------------------------------------------------------
	{
		"lukas-reineke/indent-blankline.nvim",
		main = "ibl",
		---@module "ibl"
		---@type ibl.config
		opts = {
			scope = {
				enabled = true,
				show_start = true,
				show_end = false,
			},
		},
	},

	----------------------------------------------------------------------
	-- Nvim Colorizer
	----------------------------------------------------------------------
	{
		"NvChad/nvim-colorizer.lua",
		event = "BufReadPre",
		opts = {
			filetypes = { "*" },
			user_default_options = {
				RGB = true,
				RRGGBB = true,
				names = false, -- disable named colors (e.g. "Blue")
				RRGGBBAA = true,
				rgb_fn = true,
				hsl_fn = true,
				css = true,
				css_fn = true,
				mode = "background", -- "background" | "foreground" | "virtualtext"
				tailwind = false,
				virtualtext = "■",
			},
		},
	},

	----------------------------------------------------------------------
	-- Noice (UI for messages, cmdline, and notifications)
	----------------------------------------------------------------------
	{
		"folke/noice.nvim",
		event = "VeryLazy",
		dependencies = {
			"MunifTanjim/nui.nvim",
			"rcarriga/nvim-notify",
		},
		opts = {
			lsp = {
				-- Override LSP handlers to use Noice's UI
				override = {
					["vim.lsp.util.convert_input_to_markdown_lines"] = true,
					["vim.lsp.util.stylize_markdown"] = true,
					["cmp.entry.get_documentation"] = true,
				},
			},
			presets = {
				bottom_search = true, -- classic bottom search bar
				command_palette = true, -- cmdline and popupmenu at top center
				long_message_to_split = true, -- long messages go to a split
				lsp_doc_border = true, -- bordered LSP hover/signature docs
			},
		},
	},
}
