-- Toggle Spell Check
vim.keymap.set("n", "<leader>sp", "<cmd>setlocal spell!<cr>", { silent = true, desc = "Toggle spell check" })

-- Yank
vim.keymap.set("v", "<leader>y", '"+y', { silent = true, desc = "Yank to system clipboard" })
vim.keymap.set("v", "<leader>x", '"+d', { silent = true, desc = "Cut to system clipboard" })

-- Clear search
vim.keymap.set("n", "<leader>/", "<cmd>nohlsearch<cr>", { silent = true, desc = "Clear search highlight" })
vim.keymap.set("n", "<Esc><Esc>", "<cmd>nohlsearch<cr>", { silent = true, desc = "Clear search highlight" })

-- Open Terminal
vim.keymap.set("n", "<leader>t", function()
	vim.cmd("belowright 15split | term")
end, { silent = true, desc = "Open terminal" })

-- Terminal: exit insert mode with Escape
vim.keymap.set("t", "<Esc><Esc>", "<C-\\><C-n>", { silent = true, desc = "Terminal: exit insert mode" })

-- Toggle jk
local toggle_jk = false
vim.keymap.set("n", "<leader>sj", function()
	if toggle_jk then
		vim.keymap.set("n", "j", "j", { remap = false, silent = true })
		vim.keymap.set("n", "k", "k", { remap = false, silent = true })
		toggle_jk = false
		print("Switched to normal j/k")
	else
		vim.keymap.set("n", "j", "gj", { remap = false, silent = true })
		vim.keymap.set("n", "k", "gk", { remap = false, silent = true })
		toggle_jk = true
		print("Switched to gj/gk")
	end
end, { silent = true, desc = "Toggle j/k and gj/gk" })

-- Theme selector
vim.keymap.set("n", "<leader>st", function()
	require("config.theme_picker").pick()
end, { silent = true, desc = "Theme: pick" })

-- Navigate between splits with Ctrl-hjkl
vim.keymap.set("n", "<C-h>", "<C-w>h", { silent = true, desc = "Move to left split" })
vim.keymap.set("n", "<C-j>", "<C-w>j", { silent = true, desc = "Move to below split" })
vim.keymap.set("n", "<C-k>", "<C-w>k", { silent = true, desc = "Move to above split" })
vim.keymap.set("n", "<C-l>", "<C-w>l", { silent = true, desc = "Move to right split" })

-- Go to definition (follow into other files)
vim.keymap.set("n", "gd", "<C-]>", { silent = true, desc = "Go to definition" })

-- Keep selection after indent/unindent
vim.keymap.set("v", "<", "<gv", { silent = true, desc = "Indent left and reselect" })
vim.keymap.set("v", ">", ">gv", { silent = true, desc = "Indent right and reselect" })

-- Center cursor after search navigation
vim.keymap.set("n", "n", "nzzzv", { silent = true, desc = "Next search result (centered)" })
vim.keymap.set("n", "N", "Nzzzv", { silent = true, desc = "Prev search result (centered)" })

-- Replace strings
vim.keymap.set("n", "<leader>sr", ":%s//gc<Left><Left><Left>", { silent = false, desc = "Search & replace (global)" })
vim.keymap.set(
	"n",
	"<leader>sq",
	":cdo %s//gc | update<Left><left><Left><Left><Left><Left><Left><Left><Left><Left><Left><Left>",
	{ silent = false, desc = "Search & replace (quickfix)" }
)
