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

----------------------------------------------------------------------
-- Neo-tree: make the cursor line a solid accent bar
-- Colours are pulled from whatever theme is active (theme picker swaps
-- them), so this re-runs on every colorscheme change.
----------------------------------------------------------------------
local function neotree_cursorline()
	local function get(name)
		return vim.api.nvim_get_hl(0, { name = name, link = false })
	end
	-- Mix the theme's accent into the background. Neo-tree draws names/icons with their
	-- own fg colours, so the bar must stay dark/light enough for those to read on it.
	local function blend(fg, bg, alpha)
		local function ch(c, shift)
			return bit.band(bit.rshift(c, shift), 0xff)
		end
		local out = 0
		for _, shift in ipairs({ 16, 8, 0 }) do
			local v = math.floor(ch(fg, shift) * alpha + ch(bg, shift) * (1 - alpha) + 0.5)
			out = out + bit.lshift(v, shift)
		end
		return out
	end
	local accent = get("Function").fg or get("Title").fg or get("Directory").fg
	local bg = get("NeoTreeNormal").bg or get("Normal").bg or (vim.o.background == "light" and 0xffffff or 0x000000)
	if accent then
		vim.api.nvim_set_hl(0, "NeoTreeCursorLine", { bg = blend(accent, bg, 0.35), bold = true })
	else
		vim.api.nvim_set_hl(0, "NeoTreeCursorLine", { link = "Visual" })
	end
end

vim.api.nvim_create_autocmd("ColorScheme", {
	desc = "Neo-tree: high-contrast cursor line",
	callback = neotree_cursorline,
})
