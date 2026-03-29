----------------------------------------------------------------------
-- Markdown to PDF
----------------------------------------------------------------------
vim.api.nvim_create_user_command("MarkdownToPdf", function()
	local current_file = vim.fn.expand("%:p")
	local output_file = vim.fn.fnamemodify(current_file, ":r") .. ".pdf"
	vim.fn.system({ "pandoc", current_file, "-o", output_file })
	print("Converted " .. current_file .. " to " .. output_file)
end, { desc = "Convert current Markdown file to PDF" })

vim.keymap.set("n", "<leader>sc", ":MarkdownToPdf<CR>", { desc = "Convert Markdown to Pdf" })

----------------------------------------------------------------------
-- Treat .env files as plain text (prevent shellcheck diagnostics)
----------------------------------------------------------------------
vim.api.nvim_create_autocmd({ "BufRead", "BufNewFile" }, {
	pattern = ".env*",
	callback = function()
		vim.bo.filetype = "config"
	end,
})

----------------------------------------------------------------------
-- Auto-save on focus lost / buffer leave
----------------------------------------------------------------------
vim.api.nvim_create_autocmd({ "FocusLost", "BufLeave" }, {
	desc = "Auto-save on focus lost",
	callback = function()
		if vim.bo.modified and vim.bo.buftype == "" and vim.fn.expand("%") ~= "" then
			vim.cmd("silent! write")
		end
	end,
})

----------------------------------------------------------------------
-- Per-filetype indent defaults (fallback for new/empty files where sleuth can't detect)
----------------------------------------------------------------------
vim.api.nvim_create_autocmd("FileType", {
	pattern = { "javascript", "typescript", "typescriptreact", "javascriptreact", "json", "yaml", "html", "css" },
	callback = function()
		vim.opt_local.tabstop = 2
		vim.opt_local.shiftwidth = 2
		vim.opt_local.softtabstop = 2
	end,
})

----------------------------------------------------------------------
-- Refresh Neo-tree on git/diagnostic changes
----------------------------------------------------------------------
vim.api.nvim_create_autocmd({ "FocusGained", "BufWritePost", "ShellCmdPost", "TermLeave" }, {
	desc = "Refresh Neo-tree git status and diagnostics",
	callback = function()
		local ok, manager = pcall(require, "neo-tree.sources.manager")
		if ok then
			manager.refresh("filesystem")
		end
	end,
})

----------------------------------------------------------------------
-- Re-read buffers changed outside Neovim
----------------------------------------------------------------------
vim.api.nvim_create_autocmd({ "FocusGained", "BufEnter", "TermLeave" }, {
	desc = "Check for external file changes",
	command = "silent! checktime",
})

----------------------------------------------------------------------
-- Trim Trailing Whitespace
----------------------------------------------------------------------
local trim_ws_group = vim.api.nvim_create_augroup("TrimTrailingWhitespace", { clear = true })

vim.api.nvim_create_autocmd("BufWritePre", {
	group = trim_ws_group,
	desc = "Trim trailing whitespace",
	callback = function(args)
		local bufnr = args.buf
		if vim.bo[bufnr].buftype ~= "" then
			return
		end

		-- Keep intentional trailing spaces (e.g. Markdown hard line breaks)
		local ft = vim.bo[bufnr].filetype
		if ft == "markdown" then
			return
		end

		local view = vim.fn.winsaveview()
		vim.cmd([[silent! %s/\s\+$//e]])
		vim.fn.winrestview(view)
	end,
})
