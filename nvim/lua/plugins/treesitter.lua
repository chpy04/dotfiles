----------------------------------------------------------------------
-- TreeSitter
----------------------------------------------------------------------
return {
	"nvim-treesitter/nvim-treesitter",
	build = ":TSUpdate",
	lazy = false,
	dependencies = {
		"nvim-treesitter/nvim-treesitter-textobjects",
		"windwp/nvim-ts-autotag",
	},
	config = function()
		local ts = require("nvim-treesitter")
		ts.setup()

		-- Install parsers (skips already installed ones)
		ts.install({
			"vimdoc",
			"javascript",
			"typescript",
			"c",
			"cpp",
			"lua",
			"rust",
			"go",
			"gomod",
			"gosum",
			"jsdoc",
			"bash",
			"r",
			"markdown",
			"markdown_inline",
			"zig",
			"python",
			"prisma",
		})

		-- Enable treesitter features per filetype
		vim.api.nvim_create_autocmd("FileType", {
			callback = function(event)
				local buf = event.buf
				local lang = vim.treesitter.language.get_lang(event.match) or event.match

				-- Skip html
				if lang == "html" then
					return
				end

				-- Skip large files
				local max_filesize = 100 * 1024 -- 100 KB
				local ok, stats = pcall(vim.uv.fs_stat, vim.api.nvim_buf_get_name(buf))
				if ok and stats and stats.size > max_filesize then
					vim.notify(
						"File larger than 100KB treesitter disabled for performance",
						vim.log.levels.WARN,
						{ title = "Treesitter" }
					)
					return
				end

				-- Enable highlighting
				pcall(vim.treesitter.start, buf, lang)

				-- Enable indentation
				vim.bo[buf].indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
			end,
		})

		-- Auto-close and auto-rename HTML/JSX tags
		require("nvim-ts-autotag").setup()

		-- Set up textobjects keymaps manually
		local ts_select = require("nvim-treesitter-textobjects.select")
		local ts_move = require("nvim-treesitter-textobjects.move")
		local ts_swap = require("nvim-treesitter-textobjects.swap")

		-- Select
		local select_maps = {
			["af"] = "@function.outer",
			["if"] = "@function.inner",
			["ac"] = "@class.outer",
			["ic"] = "@class.inner",
			["aa"] = "@parameter.outer",
			["ia"] = "@parameter.inner",
			["ai"] = "@conditional.outer",
			["ii"] = "@conditional.inner",
			["al"] = "@loop.outer",
			["il"] = "@loop.inner",
		}
		for key, query in pairs(select_maps) do
			vim.keymap.set({ "x", "o" }, key, function()
				ts_select.select_textobject(query, "textobjects")
			end, { desc = "TS: " .. query })
		end

		-- Move
		local move_maps = {
			["]m"] = { fn = ts_move.goto_next_start, query = "@function.outer", desc = "Next function start" },
			["]M"] = { fn = ts_move.goto_next_end, query = "@function.outer", desc = "Next function end" },
			["]]"] = { fn = ts_move.goto_next_start, query = "@class.outer", desc = "Next class start" },
			["]["] = { fn = ts_move.goto_next_end, query = "@class.outer", desc = "Next class end" },
			["]a"] = { fn = ts_move.goto_next_start, query = "@parameter.inner", desc = "Next argument" },
			["[m"] = { fn = ts_move.goto_previous_start, query = "@function.outer", desc = "Prev function start" },
			["[M"] = { fn = ts_move.goto_previous_end, query = "@function.outer", desc = "Prev function end" },
			["[["] = { fn = ts_move.goto_previous_start, query = "@class.outer", desc = "Prev class start" },
			["[]"] = { fn = ts_move.goto_previous_end, query = "@class.outer", desc = "Prev class end" },
			["[a"] = { fn = ts_move.goto_previous_start, query = "@parameter.inner", desc = "Prev argument" },
		}
		for key, map in pairs(move_maps) do
			vim.keymap.set({ "n", "x", "o" }, key, function()
				map.fn(map.query, "textobjects")
			end, { desc = map.desc })
		end

		-- Swap
		vim.keymap.set("n", "<leader>ma", function()
			ts_swap.swap_next("@parameter.inner", "textobjects")
		end, { desc = "Swap with next argument" })
		vim.keymap.set("n", "<leader>mA", function()
			ts_swap.swap_previous("@parameter.inner", "textobjects")
		end, { desc = "Swap with prev argument" })

		-- Custom parser for templ
		vim.api.nvim_create_autocmd("User", {
			pattern = "TSUpdate",
			callback = function()
				require("nvim-treesitter.parsers").templ = {
					install_info = {
						url = "https://github.com/vrischmann/tree-sitter-templ.git",
						files = { "src/parser.c", "src/scanner.c" },
						branch = "master",
					},
				}
			end,
		})

		vim.treesitter.language.register("templ", "templ")
	end,
}
