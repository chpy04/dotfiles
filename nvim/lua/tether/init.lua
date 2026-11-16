-- tether: send a comment tied to a line (or range) to an agent in the current Herdr workspace.
--   require("tether").setup({ keys = { comment = "<leader>ac", comment_pick = "<leader>aC" } })
local M = {}

local config = require("tether.config")
local float = require("tether.float")
local picker = require("tether.picker")

M.opts = nil
M._setup_done = false
local mapped = {} -- { {mode, lhs}, ... } so setup() can be re-run cleanly

local function notify(msg, level)
  vim.notify(msg, level or vim.log.levels.INFO, { title = "tether" })
end

local function in_herdr()
  return vim.env.HERDR_ENV == "1"
end

local function first_line(s)
  return ((s or ""):gsub("^%s+", ""):match("([^\n]*)")) or ""
end

--- Capture what is being commented on NOW (before anything async or any focus change).
--- `range` = { first, last } overrides (used by :TetherComment with a range).
local function capture(range)
  local win = vim.api.nvim_get_current_win()
  local buf = vim.api.nvim_win_get_buf(win)
  local mode = vim.fn.mode()
  local first, last
  if range then
    first, last = range[1], range[2]
  elseif mode == "v" or mode == "V" or mode == "\22" then
    first, last = vim.fn.line("v"), vim.fn.line(".")
    if first > last then
      first, last = last, first
    end
    -- leave visual mode; the cursor stays where it is
    vim.cmd([[execute "normal! \<Esc>"]])
  else
    first = vim.api.nvim_win_get_cursor(win)[1]
    last = first
  end
  return { win = win, buf = buf, first = first, last = last }
end

local function ref_for(cap)
  if vim.bo[cap.buf].buftype ~= "" then
    return nil, "this buffer is not a file"
  end
  local name = vim.api.nvim_buf_get_name(cap.buf)
  if name == "" then
    return nil, "this buffer has no file name (save it first)"
  end
  local path = vim.fn.fnamemodify(name, ":p")
  if cap.first == cap.last then
    return ("%s:%d"):format(path, cap.first)
  end
  return ("%s:%d-%d"):format(path, cap.first, cap.last)
end

--- Run tether asynchronously; cb(code, stdout, stderr). Never blocks the editor.
local function run_tether(args, cb)
  local argv = { M.opts.bin }
  vim.list_extend(argv, args)
  local ok, err = pcall(vim.system, argv, { text = true, timeout = M.opts.timeout_ms }, function(res)
    vim.schedule(function()
      cb(res.code, res.stdout or "", res.stderr or "")
    end)
  end)
  if not ok then
    vim.schedule(function()
      cb(127, "", ("cannot run `%s`: %s"):format(M.opts.bin, tostring(err)))
    end)
  end
end

local function send(ref, target, text)
  run_tether({ "send", "--to", target, "--ref", ref, "--text", text }, function(code, stdout, stderr)
    local err = vim.trim(stderr):gsub("tether: error: ", "")
    if code == 0 then
      local msg = first_line(stdout)
      if msg == "" then
        msg = "comment sent"
      end
      local warn = vim.trim(stderr)
      if warn ~= "" then
        notify(msg .. "\n" .. warn, vim.log.levels.WARN)
      else
        notify(msg)
      end
    else
      if err == "" then
        err = "unknown error"
      end
      notify(("send failed (exit %d): %s"):format(code, err), vim.log.levels.ERROR)
    end
  end)
end

