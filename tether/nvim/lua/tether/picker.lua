-- tether.picker: choose an agent with vim.ui.select (uses whatever picker the user configured).
local M = {}

--- `tab 1 · claude — idle`
function M.format_item(a)
  return ("tab %s · %s — %s"):format(a.tab_index, a.kind, a.state)
end

--- Parse `tether agents --json` stdout.
function M.parse(stdout)
  local ok, decoded = pcall(vim.json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    return nil, "could not parse `tether agents --json` output"
  end
  return decoded
end

--- Pick one agent. cb(agent|nil).
function M.pick(agents, cb)
  vim.ui.select(agents, {
    prompt = "Send comment to",
    format_item = M.format_item,
  }, function(choice)
    cb(choice)
  end)
end

return M
