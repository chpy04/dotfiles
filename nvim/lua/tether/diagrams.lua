-- tether.diagrams: pick one of this repo's saved diagrams (`tether diagram list --json`) with
-- vim.ui.select; the choice opens in a tether tab (`tether view <id> --placement tab`).
local M = {}

--- Compact relative age, the same scale as `tether diagram list`: now, 5m, 3h, 2d, 6w, 4mo, 2y.
function M.age(now, then_)
  local s = math.max(0, (now or 0) - (then_ or 0))
  local m, h, d = 60, 3600, 86400
  if s < m then
    return "now"
  elseif s < h then
    return ("%dm"):format(math.floor(s / m))
  elseif s < d then
    return ("%dh"):format(math.floor(s / h))
  elseif s < 14 * d then
    return ("%dd"):format(math.floor(s / d))
  elseif s < 63 * d then
    return ("%dw"):format(math.floor(s / (7 * d)))
  elseif s < 365 * d then
    return ("%dmo"):format(math.floor(s / (30 * d)))
  end
  return ("%dy"):format(math.floor(s / (365 * d)))
end

--- `★ Session refresh · flowchart · feat-login · 2d` (unstarred rows get two spaces instead of the star).
function M.format_item(d, now)
  local parts = { d.title or d.id, d.type or "?" }
  if type(d.branch) == "string" and d.branch ~= "" then
    table.insert(parts, d.branch)
  end
  local when = d.activity or d.updated or d.modified
  if type(when) == "number" then
    table.insert(parts, M.age(now or os.time(), when))
  end
  return (d.starred and "★ " or "  ") .. table.concat(parts, " · ")
end

--- Parse `tether diagram list --json` stdout: the array, or nil + a reason.
function M.parse(stdout)
  local ok, decoded = pcall(vim.json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    return nil, "could not parse `tether diagram list --json` output"
  end
  return decoded
end

--- Pick one diagram. cb(row|nil).
function M.pick(rows, cb, now)
  vim.ui.select(rows, {
    prompt = "Open diagram",
    format_item = function(d)
      return M.format_item(d, now)
    end,
  }, function(choice)
    cb(choice)
  end)
end

return M