--- Open the comment float for `cap`, targeting `target` ("primary" or a pane id).
--- `agent` (optional) is the already-known agent row used for the title.
local function open_float(cap, ref, target, agent)
  local title = "→ primary agent"
  if agent then
    title = ("→ %s · tab %s"):format(agent.kind, agent.tab_index)
  end
  local handle = float.open({
    win = cap.win,
    end_lnum = cap.last,
    title = title,
    width = M.opts.float.width,
    max_height = M.opts.float.max_height,
    border = M.opts.float.border,
    on_submit = function(text)
      send(ref, target, text)
    end,
  })
  if not agent then
    -- fill in the real title (`→ claude · tab 1`) once we know who "primary" is
    run_tether({ "agents", "--json" }, function(code, stdout, stderr)
      if not handle.is_open() then
        return
      end
      local agents = code == 0 and picker.parse(stdout) or nil
      if not agents then
        handle.cancel()
        local why = vim.trim(stderr):gsub("tether: error: ", "")
        notify("cannot find the agent: " .. (why ~= "" and why or "tether agents failed"), vim.log.levels.ERROR)
        return
      end
      for _, a in ipairs(agents) do
        if (target == "primary" and a.is_primary) or a.pane_id == target then
          handle.set_title(("→ %s · tab %s"):format(a.kind, a.tab_index))
          return
        end
      end
      handle.cancel()
      notify("no agent in this Herdr workspace; start one (e.g. claude) first", vim.log.levels.ERROR)
    end)
  end
  return handle
end

local function precheck()
  if not M.opts then
    M.setup()
  end
  if not in_herdr() then
    notify(
      "not inside Herdr (HERDR_ENV is not 1): run Neovim in a Herdr pane to send comments to an agent",
      vim.log.levels.ERROR
    )
    return false
  end
  return true
end

--- Comment on the current line / visual range (or opts.range = {first,last}) to the primary agent.
function M.comment(opts)
  opts = opts or {}
  if not precheck() then
    return
  end
  local cap = capture(opts.range)
  local ref, err = ref_for(cap)
  if not ref then
    return notify("cannot comment: " .. err, vim.log.levels.ERROR)
  end
  return open_float(cap, ref, opts.target or M.opts.target)
end

--- Pick an agent (vim.ui.select), then open the same float targeting it.
function M.comment_pick(opts)
  opts = opts or {}
  if not precheck() then
    return
  end
  local cap = capture(opts.range)
  local ref, err = ref_for(cap)
  if not ref then
    return notify("cannot comment: " .. err, vim.log.levels.ERROR)
  end
  run_tether({ "agents", "--json" }, function(code, stdout, stderr)
    local agents = code == 0 and picker.parse(stdout) or nil
    if not agents then
      local why = vim.trim(stderr):gsub("tether: error: ", "")
      return notify("cannot list agents: " .. (why ~= "" and why or "tether agents failed"), vim.log.levels.ERROR)
    end
    if #agents == 0 then
      return notify("no agents in this Herdr workspace", vim.log.levels.WARN)
    end
    picker.pick(agents, function(choice)
      if not choice then
        return
      end
      -- the buffer/window may have changed while the picker was open; use the captured ones
      if not vim.api.nvim_win_is_valid(cap.win) then
        return notify("the original window is gone", vim.log.levels.WARN)
      end
      vim.api.nvim_set_current_win(cap.win)
      open_float(cap, ref, choice.pane_id, choice)
    end)
  end)
end

local function unmap_all()
  for _, m in ipairs(mapped) do
    pcall(vim.keymap.del, m[1], m[2])
  end
  mapped = {}
end

function M.setup(opts)
  unmap_all()
  M.opts = config.resolve(opts)
  M._setup_done = true
  float.define_highlights()
  vim.api.nvim_create_autocmd("ColorScheme", {
    group = vim.api.nvim_create_augroup("TetherHighlights", { clear = true }),
    callback = float.define_highlights,
  })
  local keys = M.opts.keys
  local function map(modes, lhs, fn, desc)
    if lhs and lhs ~= "" and lhs ~= false then
      vim.keymap.set(modes, lhs, fn, { desc = desc, silent = true })
      for _, mode in ipairs(modes) do
        table.insert(mapped, { mode, lhs })
      end
    end
  end
  map({ "n", "x" }, keys.comment, function()
    M.comment()
  end, "tether: comment on line/selection to the primary agent")
  map({ "n", "x" }, keys.comment_pick, function()
    M.comment_pick()
  end, "tether: comment on line/selection to a picked agent")
end

return M
