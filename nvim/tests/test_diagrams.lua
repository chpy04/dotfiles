-- Headless tests for :TetherDiagrams / <leader>ad. vim.system and vim.ui.select are stubbed: no
-- process is spawned, no picker is shown.
local pass, fail = 0, 0
local failures = {}

local function check(name, cond, detail)
  if cond then
    pass = pass + 1
    io.stdout:write("  ok   " .. name .. "\n")
  else
    fail = fail + 1
    table.insert(failures, name)
    io.stdout:write("  FAIL " .. name .. (detail and (" :: " .. tostring(detail)) or "") .. "\n")
  end
end

local function eq(name, got, want)
  check(name, vim.deep_equal(got, want), ("got %s, want %s"):format(vim.inspect(got), vim.inspect(want)))
end

local now = os.time()
local rows = {
  { id = "session-refresh-aaa111", title = "Session refresh", type = "flowchart", starred = true, branch = "feat-login", activity = now - 2 * 86400 },
  { id = "login-request-bbb222", title = "Login request", type = "sequence", starred = false, branch = vim.NIL, activity = now - 300 },
}

local calls = {} -- { argv =, opts = }
local notes = {}
local list_result
local view_result
local selects = {} -- { items =, opts = }
local choose = nil -- index to choose in the next select (nil = cancel)

vim.system = function(argv, opts, on_exit)
  table.insert(calls, { argv = vim.deepcopy(argv), opts = vim.deepcopy(opts) })
  local res
  if argv[2] == "diagram" and argv[3] == "list" then
    res = list_result
  elseif argv[2] == "view" then
    res = view_result
  elseif argv[2] == "config" then
    res = { code = 0, stdout = "{}", stderr = "" }
  else
    res = { code = 1, stdout = "", stderr = "unexpected" }
  end
  if on_exit then
    vim.schedule(function()
      on_exit(res)
    end)
  end
  return {
    wait = function()
      return res
    end,
  }
end
vim.notify = function(msg, level)
  table.insert(notes, { msg = msg, level = level })
end
vim.ui.select = function(items, opts, on_choice)
  table.insert(selects, { items = items, opts = opts })
  vim.schedule(function()
    on_choice(choose and items[choose] or nil, choose)
  end)
end

local function reset()
  calls, notes, selects = {}, {}, {}
  choose = nil
  list_result = { code = 0, stdout = vim.json.encode(rows), stderr = "" }
  view_result = { code = 0, stdout = 'opened tab "Session refresh" (w1:t7)\n', stderr = "" }
end

local co
local function resume()
  local ok, err = coroutine.resume(co)
  if not ok then
    io.stderr:write("TEST HARNESS ERROR: " .. tostring(err) .. "\n")
    vim.cmd("cquit 2")
  end
end
local function settle(ms)
  vim.defer_fn(resume, ms or 80)
  coroutine.yield()
end

local function views()
  local out = {}
  for _, c in ipairs(calls) do
    if c.argv[2] == "view" then
      table.insert(out, c)
    end
  end
  return out
end

