-- Global diagnostic UI config
vim.diagnostic.config({
	virtual_text = false,
	underline = true,
	severity_sort = true,
	float = {
		border = "rounded",
		source = true,
		wrap = true,
		max_width = 80,
	},
})

-- Diagnostic state
local vt_on = false
local ul_on = true

-- Toggle virtual text
vim.keymap.set("n", "<leader>lee", function()
	vt_on = not vt_on
	vim.diagnostic.config({ virtual_text = vt_on })
	vim.notify("Virtual text: " .. (vt_on and "ENABLED" or "DISABLED"), vim.log.levels.INFO)
end, { desc = "Diagnostics: toggle virtual text" })

-- Toggle underline
vim.keymap.set("n", "<leader>leu", function()
	ul_on = not ul_on
	vim.diagnostic.config({ underline = ul_on })
	vim.notify("Underline: " .. (ul_on and "ENABLED" or "DISABLED"), vim.log.levels.INFO)
end, { desc = "Diagnostics: toggle underline" })

-- Toggle inlay hints
vim.keymap.set("n", "<leader>li", function()
	local enabled = not vim.lsp.inlay_hint.is_enabled()
	vim.lsp.inlay_hint.enable(enabled)
	vim.notify("Inlay hints: " .. (enabled and "ENABLED" or "DISABLED"), vim.log.levels.INFO)
end, { desc = "LSP: toggle inlay hints" })

-- Open diagnostic float
vim.keymap.set("n", "<leader>lef", function()
	vim.diagnostic.open_float(nil, { focusable = true })
end, { silent = true, desc = "Diagnostics: float" })

-- gh: Show diagnostic if available, otherwise show hover (like VSCode Vim)
vim.keymap.set("n", "gh", function()
	local diagnostics = vim.diagnostic.get(0, { lnum = vim.fn.line(".") - 1 })
	if #diagnostics > 0 then
		vim.diagnostic.open_float(nil, { focusable = true })
	else
		vim.lsp.buf.hover()
	end
end, { silent = true, desc = "Show diagnostic or hover" })

-- Navigation: next / prev diagnostic
vim.keymap.set("n", "]d", function()
	vim.diagnostic.jump({ count = 1, float = true })
end, { silent = true, desc = "Diagnostics: next" })

vim.keymap.set("n", "[d", function()
	vim.diagnostic.jump({ count = -1, float = true })
end, { silent = true, desc = "Diagnostics: prev" })

-- Navigation: next / prev ERROR only
vim.keymap.set("n", "]e", function()
	vim.diagnostic.jump({ count = 1, float = true, severity = vim.diagnostic.severity.ERROR })
end, { silent = true, desc = "Diagnostics: next error" })

vim.keymap.set("n", "[e", function()
	vim.diagnostic.jump({ count = -1, float = true, severity = vim.diagnostic.severity.ERROR })
end, { silent = true, desc = "Diagnostics: prev error" })

-- Navigation: next / prev WARNING only
vim.keymap.set("n", "]w", function()
	vim.diagnostic.jump({ count = 1, float = true, severity = vim.diagnostic.severity.WARN })
end, { silent = true, desc = "Diagnostics: next warning" })

vim.keymap.set("n", "[w", function()
	vim.diagnostic.jump({ count = -1, float = true, severity = vim.diagnostic.severity.WARN })
end, { silent = true, desc = "Diagnostics: prev warning" })

-- Copy diagnostic under cursor to clipboard
vim.keymap.set("n", "<leader>ley", function()
	local diagnostics = vim.diagnostic.get(0, { lnum = vim.fn.line(".") - 1 })
	if #diagnostics == 0 then
		vim.notify("No diagnostics on this line", vim.log.levels.INFO)
		return
	end
	local messages = {}
	for _, d in ipairs(diagnostics) do
		table.insert(messages, d.message)
	end
	local text = table.concat(messages, "\n")
	vim.fn.setreg("+", text)
	vim.notify("Copied diagnostic to clipboard", vim.log.levels.INFO)
end, { silent = true, desc = "Diagnostics: yank to clipboard" })

-- Send diagnostics to quickfix / loclist
vim.keymap.set("n", "<leader>leq", function()
	vim.diagnostic.setqflist()
end, { silent = true, desc = "Diagnostics: send to quickfix" })

vim.keymap.set("n", "<leader>lel", function()
	vim.diagnostic.setloclist()
end, { silent = true, desc = "Diagnostics: send to loclist" })

-- Browse diagnostics via Telescope (global — works without LSP)
vim.keymap.set("n", "<leader>lD", function()
	local ok, tb = pcall(require, "telescope.builtin")
	if ok then
		tb.diagnostics()
	else
		vim.diagnostic.setqflist()
		vim.cmd("copen")
	end
end, { silent = true, desc = "Diagnostics: browse (Telescope)" })
