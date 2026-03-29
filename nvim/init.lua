local lazypath = vim.fn.stdpath("data") .. "/lazy/lazy.nvim"
if not (vim.uv or vim.loop).fs_stat(lazypath) then
	local lazyrepo = "https://github.com/folke/lazy.nvim.git"
	local out = vim.fn.system({ "git", "clone", "--filter=blob:none", "--branch=stable", lazyrepo, lazypath })
	if vim.v.shell_error ~= 0 then
		vim.api.nvim_echo({
			{ "Failed to clone lazy.nvim:\n", "ErrorMsg" },
			{ out, "WarningMsg" },
			{ "\nPress any key to exit..." },
		}, true, {})
		vim.fn.getchar()
		os.exit(1)
	end
end
vim.opt.rtp:prepend(lazypath)

require("config")

require("lazy").setup({
	spec = {
		{ import = "plugins" },
	},
	checker = { enabled = false },
})

local theme = require("config.theme_picker").load()
local ok, _ = pcall(vim.cmd.colorscheme, theme)
if not ok then
	vim.notify("Colorscheme '" .. theme .. "' not found, falling back to default", vim.log.levels.WARN)
end

-- Ensure NotifyBackground is defined (prevents nvim-notify warning)
if vim.fn.hlexists("NotifyBackground") == 0 or vim.api.nvim_get_hl(0, { name = "NotifyBackground" }).bg == nil then
	local normal_bg = vim.api.nvim_get_hl(0, { name = "Normal" }).bg
	vim.api.nvim_set_hl(0, "NotifyBackground", { bg = normal_bg or 0x000000 })
end
