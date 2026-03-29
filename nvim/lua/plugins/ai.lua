return {
	----------------------------------------------------------------------
	-- Opencode
	----------------------------------------------------------------------
	{
		"nickjvandyke/opencode.nvim",
		version = "*", -- Latest stable release.
		dependencies = {
			{
				-- `snacks.nvim` integration is recommended, but optional.
				---@module 'snacks' <- Loads `snacks.nvim` types for configuration intellisense.
				"folke/snacks.nvim",
				optional = true,
				opts = {
					-- Enhances `ask()`.
					input = {},
					-- Enhances `select()`.
					picker = {
						actions = {
							opencode_send = function(...)
								return require("opencode").snacks_picker_send(...)
							end,
						},
						win = {
							input = {
								keys = {
									["<a-a>"] = { "opencode_send", mode = { "n", "i" } },
								},
							},
						},
					},
					-- Enables the `snacks` provider.
					terminal = {},
				},
			},
		},
		config = function()
			---@type opencode.Opts
			vim.g.opencode_opts = {
				-- Your configuration, if any; goto definition on the type or field for details.
			}

			-- Required for `opts.events.reload`.
			vim.o.autoread = true

            local op = require("opencode")


			vim.keymap.set({ "n", "x" }, "<leader>aoa", function()
				op.ask("@this: ", { submit = true })
			end, { desc = "opencode: ask (@this)" })
			vim.keymap.set({ "n", "x" }, "<leader>aoe", function()
				op.select()
			end, { desc = "opencode: execute action" })
			vim.keymap.set({ "n", "t" }, "<leader>aot", function()
				op.toggle()
			end, { desc = "opencode: toggle" })

			vim.keymap.set({ "n", "x" }, "<leader>aor", function()
				return op.operator("@this ")
			end, { desc = "opencode: add range (@this)", expr = true })
			vim.keymap.set("n", "<leader>aol", function()
				return op.operator("@this ") .. "_"
			end, { desc = "opencode: add line (@this)", expr = true })

			-- Scroll Session
			vim.keymap.set("n", "<leader>aok", function()
				op.command("session.half.page.up")
			end, { desc = "opencode: scroll up" })
			vim.keymap.set("n", "<leader>aoj", function()
				op.command("session.half.page.down")
			end, { desc = "opencode: scroll down" })
		end,
	},

	----------------------------------------------------------------------
	-- Copilot
	----------------------------------------------------------------------
	{
		"zbirenbaum/copilot.lua",
		cmd = "Copilot",
		event = "InsertEnter",
		keys = {
			{ "<leader>act", "<cmd>Copilot toggle<cr>", silent = true, desc = "Copilot: toggle" },
			{ "<leader>acd", "<cmd>Copilot disable<cr>", silent = true, desc = "Copilot: disable" },
			{ "<leader>ace", "<cmd>Copilot enable<cr>", silent = true, desc = "Copilot: enable" },
		},
		opts = {
			suggestion = {
				auto_trigger = true,
				keymap = {
					accept = "<M-y>",
					next = "<M-]>",
					prev = "<M-[>",
					dismiss = "<C-]>",
				},
			},
			panel = { enabled = false },
		},
	},

	----------------------------------------------------------------------
	-- 99
	----------------------------------------------------------------------
	{
		"ThePrimeagen/99",
		config = function()
			local _99 = require("99")

			_99.setup({
				logger = {
					level = _99.ERROR,
					path = "/tmp/99.debug",
					print_on_error = true,
				},

				-- Model to use
				provider = _99.OpenCodeProvider,
				model = "openai/gpt-5.2",

				-- Visual-only usage: no cmp integration, no rules, no file completion
				completion = {
					source = nil,
					custom_rules = {},
					files = { enabled = false },
				},

				-- Don’t auto-inject AGENT.md files
				md_files = {},
			})

			vim.keymap.set("v", "<leader>a9v", function()
				_99.visual()
			end, { desc = "99: Visual prompt" })

			vim.keymap.set("n", "<leader>a9s", function()
				_99.stop_all_requests()
			end, { desc = "99: Stop requests" })

			vim.keymap.set("n", "<leader>a9l", function()
				_99.view_logs()
			end, { desc = "99: View logs" })
		end,
	},
}
