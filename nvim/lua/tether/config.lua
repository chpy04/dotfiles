-- tether.config: plugin options. Key bindings are defaults <- `tether config show --json` <- opts
-- passed to setup(). The CLI config is read ONLY inside Herdr (HERDR_ENV == "1") and never blocks:
-- startup maps the Lua defaults plus opts at once; the config is read in the background and, if it
-- defines other keys, they are remapped when it arrives (see init.lua `apply_cli_config`).
local M = {}

M.defaults = {
  bin = nil, -- the tether CLI; nil = find it (see M.find_bin)
  read_config = true, -- read key bindings from `tether config show --json` (inside Herdr only)
  keys = {
    comment = "<leader>ac", -- normal + visual: comment on line / range, to the primary agent
    comment_pick = "<leader>aC", -- same, but pick the agent first
    diagrams = "<leader>ad", -- pick a saved diagram of this repo and open it in a tab (:TetherDiagrams)
  },
  target = "primary", -- default `--to` for comment
  float = {
    width = 64, -- content width (columns), clamped to the window
    max_height = 10,
    border = "rounded",
  },
  timeout_ms = 15000, -- kill a hanging `tether send`
}

local uv = vim.uv or vim.loop

--- Directory of this plugin (the `nvim/` dir that holds lua/ and plugin/).
function M.plugin_root()
  local src = debug.getinfo(1, "S").source:sub(2)
  return vim.fn.fnamemodify(src, ":p:h:h:h")
end

local function is_exec(path)
  local st = uv.fs_stat(path)
  return st ~= nil and st.type == "file" and vim.fn.executable(path) == 1
end

--- Find the tether binary: the `bin` option, else `tether` on PATH, else the repo build next to the
--- plugin (`<plugin root>/../target/release/tether`, then `.../debug/tether`). Returns `bin` or
--- "tether" when nothing is found (the later failure then names the command).
--- `probe` = { root = dir, exepath = fn(name), is_exec = fn(path) } is for tests.
function M.find_bin(bin, probe)
  probe = probe or {}
  if bin and bin ~= "" then
    return bin
  end
  local exepath = probe.exepath or vim.fn.exepath
  local found = exepath("tether")
  if found and found ~= "" then
    return found
  end
  local root = probe.root or M.plugin_root()
  local exec = probe.is_exec or is_exec
  for _, profile in ipairs({ "release", "debug" }) do
    local cand = root .. "/../target/" .. profile .. "/tether"
    if exec(cand) then
      return (vim.fn.fnamemodify(cand, ":p"):gsub("/+$", ""))
    end
  end
  return "tether"
end

local cached_cli_config -- table|false once read; nil = not read yet
local pending -- callbacks waiting for the read in flight

--- Read the effective tether config in the background; cb(table|nil) is called on the main loop.
--- Cached: later calls answer from the cache. Never blocks.
function M.read_cli_config_async(bin, cb)
  if cached_cli_config ~= nil then
    return vim.schedule(function()
      cb(cached_cli_config or nil)
    end)
  end
  if pending then
    table.insert(pending, cb)
    return
  end
  pending = { cb }
  local function done(result)
    cached_cli_config = result or false
    local waiting = pending
    pending = nil
    for _, f in ipairs(waiting) do
      f(result)
    end
  end
  local ok = pcall(vim.system, { bin, "config", "show", "--json" }, { text = true, timeout = 3000 }, function(res)
    vim.schedule(function()
      local decoded
      if res.code == 0 and res.stdout and res.stdout ~= "" then
        local ok2, d = pcall(vim.json.decode, res.stdout)
        if ok2 and type(d) == "table" then
          decoded = d
        end
      end
      done(decoded)
    end)
  end)
  if not ok then
    vim.schedule(function()
      done(nil)
    end)
  end
end

function M.reset_cache()
  cached_cli_config = nil
  pending = nil
end

--- The key bindings a CLI config table asks for ({} when none).
function M.cli_keys(cli)
  local out = {}
  if type(cli) == "table" and type(cli.keys) == "table" then
    for _, k in ipairs({ "comment", "comment_pick", "diagrams" }) do
      if type(cli.keys[k]) == "string" and cli.keys[k] ~= "" then
        out[k] = cli.keys[k]
      end
    end
  end
  return out
end

--- Merge defaults <- (cli config, when given) <- opts. No process is spawned.
function M.resolve(opts, cli)
  opts = opts or {}
  local merged = vim.deepcopy(M.defaults)
  if opts.read_config ~= nil then
    merged.read_config = opts.read_config
  end
  for k, v in pairs(M.cli_keys(cli)) do
    merged.keys[k] = v
  end
  local out = vim.tbl_deep_extend("force", merged, opts)
  out.bin = M.find_bin(out.bin)
  return out
end

return M
