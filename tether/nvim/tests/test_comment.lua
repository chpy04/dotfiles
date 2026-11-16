-- Headless tests for the tether Neovim plugin. vim.system is stubbed: no process is spawned.
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

-- ---- stubs ---------------------------------------------------------------------------------
local calls = {} -- every vim.system argv
local notes = {} -- every vim.notify {msg, level}
local agents_json = vim.json.encode({
  { tab_index = 1, tab_label = "1", tab_id = "w1:t1", pane_id = "w1:p2", kind = "claude", state = "idle", is_primary = true },
  { tab_index = 3, tab_label = "tests", tab_id = "w1:t3", pane_id = "w1:p5", kind = "codex", state = "blocked", is_primary = false },
})
local cfg_json = vim.json.encode({ keys = { comment = "<leader>ac", comment_pick = "<leader>aC" } })
local send_result = { code = 0, stdout = "sent to claude · tab 1 (w1:p2, idle)\n", stderr = "" }

vim.system = function(argv, opts, on_exit)
  table.insert(calls, vim.deepcopy(argv))
  local res
  if argv[2] == "config" then
    res = { code = 0, stdout = cfg_json, stderr = "" }
  elseif argv[2] == "agents" then
    res = { code = 0, stdout = agents_json, stderr = "" }
  elseif argv[2] == "send" then
    res = send_result
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

local function sends()
  local out = {}
  for _, c in ipairs(calls) do
    if c[2] == "send" then
      table.insert(out, c)
    end
  end
  return out
end

local function floats()
  local out = {}
  for _, w in ipairs(vim.api.nvim_list_wins()) do
    local c = vim.api.nvim_win_get_config(w)
    if c.relative ~= "" then
      table.insert(out, { win = w, cfg = c })
    end
  end
  return out
end

-- The test body runs in a coroutine; settle() yields back to Neovim's main loop (which then
-- consumes typed keys, runs startinsert, fires vim.schedule callbacks) and resumes via a timer.
local co
local function resume()
  local ok, err = coroutine.resume(co)
  if not ok then
    io.stderr:write("TEST HARNESS ERROR: " .. tostring(err) .. "\n")
    vim.cmd("cquit 2")
  end
end

local function settle(ms)
  vim.defer_fn(resume, ms or 120)
  coroutine.yield()
end

local function feed(keys)
  vim.api.nvim_feedkeys(vim.keycode(keys), "t", false)
  settle(120)
end

-- record the config each float is REQUESTED with (nvim converts relative=cursor to a
-- window-relative position at creation time, so get_config can't show it)
local open_requests = {}
local real_open_win = vim.api.nvim_open_win
vim.api.nvim_open_win = function(buf, enter, cfg)
  table.insert(open_requests, vim.deepcopy(cfg))
  return real_open_win(buf, enter, cfg)
end

local function reset()
  calls, notes, open_requests = {}, {}, {}
  send_result = { code = 0, stdout = "sent to claude · tab 1 (w1:p2, idle)\n", stderr = "" }
end

local function title_text(cfg)
  local t = cfg.title
  if type(t) == "table" then
    local s = {}
    for _, chunk in ipairs(t) do
      table.insert(s, type(chunk) == "table" and chunk[1] or chunk)
    end
    return table.concat(s)
  end
  return tostring(t)
end

local function main()
-- the binary lookup is tested on its own below; here it is always plain `tether`
require("tether.config").find_bin = function(b)
  return b or "tether"
end
-- ---- fixture file --------------------------------------------------------------------------
local dir = vim.fn.tempname()
vim.fn.mkdir(dir, "p")
local file = dir .. "/sample.txt"
local lines = {}
for i = 1, 20 do
  lines[i] = "line " .. i
end
vim.fn.writefile(lines, file)
vim.cmd("edit " .. vim.fn.fnameescape(file))
local path = vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ":p") -- nvim resolves /var -> /private/var
local src_win = vim.api.nvim_get_current_win()

vim.env.HERDR_ENV = "1"
vim.env.HERDR_WORKSPACE_ID = "w1"
local tether = require("tether")

