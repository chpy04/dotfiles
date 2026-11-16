-- tether.config: plugin options. Key bindings come from `tether config show --json` (read once,
-- cached), then opts passed to setup() override them.
local M = {}

M.defaults = {
  bin = "tether", -- the tether CLI
  read_config = true, -- shell out once to `tether config show --json` for key bindings
  keys = {
    comment = "<leader>ac", -- normal + visual: comment on line / range, to the primary agent
    comment_pick = "<leader>aC", -- same, but pick the agent first
  },
  target = "primary", -- default `--to` for comment
  float = {
    width = 64, -- content width (columns), clamped to the window
    max_height = 10,
    border = "rounded",
  },
  timeout_ms = 15000, -- kill a hanging `tether send`
}

local cached_cli_config -- table|false once read

--- Read (once) the effective tether config as a Lua table, or nil if tether is unavailable.
function M.read_cli_config(bin)
  if cached_cli_config ~= nil then
    return cached_cli_config or nil
  end
  cached_cli_config = false
  local ok, proc = pcall(vim.system, { bin, "config", "show", "--json" }, { text = true })
  if not ok or not proc then
    return nil
  end
  local ok2, res = pcall(function()
    return proc:wait(2000)
  end)
  if ok2 and res and res.code == 0 and res.stdout and res.stdout ~= "" then
    local ok3, decoded = pcall(vim.json.decode, res.stdout)
    if ok3 and type(decoded) == "table" then
      cached_cli_config = decoded
    end
  end
  return cached_cli_config or nil
end

function M.reset_cache()
  cached_cli_config = nil
end

--- Merge defaults <- tether config <- opts.
function M.resolve(opts)
  opts = opts or {}
  local merged = vim.deepcopy(M.defaults)
  if opts.read_config ~= nil then
    merged.read_config = opts.read_config
  end
  if opts.bin then
    merged.bin = opts.bin
  end
  if merged.read_config then
    local cli = M.read_cli_config(merged.bin)
    if cli and type(cli.keys) == "table" then
      for _, k in ipairs({ "comment", "comment_pick" }) do
        if type(cli.keys[k]) == "string" and cli.keys[k] ~= "" then
          merged.keys[k] = cli.keys[k]
        end
      end
    end
  end
  return vim.tbl_deep_extend("force", merged, opts)
end

return M
