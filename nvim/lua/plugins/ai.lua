return {
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
}
