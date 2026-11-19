-- Pick what the git highlighting compares against (<leader>gB).
--
-- One choice drives gitsigns (gutter, hunks, ]g/[g), ]G/[G (changed files, which follow
-- gitsigns' base), neo-tree's git status, and <leader>gdb (diffview on the same base).
--
-- Branch comparisons use the merge-base, so you see only your own work (like a PR), not
-- commits that landed on the other branch after you branched. Bases are re-resolved on
-- FocusGained, so a fetch, rebase or push is picked up when you come back to Neovim.
--
-- tether's `code open --diff <rev>` sets its own base on top of this one and restores this
-- one when it clears (it saves gitsigns' base first). Picking here while tether's base is
-- active takes over from tether.

local M = {}

local function git(root, args)
	local cmd = { "git", "-C", root }
	vim.list_extend(cmd, args)
	local res = vim.system(cmd, { text = true }):wait()
	if res.code ~= 0 then
		return nil
	end
	return vim.trim(res.stdout)
end

local function repo_root()
	local file = vim.api.nvim_buf_get_name(0)
	local dir = (file ~= "" and vim.bo.buftype == "") and vim.fs.dirname(file) or vim.fn.getcwd()
	return git(dir, { "rev-parse", "--show-toplevel" })
end

-- origin/HEAD if the remote advertises it, else the first of main/master that exists.
local function default_branch(root)
	local ref = git(root, { "symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD" })
	if ref then
		return ref
	end
	for _, name in ipairs({ "origin/main", "origin/master", "main", "master" }) do
		if git(root, { "rev-parse", "--verify", "--quiet", name .. "^{commit}" }) then
			return name
		end
	end
end

-- Each mode resolves to { base, label, tree } or nil (+ reason) when unavailable.
--   base:  what gitsigns diffs against (nil = the index, gitsigns' default)
--   tree:  what neo-tree diffs against (nil = its normal staged/unstaged status)
--   short: shown in the statusline
local modes = {
	{
		id = "unstaged",
		label = "Unstaged changes",
		resolve = function()
			return { base = nil, tree = nil, short = nil }
		end,
	},
	{
		id = "head",
		label = "Staged + unstaged (vs HEAD)",
		resolve = function()
			return { base = "HEAD", tree = "HEAD", short = "HEAD" }
		end,
	},
	{
		id = "upstream",
		label = "Unpushed commits + local (vs upstream)",
		resolve = function(root)
			local up = git(root, { "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}" })
			if not up then
				return nil, "no upstream branch"
			end
			-- A ref, not a sha: gitsigns re-reads it, so a push or fetch moves the base.
			return { base = "@{u}", tree = up, short = up, detail = up }
		end,
	},
	{
		id = "default",
		label = "Branch vs default branch (merge-base)",
		resolve = function(root)
			local branch = default_branch(root)
			if not branch then
				return nil, "no default branch found"
			end
			local mb = git(root, { "merge-base", "HEAD", branch })
			if not mb then
				return nil, "no merge-base with " .. branch
			end
			local short = git(root, { "rev-parse", "--short", mb }) or mb:sub(1, 7)
			return { base = mb, tree = mb, short = branch .. "…", detail = ("%s @ %s"):format(branch, short) }
		end,
	},
}

local function find_mode(id)
	for _, m in ipairs(modes) do
		if m.id == id then
			return m
		end
	end
end

-- Current choice: { id, root, base, tree, short }. nil = never picked (same as "unstaged").
M.current = nil

local function tether_active()
	return vim.g.tether_git_base ~= nil
end

-- neo-tree: set our base for this worktree on every source state. Trees opened later get it
-- through the FileType autocmd below.
local function tree_apply()
	if not package.loaded["neo-tree"] or tether_active() then
		return
	end
	local ok, manager = pcall(require, "neo-tree.sources.manager")
	if not ok then
		return
	end
	local cur = M.current
	for _, src in ipairs({ "filesystem", "git_status", "buffers" }) do
		pcall(manager._for_each_state, src, function(state)
			state.git_base_by_worktree = state.git_base_by_worktree or {}
			if state.diffbase_root then
				state.git_base_by_worktree[state.diffbase_root] = nil
				state.diffbase_root = nil
			end
			if cur and cur.tree then
				state.git_base_by_worktree[cur.root] = cur.tree
				state.diffbase_root = cur.root
			end
		end)
		pcall(manager.refresh, src)
	end
end

-- Take over from tether: forget its base so it neither restores over us nor keeps its tree entry.
local function release_tether()
	if not tether_active() then
		return
	end
	local root = vim.g.tether_git_base.root
	vim.g.tether_git_base = nil
	vim.g.tether_gs_orig = nil
	local ok, manager = pcall(require, "neo-tree.sources.manager")
	if ok then
		for _, src in ipairs({ "filesystem", "git_status", "buffers" }) do
			pcall(manager._for_each_state, src, function(state)
				if state.tether_git_root == root and state.git_base_by_worktree then
					state.git_base_by_worktree[root] = nil
				end
				state.tether_git_root = nil
			end)
		end
	end
end

local function apply(id, root, opts)
	opts = opts or {}
	local mode = find_mode(id)
	local r, why = mode.resolve(root)
	if not r then
		vim.notify(("Diff base: %s unavailable (%s)"):format(mode.label, why), vim.log.levels.WARN)
		return false
	end
	local prev = M.current
	local changed = not prev or prev.id ~= id or prev.base ~= r.base or prev.tree ~= r.tree or prev.root ~= root
	if opts.passive and not changed then
		return true
	end

	release_tether()
	M.current = { id = id, root = root, base = r.base, tree = r.tree, short = r.short }
	local ok, gs = pcall(require, "gitsigns")
	if ok then
		gs.change_base(r.base, true)
	end
	tree_apply()
	if package.loaded["lualine"] then
		pcall(require("lualine").refresh)
	end
	if not opts.passive then
		local msg = "Diff base: " .. mode.label .. (r.detail and (" — " .. r.detail) or "")
		vim.notify(msg, vim.log.levels.INFO)
	end
	return true
end

--- Open the picker.
function M.pick()
	local root = repo_root()
	if not root then
		vim.notify("Not in a git repo", vim.log.levels.WARN)
		return
	end
	local cur_id = (M.current and not tether_active()) and M.current.id or (not tether_active() and "unstaged")
	local items = {}
	for _, m in ipairs(modes) do
		local r, why = m.resolve(root)
		table.insert(items, { mode = m, ok = r ~= nil, detail = r and r.detail or why })
	end
	vim.ui.select(items, {
		prompt = tether_active() and ("Diff base (tether: %s)"):format(vim.g.tether_git_base.rev) or "Diff base",
		format_item = function(it)
			local mark = it.mode.id == cur_id and "● " or "  "
			local tail = it.detail and ("  (" .. it.detail .. ")") or ""
			if not it.ok then
				tail = "  [unavailable: " .. it.detail .. "]"
			end
			return mark .. it.mode.label .. tail
		end,
	}, function(choice)
		if choice then
			apply(choice.mode.id, root)
		end
	end)
end

--- The gitsigns-style base diffview should open on (nil = diffview's default: index vs worktree).
function M.diffview_open()
	local base = tether_active() and vim.g.tether_git_base.sha or (M.current and M.current.base)
	vim.cmd(base and ("DiffviewOpen " .. base) or "DiffviewOpen")
end

--- Statusline text: empty when on the default (unstaged) base.
function M.status()
	if tether_active() then
		return "Δ tether:" .. vim.g.tether_git_base.rev
	end
	local cur = M.current
	if not cur or cur.id == "unstaged" then
		return ""
	end
	return "Δ " .. cur.short
end

--- The mode currently in effect ("unstaged" when nothing was picked).
function M.mode()
	return M.current and M.current.id or "unstaged"
end

local group = vim.api.nvim_create_augroup("DiffBase", { clear = true })

-- Re-resolve on focus (fetch / rebase / new commits moved the merge-base) and keep the tree in sync.
vim.api.nvim_create_autocmd("FocusGained", {
	group = group,
	desc = "Diff base: re-resolve after outside git changes",
	callback = function()
		local cur = M.current
		if cur and cur.id ~= "unstaged" and not tether_active() then
			vim.schedule(function()
				if not apply(cur.id, cur.root, { passive = true }) then
					apply("unstaged", cur.root, { passive = true })
				end
				tree_apply()
			end)
		end
	end,
})

vim.api.nvim_create_autocmd("FileType", {
	group = group,
	pattern = "neo-tree",
	desc = "Diff base: apply to new neo-tree states",
	callback = function()
		if M.current then
			vim.schedule(tree_apply)
		end
	end,
})

return M