local function main()
  require("tether.config").find_bin = function(b)
    return b or "tether"
  end
  local D = require("tether.diagrams")
  local tether = require("tether")

  print("# formatting")
  eq("age scale", {
    D.age(1000, 1000),
    D.age(1000 + 300, 1000),
    D.age(1000 + 3 * 3600, 1000),
    D.age(1000 + 2 * 86400, 1000),
    D.age(1000 + 21 * 86400, 1000),
    D.age(1000 + 120 * 86400, 1000),
    D.age(1000 + 800 * 86400, 1000),
  }, { "now", "5m", "3h", "2d", "3w", "4mo", "2y" })
  eq("starred row", D.format_item(rows[1], now), "★ Session refresh · flowchart · feat-login · 2d")
  eq("unstarred row without a branch keeps alignment", D.format_item(rows[2], now), "  Login request · sequence · 5m")
  eq("parse accepts the CLI array", #D.parse(vim.json.encode(rows)), 2)
  check("parse rejects garbage", D.parse("not json") == nil)

  print("# setup, mapping and command")
  vim.env.HERDR_ENV = "1"
  reset()
  tether.setup({ read_config = false })
  check("default mapping <leader>ad", vim.fn.maparg(" ad", "n") ~= "")
  check(":TetherDiagrams exists", vim.fn.exists(":TetherDiagrams") == 2)
  eq("config key default", require("tether.config").defaults.keys.diagrams, "<leader>ad")
  eq("a CLI config can rebind it", require("tether.config").resolve({}, { keys = { diagrams = "<leader>qd" } }).keys.diagrams, "<leader>qd")
  tether.setup({ read_config = false, keys = { diagrams = "<leader>zd" } })
  check("configurable via keys.diagrams", vim.fn.maparg(" zd", "n") ~= "" and vim.fn.maparg(" ad", "n") == "")
  tether.setup({ read_config = false, keys = { diagrams = false } })
  eq("can be disabled", vim.fn.maparg(" ad", "n"), "")
  tether.setup({ read_config = false })

  print("# outside Herdr: a clear error, nothing spawned")
  reset()
  vim.env.HERDR_ENV = nil
  tether.diagrams()
  settle()
  eq("no process", #calls, 0)
  eq("no picker", #selects, 0)
  check("error notification", notes[1] and notes[1].level == vim.log.levels.ERROR and notes[1].msg:find("not inside Herdr", 1, true) ~= nil, vim.inspect(notes))
  vim.env.HERDR_ENV = "1"

  print("# pick and open")
  reset()
  choose = 1
  vim.cmd("TetherDiagrams")
  eq("returns before anything finished (async): no picker yet", #selects, 0)
  eq("lists with `tether diagram list --json`", calls[1] and calls[1].argv, { "tether", "diagram", "list", "--json" })
  eq("in the editor's cwd", calls[1] and calls[1].opts.cwd, vim.fn.getcwd())
  settle()
  eq("one picker", #selects, 1)
  eq("prompt", selects[1].opts.prompt, "Open diagram")
  eq("items are the rows, in order", { selects[1].items[1].id, selects[1].items[2].id }, { rows[1].id, rows[2].id })
  eq("first item text", selects[1].opts.format_item(selects[1].items[1]), "★ Session refresh · flowchart · feat-login · 2d")
  eq("second item text", selects[1].opts.format_item(selects[1].items[2]), "  Login request · sequence · 5m")
  eq("one view call", #views(), 1)
  eq("opens in a tab", views()[1].argv, { "tether", "view", "session-refresh-aaa111", "--placement", "tab" })
  eq("with the same cwd", views()[1].opts.cwd, vim.fn.getcwd())
  check("success is reported", notes[#notes] and notes[#notes].level == vim.log.levels.INFO and notes[#notes].msg:find("opened tab", 1, true) ~= nil, vim.inspect(notes))

  print("# second row, key mapping")
  reset()
  choose = 2
  vim.api.nvim_feedkeys(vim.keycode("<leader>ad"), "x", false)
  settle()
  eq("<leader>ad opens the picker", #selects, 1)
  eq("view of the chosen id", views()[1] and views()[1].argv[3], "login-request-bbb222")

  print("# cancel")
  reset()
  tether.diagrams()
  settle()
  eq("picker shown", #selects, 1)
  eq("cancel opens nothing", #views(), 0)

  print("# failures are reported, not swallowed")
  reset()
  choose = 1
  view_result = { code = 3, stdout = "", stderr = "tether: error: not inside Herdr (HERDR_ENV is not set)\n" }
  tether.diagrams()
  settle()
  local n = notes[#notes]
  check("view failure is an error notification", n and n.level == vim.log.levels.ERROR and n.msg:find("Session refresh", 1, true) and n.msg:find("not inside Herdr", 1, true), vim.inspect(notes))

  reset()
  list_result = { code = 1, stdout = "", stderr = "tether: error: cannot determine cwd\n" }
  tether.diagrams()
  settle()
  eq("no picker when the list failed", #selects, 0)
  n = notes[#notes]
  check("list failure notified", n and n.level == vim.log.levels.ERROR and n.msg:find("cannot determine cwd", 1, true) ~= nil, vim.inspect(notes))

  reset()
  list_result = { code = 0, stdout = "<<garbage>>", stderr = "" }
  tether.diagrams()
  settle()
  n = notes[#notes]
  check("unparseable list notified", n and n.level == vim.log.levels.ERROR and n.msg:find("could not parse", 1, true) ~= nil, vim.inspect(notes))

  reset()
  list_result = { code = 0, stdout = "[]", stderr = "" }
  tether.diagrams()
  settle()
  eq("no picker for an empty library", #selects, 0)
  n = notes[#notes]
  check("friendly empty message", n and n.level == vim.log.levels.WARN and n.msg:find("no saved diagrams", 1, true) ~= nil, vim.inspect(notes))

  reset()
  choose = 1
  view_result = { code = 124, stdout = "", stderr = "" }
  tether.diagrams()
  settle()
  n = notes[#notes]
  check("a timeout is described as one", n and n.msg:find("timed out", 1, true) ~= nil, vim.inspect(notes))

  io.stdout:write(("\nneovim diagram tests: %d passed, %d failed\n"):format(pass, fail))
  if fail > 0 then
    io.stdout:write("failed: " .. table.concat(failures, "; ") .. "\n")
  end
  vim.cmd(fail > 0 and "cquit 1" or "qall!")
end

co = coroutine.create(main)
resume()