-- ---- setup / config ------------------------------------------------------------------------
print("# setup")
tether.setup()
-- keys are mapped at once from the Lua defaults; nothing waited for the CLI
check("normal-mode mapping from defaults, immediately", vim.fn.maparg(" ac", "n") ~= "")
check("visual-mode mapping from defaults, immediately", vim.fn.maparg(" ac", "x") ~= "")
check("picker mapping", vim.fn.maparg(" aC", "n") ~= "")
settle()
eq("config read in the background via `tether config show --json`", calls[1], { "tether", "config", "show", "--json" })
eq("exactly one config shell-out", #calls, 1)
check("highlight groups defined", vim.fn.hlexists("TetherFloat") == 1 and vim.fn.hlexists("TetherFloatBorder") == 1 and vim.fn.hlexists("TetherFloatTitle") == 1)

print("# config keys arrive asynchronously")
reset()
require("tether.config").reset_cache()
cfg_json = vim.json.encode({ keys = { comment = "<leader>qq", comment_pick = "<leader>qQ" } })
tether.setup()
check("defaults mapped before the config arrives", vim.fn.maparg(" ac", "n") ~= "" and vim.fn.maparg(" qq", "n") == "")
settle()
check("keys from the tether config remap when it arrives", vim.fn.maparg(" qq", "n") ~= "" and vim.fn.maparg(" qQ", "x") ~= "")
eq("old default mappings removed on remap", vim.fn.maparg(" ac", "n"), "")
cfg_json = vim.json.encode({ keys = { comment = "<leader>ac", comment_pick = "<leader>aC" } })
require("tether.config").reset_cache()
tether.setup()
settle()
check("back to the defaults", vim.fn.maparg(" ac", "n") ~= "" and vim.fn.maparg(" qq", "n") == "")

print("# outside Herdr nothing is spawned at startup")
reset()
vim.env.HERDR_ENV = nil
require("tether.config").reset_cache()
tether.setup()
settle()
eq("no shell-out outside Herdr", #calls, 0)
check("defaults still mapped outside Herdr", vim.fn.maparg(" ac", "n") ~= "")
vim.env.HERDR_ENV = "1"
require("tether.config").reset_cache()
tether.setup()
settle()

print("# binary lookup")
do
  local cfg = require("tether.config")
  local real_find = cfg.find_bin
  -- (the stub above replaced find_bin; the original is the module's own function, re-require it)
  package.loaded["tether.config"] = nil
  local fresh = require("tether.config")
  eq("bin option wins", fresh.find_bin("/x/mytether", { exepath = function() return "/usr/bin/tether" end }), "/x/mytether")
  eq("PATH next", fresh.find_bin(nil, { exepath = function() return "/usr/bin/tether" end }), "/usr/bin/tether")
  local seen = {}
  local function none(_) return "" end
  eq("release build next to the plugin", fresh.find_bin(nil, { exepath = none, root = "/r/nvim", is_exec = function(p) table.insert(seen, p) return p:find("release", 1, true) ~= nil end }):gsub("^/r/nvim/%.%./", ""), "target/release/tether")
  eq("debug build last", (fresh.find_bin(nil, { exepath = none, root = "/r/nvim", is_exec = function(p) return p:find("debug", 1, true) ~= nil end }):gsub("^/r/nvim/%.%./", "")), "target/debug/tether")
  eq("nothing found: plain tether", fresh.find_bin(nil, { exepath = none, root = "/r/nvim", is_exec = function() return false end }), "tether")
  package.loaded["tether.config"] = cfg
  cfg.find_bin = real_find
end

-- ---- normal mode: open float, type, send ----------------------------------------------------
print("# normal mode send")
reset()
vim.api.nvim_win_set_cursor(0, { 3, 2 })
feed("<leader>ac")
settle()
local fl = floats()
eq("one float opened", #fl, 1)
local f = fl[1] and fl[1].cfg or {}
check("float is bordered", type(f.border) == "table" and #f.border > 0 and f.border[1] ~= "", vim.inspect(f.border))
eq("float requested relative to the cursor", open_requests[1] and open_requests[1].relative, "cursor")
eq("float requested just below the cursor line", open_requests[1] and open_requests[1].row, 1)
eq("float is attached to the source window", f.win, src_win)
eq("float sits on the row below the cursor", f.row, vim.api.nvim_win_call(src_win, vim.fn.winline))
check("title is `→ claude · tab 1`", title_text(f):find("→ claude · tab 1", 1, true) ~= nil, title_text(f))
check("footer has send/cancel hints", vim.inspect(f.footer):find("send", 1, true) ~= nil and vim.inspect(f.footer):find("cancel", 1, true) ~= nil)
eq("float starts in insert mode", vim.fn.mode(), "i")
check("float uses own highlight groups", vim.wo[fl[1].win].winhighlight:find("TetherFloatBorder", 1, true) ~= nil)
check("float is above the code (zindex)", (f.zindex or 0) >= 50)
eq("agents resolved asynchronously for the title", calls[1], { "tether", "agents", "--json" })
-- the code buffer must be untouched (no virtual lines / extmarks)
local ns_all = vim.api.nvim_get_namespaces()
local marks = 0
for _, id in pairs(ns_all) do
  marks = marks + #vim.api.nvim_buf_get_extmarks(vim.fn.bufnr(file), id, 0, -1, {})
end
eq("no extmarks/virtual lines in the code buffer", marks, 0)

feed("why client clock?<CR>")
settle()
eq("exactly one send", #sends(), 1)
eq("send argv", sends()[1], { "tether", "send", "--to", "primary", "--ref", path .. ":3", "--text", "why client clock?" })
eq("float closed after send", #floats(), 0)
eq("back in the source window", vim.api.nvim_get_current_win(), src_win)
eq("cursor restored", vim.api.nvim_win_get_cursor(0), { 3, 2 })
eq("back in normal mode", vim.fn.mode(), "n")
local last = notes[#notes]
check("success notification", last ~= nil and last.msg:find("sent to claude", 1, true) ~= nil and last.level == vim.log.levels.INFO, vim.inspect(notes))

-- ---- newline keys ------------------------------------------------------------------------------
print("# newlines")
reset()
vim.api.nvim_win_set_cursor(0, { 4, 0 })
feed("<leader>ac")
settle()
feed("first<S-CR>second<C-j>third<CR>")
settle()
eq("S-CR and C-j insert newlines, CR sends", sends()[1] and sends()[1][8], "first\nsecond\nthird")
eq("ref for line 4", sends()[1] and sends()[1][6], path .. ":4")

-- ---- visual mode -------------------------------------------------------------------------------
print("# visual mode")
reset()
vim.api.nvim_win_set_cursor(0, { 6, 0 })
feed("Vjj") -- lines 6..8
feed("<leader>ac")
settle()
fl = floats()
eq("float opened for visual range", #fl, 1)
eq("cursor at the end of the range -> float 1 row below", open_requests[1] and open_requests[1].row, 1)
feed("range q<CR>")
settle()
eq("visual send has a range ref", sends()[1] and sends()[1][6], path .. ":6-8")
eq("visual send text", sends()[1] and sends()[1][8], "range q")
eq("no longer in visual mode", vim.fn.mode(), "n")
eq("cursor stays at the visual cursor end", vim.api.nvim_win_get_cursor(0)[1], 8)

reset()
vim.api.nvim_win_set_cursor(0, { 9, 0 })
feed("vl") -- single-line charwise selection
feed("<leader>ac")
settle()
feed("one line<CR>")
settle()
eq("single-line visual selection gives a plain line ref", sends()[1] and sends()[1][6], path .. ":9")

-- bottom-up selection (cursor above anchor)
reset()
vim.api.nvim_win_set_cursor(0, { 12, 0 })
feed("Vkk") -- cursor on line 10, anchor on 12
feed("<leader>ac")
settle()
eq("range anchored from the top: float is placed below the last selected line", open_requests[1] and open_requests[1].row, 3)
feed("up<CR>")
settle()
eq("reverse selection is normalised", sends()[1] and sends()[1][6], path .. ":10-12")

-- ---- cancel ------------------------------------------------------------------------------------
print("# cancel")
reset()
vim.api.nvim_win_set_cursor(0, { 5, 1 })
feed("<leader>ac")
settle()
eq("float open", #floats(), 1)
feed("some text<Esc>")
settle()
eq("Esc closes the float", #floats(), 0)
eq("Esc sends nothing", #sends(), 0)
eq("source window focused after cancel", vim.api.nvim_get_current_win(), src_win)
eq("cursor restored after cancel", vim.api.nvim_win_get_cursor(0), { 5, 1 })

reset()
feed("<leader>ac")
settle()
feed("<CR>")
settle()
eq("empty comment does not send", #sends(), 0)
eq("float stays open on empty enter", #floats(), 1)
feed("<Esc>")
settle()
eq("then Esc closes it", #floats(), 0)

-- ---- send failures / warnings ----------------------------------------------------------------------
print("# failure reporting")
reset()
send_result = { code = 6, stdout = "", stderr = "tether: error: agent w1:p2 is blocked (herdr error: agent_blocked)\ntether: hint: answer it first\n" }
vim.api.nvim_win_set_cursor(0, { 2, 0 })
feed("<leader>ac")
settle()
feed("hi<CR>")
settle()
last = notes[#notes]
check("failure is reported as an error with tether's stderr", last ~= nil and last.level == vim.log.levels.ERROR and last.msg:find("agent_blocked", 1, true) ~= nil and last.msg:find("exit 6", 1, true) ~= nil, vim.inspect(notes))
eq("float closed on failure too", #floats(), 0)
eq("the comment text is saved in register t", vim.fn.getreg("t"), "hi")
check("error says where the text went", last.msg:find("register t", 1, true) ~= nil, last.msg)

reset()
send_result = { code = 124, stdout = "", stderr = "" }
feed("<leader>ac")
settle()
feed("slow one<CR>")
settle()
last = notes[#notes]
check("exit 124 is a timeout, not an unknown error", last ~= nil and last.msg:find("timed out", 1, true) ~= nil and last.msg:find("unknown", 1, true) == nil, vim.inspect(notes))
eq("timeout keeps the text in register t", vim.fn.getreg("t"), "slow one")
reset()
send_result = { code = 1, stdout = "", stderr = "  \n" }
feed("<leader>ac")
settle()
feed("silent one<CR>")
settle()
last = notes[#notes]
check("empty stderr is reported as no output", last ~= nil and last.msg:find("no output", 1, true) ~= nil and last.msg:find("unknown", 1, true) == nil, vim.inspect(notes))
eq("no-output failure keeps the text in register t", vim.fn.getreg("t"), "silent one")
reset()
send_result = { code = 0, stdout = "sent to claude · tab 1 (w1:p2, idle)\n", stderr = "" }
vim.fn.setreg("t", "untouched")
feed("<leader>ac")
settle()
feed("fine<CR>")
settle()
eq("register t is left alone on success", vim.fn.getreg("t"), "untouched")

reset()
send_result = { code = 0, stdout = "sent to claude · tab 1 (w1:p2, working)\n", stderr = "tether: warning: agent is blocked\n" }
feed("<leader>ac")
settle()
feed("hi<CR>")
settle()
last = notes[#notes]
check("warnings surface as WARN", last ~= nil and last.level == vim.log.levels.WARN and last.msg:find("blocked", 1, true) ~= nil, vim.inspect(notes))

-- ---- completion / AI suggestions off, Enter survives other plugins ------------------------------
print("# input buffer hygiene")
reset()
local cmp_opts
package.loaded.cmp = { setup = { buffer = function(o) cmp_opts = o end } }
local grp = vim.api.nvim_create_augroup("FakeAutopairs", { clear = true })
-- a global insert-mode <CR> map plus a plugin that grabs <CR> buffer-locally on InsertEnter
local global_cr = 0
vim.keymap.set("i", "<CR>", function() global_cr = global_cr + 1 end, {})
vim.api.nvim_create_autocmd("InsertEnter", { group = grp, callback = function(a)
  vim.keymap.set("i", "<CR>", function() global_cr = global_cr + 1 end, { buffer = a.buf })
end })
vim.api.nvim_win_set_cursor(0, { 3, 0 })
feed("<leader>ac")
settle()
fl = floats()
eq("float open", #fl, 1)
local ibuf = fl[1] and vim.api.nvim_win_get_buf(fl[1].win) or -1
eq("filetype is tetherinput", vim.bo[ibuf].filetype, "tetherinput")
eq("blink completion off", vim.b[ibuf].completion, false)
eq("copilot disabled", vim.b[ibuf].copilot_enabled, false)
eq("copilot auto trigger off", vim.b[ibuf].copilot_suggestion_auto_trigger, false)
eq("cmp disabled for the buffer", cmp_opts, { enabled = false })
feed("hello enter<CR>")
settle()
eq("Enter still sends despite global + InsertEnter <CR> maps", #sends(), 1)
eq("the other plugins' <CR> maps never ran", global_cr, 0)
eq("sent text", sends()[1] and sends()[1][8], "hello enter")
vim.api.nvim_del_augroup_by_id(grp)
pcall(vim.keymap.del, "i", "<CR>")
package.loaded.cmp = nil

-- ---- picker ------------------------------------------------------------------------------------
print("# picker")
reset()
local seen_items, seen_labels, seen_prompt
vim.ui.select = function(items, o, cb)
  seen_items = items
  seen_prompt = o.prompt
  seen_labels = {}
  for _, it in ipairs(items) do
    table.insert(seen_labels, o.format_item(it))
  end
  cb(items[2])
end
vim.api.nvim_win_set_cursor(0, { 7, 0 })
feed("<leader>aC")
settle(200)
eq("picker lists every agent", #(seen_items or {}), 2)
eq("picker labels", seen_labels, { "tab 1 · claude — idle", "tab 3 · codex — blocked" })
eq("picker prompt", seen_prompt, "Send comment to")
fl = floats()
eq("float opens after picking", #fl, 1)
check("title names the picked agent", fl[1] and title_text(fl[1].cfg):find("→ codex · tab 3", 1, true) ~= nil, fl[1] and title_text(fl[1].cfg))
feed("to codex<CR>")
settle()
eq("picked pane id is the --to target", sends()[1], { "tether", "send", "--to", "w1:p5", "--ref", path .. ":7", "--text", "to codex" })
eq("cursor restored after picker flow", vim.api.nvim_win_get_cursor(0), { 7, 0 })

-- picker cancelled -> no float
reset()
vim.ui.select = function(_, _, cb)
  cb(nil)
end
feed("<leader>aC")
settle(200)
eq("cancelled picker opens no float", #floats(), 0)
eq("cancelled picker sends nothing", #sends(), 0)

-- ---- outside Herdr -------------------------------------------------------------------------------------
print("# outside Herdr")
reset()
vim.env.HERDR_ENV = nil
feed("<leader>ac")
settle()
eq("no float outside Herdr", #floats(), 0)
last = notes[#notes]
check("clear error outside Herdr", last ~= nil and last.level == vim.log.levels.ERROR and last.msg:find("not inside Herdr", 1, true) ~= nil, vim.inspect(notes))
eq("no tether call outside Herdr", #calls, 0)
reset()
feed("<leader>aC")
settle()
eq("picker also errors outside Herdr", #floats() + #calls, 0)
check("picker error notified", notes[#notes] ~= nil and notes[#notes].level == vim.log.levels.ERROR)
vim.env.HERDR_ENV = "1"

-- ---- unnamed buffer ----------------------------------------------------------------------------------------
print("# buffers without a file")
reset()
vim.cmd("enew")
feed("<leader>ac")
settle()
eq("no float for a nameless buffer", #floats(), 0)
check("explains why", notes[#notes] ~= nil and notes[#notes].msg:find("no file name", 1, true) ~= nil, vim.inspect(notes))
vim.cmd("bwipeout!")

-- ---- key overrides (opts beat tether config) -------------------------------------------------------------
print("# key overrides")
reset()
require("tether.config").reset_cache()
tether.setup({ keys = { comment = "<leader>zz", comment_pick = false } })
check("opts override tether config keys", vim.fn.maparg(" zz", "n") ~= "")
eq("old mapping removed on re-setup", vim.fn.maparg(" ac", "n"), "")
eq("picker mapping can be disabled", vim.fn.maparg(" aC", "n"), "")
settle()
check("opts still win after the config arrives", vim.fn.maparg(" zz", "n") ~= "" and vim.fn.maparg(" ac", "n") == "" and vim.fn.maparg(" aC", "n") == "")
tether.setup({ read_config = false })
check("read_config=false keeps defaults without shelling out", vim.fn.maparg(" ac", "n") ~= "" and #calls == 1)

-- ---- results ---------------------------------------------------------------------------------------------------
io.stdout:write(("\nneovim plugin tests: %d passed, %d failed\n"):format(pass, fail))
if fail > 0 then
  io.stdout:write("failed: " .. table.concat(failures, "; ") .. "\n")
end
vim.cmd(fail > 0 and "cquit 1" or "qall!")
end

co = coroutine.create(main)
resume()
