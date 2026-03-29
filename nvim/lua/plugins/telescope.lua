----------------------------------------------------------------------
-- Telescope
----------------------------------------------------------------------
return {
	{ "nvim-telescope/telescope-ui-select.nvim" },
	{ "nvim-telescope/telescope-fzf-native.nvim", build = "make" },

	{
		"nvim-telescope/telescope.nvim",
		branch = "master",
		dependencies = { "nvim-lua/plenary.nvim" },
		keys = {
			{ "<leader>ff", "<cmd>Telescope find_files<cr>", desc = "Find: files" },
			{ "<leader>fg", "<cmd>Telescope live_grep<cr>", desc = "Find: live grep" },
			{ "<leader>fb", "<cmd>Telescope buffers<cr>", desc = "Find: buffers" },
			{ "<leader>fo", "<cmd>Telescope oldfiles<cr>", desc = "Find: oldfiles" },
			{ "<leader>fw", "<cmd>Telescope grep_string<cr>", desc = "Find: word under cursor" },
			{ "<leader>fh", "<cmd>Telescope help_tags<cr>", desc = "Find: help tags" },
			{ "<leader>fk", "<cmd>Telescope keymaps<cr>", desc = "Find: keymaps" },
			{ "<leader>fc", "<cmd>Telescope commands<cr>", desc = "Find: commands" },
			{ "<leader>fC", "<cmd>Telescope command_history<cr>", desc = "Find: command history" },
			{ "<leader>fs", "<cmd>Telescope current_buffer_fuzzy_find<cr>", desc = "Find: in buffer" },
			{ "<leader>fm", "<cmd>Telescope marks<cr>", desc = "Find: marks" },
			{ "<leader>fn", "<cmd>Telescope notify<cr>", desc = "Find: notifications" },
			{ "<leader>fj", "<cmd>Telescope jumplist<cr>", desc = "Find: jumplist" },
			{ "<leader>fr", "<cmd>Telescope registers<cr>", desc = "Find: registers" },
			{ "<leader>ft", "<cmd>Telescope treesitter<cr>", desc = "Find: treesitter" },
			{ "<leader>f/", "<cmd>Telescope search_history<cr>", desc = "Find: search history" },
			{ "<leader>fq", "<cmd>Telescope quickfix<cr>", desc = "Find: quickfix" },
			{ "<leader>fl", "<cmd>Telescope loclist<cr>", desc = "Find: loclist" },
			{ "<leader>gtc", "<cmd>Telescope git_commits<cr>", desc = "Git: commits (Telescope)" },
			{ "<leader>gts", "<cmd>Telescope git_status<cr>", desc = "Git: status (Telescope)" },
			{ "<leader>gtB", "<cmd>Telescope git_branches<cr>", desc = "Git: branches (Telescope)" },
			{
				"<leader>fa",
				function()
					local dirs = {
						"~/Workspace",
						"~/Northeastern",
						"~/Work",
					}

					-- Filter to only directories that exist
					local search_dirs = {}
					for _, dir in ipairs(dirs) do
						local expanded = vim.fn.expand(dir)
						if vim.fn.isdirectory(expanded) == 1 then
							table.insert(search_dirs, expanded)
						end
					end

					if #search_dirs == 0 then
						vim.notify("No search directories found", vim.log.levels.WARN)
						return
					end

					require("telescope.builtin").find_files({
						prompt_title = "Search in Selected Dirs",
						search_dirs = search_dirs,
						hidden = true,
						no_ignore = true,
						find_command = {
							"fd",
							"--type",
							"f",
							"--hidden",
							"--no-ignore",
							"--exclude",
							"node_modules",
							"--exclude",
							"venv",
							"--exclude",
							".venv",
							"--exclude",
							".git",
							"--exclude",
							"__pycache__",
						},
					})
				end,
				desc = "Find: selected dirs",
			},
		},
		config = function()
			local telescope = require("telescope")

			telescope.setup({
				defaults = {
					sorting_strategy = "ascending",
					layout_strategy = "horizontal",
					layout_config = {
						prompt_position = "top",
						width = 0.95,
						height = 0.95,
						horizontal = {
							preview_width = 0.6,
						},
					},
					preview = { hide_on_startup = false },
					borderchars = { "─", "│", "─", "│", "╭", "╮", "╯", "╰" },
					file_ignore_patterns = {
						"node_modules/",
						".git/",
						"__pycache__/",
						".venv/",
						"venv/",
						"%.lock",
					},
				},
				extensions = {
					["ui-select"] = {
						layout_strategy = "horizontal",
						sorting_strategy = "ascending",
						layout_config = {
							prompt_position = "top",
							width = 0.95,
							height = 0.95,
							horizontal = { preview_width = 0.6 },
						},
					},
				},
			})

			telescope.load_extension("fzf")
			telescope.load_extension("ui-select")
			telescope.load_extension("notify")
		end,
	},
}
