-- Step through git-changed files (]G / [G), the file-level companion to ]g / [g.
--
-- "Changed" follows gitsigns: with a base set (`:Gitsigns change_base`, or tether's
-- `code open --diff <rev>`) it is everything that differs from that rev; with no base it
-- is staged + unstaged changes. Untracked files are always included, deleted ones never
-- (nothing to open). Files are ordered like neo-tree (directories first, then names,
-- case-insensitively), so stepping walks the highlighted files top to bottom.

local M = {}

local function git(root, args)
	local cmd = { "git", "-C", root }
	vim.list_extend(cmd, args)
	local res = vim.system(cmd, { text = true }):wait()
	if res.code ~= 0 then
		return nil, vim.trim(res.stderr or "")
	end
	return res.stdout
end

-- NUL-separated output (-z) so paths with spaces or unicode come back verbatim.
local function add_paths(set, out)
	for path in (out or ""):gmatch("[^%z]+") do
		set[path] = true
	end
end

local function gitsigns_base()
	local ok, cfg = pcall(require, "gitsigns.config")
	return ok and cfg.config.base or nil
end

local function repo_root()
	local file = vim.api.nvim_buf_get_name(0)
	local dir = (file ~= "" and vim.bo.buftype == "") and vim.fs.dirname(file) or vim.fn.getcwd()
	local out = git(dir, { "rev-parse", "--show-toplevel" })
	return out and vim.trim(out) or nil
end

-- Tree order: compare path components; at the first difference a directory sorts before
-- a file, otherwise names compare case-insensitively.
local function tree_less(a, b)
	local pa, pb = vim.split(a, "/", { plain = true }), vim.split(b, "/", { plain = true })
	for i = 1, math.min(#pa, #pb) do
		if pa[i] ~= pb[i] then
			local a_dir, b_dir = i < #pa, i < #pb
			if a_dir ~= b_dir then
				return a_dir
			end
			local la, lb = pa[i]:lower(), pb[i]:lower()
			if la ~= lb then
				return la < lb
			end
			return pa[i] < pb[i]
		end
	end
	return #pa < #pb
end

--- Changed files relative to the repo root, in tree order.
function M.list(root)
	local set = {}
	local base = gitsigns_base()
	if base then
		local out, err = git(root, { "diff", "-z", "--name-only", "--diff-filter=d", base })
		if not out then
			return nil, err
		end
		add_paths(set, out)
	else
		add_paths(set, git(root, { "diff", "-z", "--name-only", "--diff-filter=d" }))
		add_paths(set, git(root, { "diff", "-z", "--name-only", "--diff-filter=d", "--cached" }))
	end
	add_paths(set, git(root, { "ls-files", "-z", "--others", "--exclude-standard" }))

	local files = vim.tbl_keys(set)
	table.sort(files, tree_less)
	return files
end

-- Hunks are computed asynchronously after a buffer attaches, so poll gitsigns' cache until
-- they exist (staged ones too, when present), then jump. Gives up if you move first.
local function first_hunk(buf)
	local win = vim.api.nvim_get_current_win()
	local start = vim.api.nvim_win_get_cursor(win)
	local tries = 0
	local function attempt()
		tries = tries + 1
		if
			not vim.api.nvim_win_is_valid(win)
			or vim.api.nvim_win_get_buf(win) ~= buf
			or not vim.deep_equal(vim.api.nvim_win_get_cursor(win), start)
		then
			return
		end
		local ok, cache = pcall(require, "gitsigns.cache")
		local bc = ok and cache.cache[buf] or nil
		local ready = bc and bc.hunks and (#bc.hunks > 0 or bc.hunks_staged)
		if ready or tries >= 40 then
			vim.api.nvim_win_call(win, function()
				pcall(require("gitsigns").nav_hunk, "first", { target = "all", navigation_message = false })
			end)
		else
			vim.defer_fn(attempt, 50)
		end
	end
	attempt()
end

local function reveal_in_tree(path)
	local ok, command = pcall(require, "neo-tree.command")
	if not ok then
		return
	end
	command.execute({ action = "show", source = "filesystem", position = "left", reveal_file = path })
	vim.schedule(function()
		local state = require("neo-tree.sources.manager").get_state("filesystem")
		if state.winid and vim.api.nvim_win_is_valid(state.winid) then
			vim.api.nvim_win_call(state.winid, function()
				vim.cmd("normal! zz")
			end)
		end
	end)
end

--- Open the next (step = 1) or previous (step = -1) changed file.
function M.step(step)
	local root = repo_root()
	if not root then
		vim.notify("Not in a git repo", vim.log.levels.WARN)
		return
	end
	local files, err = M.list(root)
	if not files then
		vim.notify("Changed files: " .. err, vim.log.levels.ERROR)
		return
	end
	if #files == 0 then
		local base = gitsigns_base()
		vim.notify(base and ("No files changed against " .. base) or "No changed files", vim.log.levels.INFO)
		return
	end

	-- Where are we in the list? If the current file isn't in it, slot in by tree order.
	local cur = vim.api.nvim_buf_get_name(0)
	local rel = vim.startswith(cur, root .. "/") and cur:sub(#root + 2) or nil
	local idx
	for i, f in ipairs(files) do
		if f == rel then
			idx = i + step
			break
		end
	end
	if not idx then
		idx = step > 0 and #files + 1 or 0
		if rel then
			for i, f in ipairs(files) do
				if tree_less(rel, f) then
					idx = step > 0 and i or i - 1
					break
				end
			end
		end
	end
	idx = (idx - 1) % #files + 1

	-- Never open the file inside the tree (or another special window).
	if vim.bo.filetype == "neo-tree" or vim.bo.buftype ~= "" then
		vim.cmd("wincmd p")
	end

	local path = root .. "/" .. files[idx]
	vim.cmd.edit(vim.fn.fnameescape(path))
	first_hunk(vim.api.nvim_get_current_buf())
	reveal_in_tree(path)
	vim.api.nvim_echo({ { ("[%d/%d] %s"):format(idx, #files, files[idx]) } }, false, {})
end

return M
