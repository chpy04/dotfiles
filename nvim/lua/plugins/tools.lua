return {
	----------------------------------------------------------------------
	-- Neo-Tree
	----------------------------------------------------------------------
	{
		"nvim-neo-tree/neo-tree.nvim",
		branch = "v3.x",
		dependencies = {
			"nvim-lua/plenary.nvim",
			"nvim-tree/nvim-web-devicons",
			"MunifTanjim/nui.nvim",
		},
		main = "neo-tree",
		keys = {
			{ "<leader>nn", "<cmd>Neotree filesystem reveal left<cr>", silent = true, desc = "Neo-tree: reveal" },
			{ "<leader>nt", "<cmd>Neotree toggle<cr>", silent = true, desc = "Neo-tree: toggle" },
		},
		opts = {
			default_component_configs = {
				diagnostics = {
					symbols = {
						hint = "󰌵",
						info = " ",
						warn = " ",
						error = " ",
					},
					highlights = {
						hint = "DiagnosticSignHint",
						info = "DiagnosticSignInfo",
						warn = "DiagnosticSignWarn",
						error = "DiagnosticSignError",
					},
				},
				git_status = {
					symbols = {
						added = "",
						modified = "",
						deleted = "✖",
						renamed = "󰁕",
						untracked = "",
						ignored = "",
						unstaged = "󰄱",
						staged = "",
						conflict = "",
					},
				},
			},
			filesystem = {
				use_libuv_file_watcher = true,
				filtered_items = {
					visible = true,
					hide_dotfiles = false,
					hide_gitignored = true,
				},
				components = {
					name = function(config, node, state)
						local result = require("neo-tree.sources.common.components").name(config, node, state)
						local diag = state.diagnostics_lookup or {}
						local severity = diag[node:get_id()]
						if severity == vim.diagnostic.severity.ERROR then
							result.highlight = "DiagnosticError"
						elseif severity == vim.diagnostic.severity.WARN then
							result.highlight = "DiagnosticWarn"
						end
						return result
					end,
				},
			},
			window = {
				mappings = {
					-- "O" for "Open Externally" (PDFs, Browsers, etc.)
					["O"] = function(state)
						local node = state.tree:get_node()
						local path = node:get_id()
						vim.ui.open(path)
					end,
				},
			},
		},
	},

	----------------------------------------------------------------------
	-- Todo Comments
	----------------------------------------------------------------------
	{
		"folke/todo-comments.nvim",
		event = "VeryLazy",
		dependencies = { "nvim-lua/plenary.nvim" },
		keys = {
			{
				"]t",
				function()
					require("todo-comments").jump_next()
				end,
				desc = "Todo: next comment",
			},
			{
				"[t",
				function()
					require("todo-comments").jump_prev()
				end,
				desc = "Todo: prev comment",
			},
			{ "<leader>fT", "<cmd>TodoTelescope<cr>", desc = "Find: todo comments" },
		},
		opts = {},
	},

	----------------------------------------------------------------------
	-- Flash (enhanced motions)
	----------------------------------------------------------------------
	{
		"folke/flash.nvim",
		event = "VeryLazy",
		keys = {
			{
				"s",
				mode = { "n", "x", "o" },
				function()
					require("flash").jump()
				end,
				desc = "Flash: jump",
			},
			{
				"S",
				mode = { "n", "x", "o" },
				function()
					require("flash").treesitter()
				end,
				desc = "Flash: treesitter",
			},
			{
				"r",
				mode = "o",
				function()
					require("flash").remote()
				end,
				desc = "Flash: remote",
			},
			{
				"R",
				mode = { "o", "x" },
				function()
					require("flash").treesitter_search()
				end,
				desc = "Flash: treesitter search",
			},
		},
		opts = {},
	},

	----------------------------------------------------------------------
	-- Harpoon
	----------------------------------------------------------------------
	{
		"ThePrimeagen/harpoon",
		branch = "harpoon2",
		dependencies = { "nvim-lua/plenary.nvim" },
		keys = {
			{
				"<leader>ha",
				function()
					require("harpoon"):list():add()
				end,
				desc = "Harpoon: add file",
			},
			{
				"<leader>hd",
				function()
					require("harpoon"):list():remove()
				end,
				desc = "Harpoon: remove file",
			},
			{
				"<leader>hh",
				function()
					local harpoon = require("harpoon")
					harpoon.ui:toggle_quick_menu(harpoon:list())
				end,
				desc = "Harpoon: quick menu",
			},
			{
				"<leader>h1",
				function()
					require("harpoon"):list():select(1)
				end,
				desc = "Harpoon: file 1",
			},
			{
				"<leader>h2",
				function()
					require("harpoon"):list():select(2)
				end,
				desc = "Harpoon: file 2",
			},
			{
				"<leader>h3",
				function()
					require("harpoon"):list():select(3)
				end,
				desc = "Harpoon: file 3",
			},
			{
				"<leader>h4",
				function()
					require("harpoon"):list():select(4)
				end,
				desc = "Harpoon: file 4",
			},
			{
				"<leader>hp",
				function()
					require("harpoon"):list():prev()
				end,
				desc = "Harpoon: prev",
			},
			{
				"<leader>hn",
				function()
					require("harpoon"):list():next()
				end,
				desc = "Harpoon: next",
			},
		},
		config = function()
			require("harpoon"):setup()
		end,
	},

	----------------------------------------------------------------------
	-- Sleuth - auto-detect indentation from file content
	----------------------------------------------------------------------
	{ "tpope/vim-sleuth", event = "BufReadPre" },

	----------------------------------------------------------------------
	-- Autopairs
	----------------------------------------------------------------------
	{
		"windwp/nvim-autopairs",
		event = "InsertEnter",
		config = function()
			require("nvim-autopairs").setup({
				check_ts = true, -- use treesitter to avoid pairing inside strings/comments
				map_cr = true, -- expand pairs on Enter
			})

			-- Integrate with nvim-cmp if present
			local ok_cmp, cmp = pcall(require, "cmp")
			if ok_cmp then
				local ok_ap, cmp_autopairs = pcall(require, "nvim-autopairs.completion.cmp")
				if ok_ap then
					cmp.event:on("confirm_done", cmp_autopairs.on_confirm_done())
				end
			end
		end,
	},

	----------------------------------------------------------------------
	-- UndoTree
	----------------------------------------------------------------------
	{
		"mbbill/undotree",
		keys = {
			{ "<leader>u", "<cmd>UndotreeToggle<cr>", desc = "Undotree: toggle" },
		},
	},

	----------------------------------------------------------------------
	-- Which Key
	----------------------------------------------------------------------
	{
		"folke/which-key.nvim",
		event = "VeryLazy",
		opts = {},
		keys = {
			{
				"<leader>?l",
				function()
					require("which-key").show({ global = false })
				end,
				desc = "Which-key: buffer local keymaps",
			},
			{
				"<leader>?g",
				function()
					require("which-key").show({ global = true })
				end,
				desc = "Which-key: global keymaps",
			},
		},
		config = function(_, opts)
			local wk = require("which-key")
			wk.setup(opts)

			wk.add({
				{ "<leader>a", group = "ai" },
				{ "<leader>d", group = "debug" },
				{ "<leader>f", group = "find" },
				{ "<leader>g", group = "git" },
				{ "<leader>h", group = "harpoon" },
				{ "<leader>j", group = "java" },
				{ "<leader>l", group = "lsp" },
				{ "<leader>m", group = "markdown" },
				{ "<leader>n", group = "neo-tree" },
				{ "<leader>p", group = "pairs/surround" },
				{ "<leader>s", group = "settings" },
				{ "<leader>w", group = "windows" },
				{ "<leader>?", group = "whichkey" },
			})
		end,
	},

	----------------------------------------------------------------------
	-- Nvim Surround (Lua rewrite of vim-surround)
	----------------------------------------------------------------------
	{
		"kylechui/nvim-surround",
		version = "*",
		keys = {
			{ "<leader>ps", desc = "Surround: add" },
			{ "<leader>pS", desc = "Surround: line" },
			{ "<leader>pd", desc = "Surround: delete" },
			{ "<leader>pc", desc = "Surround: change" },
			{ "<leader>pC", desc = "Surround: change (surrounding)" },
			{ "<leader>ps", mode = "x", desc = "Surround: visual add" },
		},
		opts = {
			keymaps = {
				normal = "<leader>ps",
				normal_cur = "<leader>pS",
				normal_line = false,
				normal_cur_line = false,
				visual = "<leader>ps",
				visual_line = false,
				delete = "<leader>pd",
				change = "<leader>pc",
				change_line = "<leader>pC",
			},
		},
	},

	----------------------------------------------------------------------
	-- nvim-bqf - QuickFix replacement
	----------------------------------------------------------------------
	{
		"kevinhwang91/nvim-bqf",
		ft = "qf",
		opts = {
			preview = {
				winblend = 0,
			},
		},
	},

	----------------------------------------------------------------------
	-- Windows - Maximize
	----------------------------------------------------------------------
	{
		"anuvyklack/windows.nvim",
		dependencies = "anuvyklack/middleclass",
		keys = {
			{ "<leader>wm", "<cmd>WindowsMaximize<cr>", silent = true, desc = "Windows: maximize" },
		},
		opts = {},
	},

	----------------------------------------------------------------------
	-- Markview
	----------------------------------------------------------------------
	{
		"OXY2DEV/markview.nvim",
		main = "markview",
		ft = { "markdown" },
		keys = {
			{ "<leader>mm", "<cmd>Markview toggle<cr>", silent = true, desc = "Markview: toggle" },
		},
		opts = {},
	},

	----------------------------------------------------------------------
	-- Mkdnflow
	----------------------------------------------------------------------
	{
		"jakewvincent/mkdnflow.nvim",
		ft = { "markdown", "rmd" },
		config = function()
			require("mkdnflow").setup({
				modules = { maps = false },
				on_attach = function(bufnr)
					local map = function(mode, lhs, rhs, desc)
						vim.keymap.set(mode, lhs, rhs, { buffer = bufnr, silent = true, desc = desc })
					end

					map("n", "<leader>mf", "<cmd>MkdnFollowLink<cr>", "Markdown: follow link")
					map("n", "<leader>mF", "<cmd>MkdnDestroyLink<cr>", "Markdown: destroy link")
					map({ "n", "v" }, "<leader>mc", "<cmd>MkdnCreateLink<cr>", "Markdown: create link")
					map({ "n", "v" }, "<leader>mC", "<cmd>MkdnCreateLinkFromClipboard<cr>", "Markdown: link from clipboard")
					map("n", "<leader>mn", "<cmd>MkdnNextLink<cr>", "Markdown: next link")
					map("n", "<leader>mp", "<cmd>MkdnPrevLink<cr>", "Markdown: prev link")
					map("n", "<leader>mb", "<cmd>MkdnGoBack<cr>", "Markdown: back")
					map("n", "<leader>mB", "<cmd>MkdnGoForward<cr>", "Markdown: forward")
					map({ "n", "v" }, "<leader>mt", "<cmd>MkdnToggleToDo<cr>", "Markdown: toggle todo")
				end,
			})
		end,
	},

	----------------------------------------------------------------------
	-- VimTex
	----------------------------------------------------------------------
	{
		"lervag/vimtex",
		ft = { "tex" },
		init = function()
			vim.g.vimtex_view_method = "general" -- use default
			vim.g.vimtex_compiler_method = "latexmk" -- default, explicit here
			vim.g.vimtex_quickfix_mode = 0 -- don't auto-open quickfix on warnings
			vim.g.vimtex_compiler_progname = "nvr" -- for inverse search (requires neovim-remote)

			-- Put all generated files (including PDF) into ./out
			vim.g.vimtex_compiler_latexmk = {
				continuous = 1,
				out_dir = "out",
				options = { "-pdf", "-interaction=nonstopmode", "-synctex=1" },
			}
		end,
	},
}
