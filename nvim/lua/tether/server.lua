-- tether.server: make THIS Neovim reachable by `tether code open` without `tether-nvim`.
-- Inside Herdr (HERDR_ENV=1, HERDR_PANE_ID set) it listens on the same socket the wrapper
-- `bin/tether-nvim` and `crates/tether-core/src/nvim.rs` use:
--     <runtime>/tether/<pane id with ':' -> '-'>.sock
-- with <runtime> = $XDG_RUNTIME_DIR, else $TMPDIR, else /tmp (trailing '/' stripped).
local M = {}

local uv = vim.uv or vim.loop

--- Socket path for `pane_id`, from `env` (a table; defaults to the process environment).
function M.socket_path(pane_id, env)
  env = env or vim.env
  local function get(k)
    local v = env[k]
    return (v ~= nil and v ~= "") and v or nil
  end
  local base = get("XDG_RUNTIME_DIR") or get("TMPDIR") or "/tmp"
  base = base:gsub("/+$", "")
  if base == "" then
    base = "/"
  end
  return ("%s/tether/%s.sock"):format(base, (pane_id:gsub(":", "-")))
end

--- Does something listen on `sock`? (connect only, no RPC handshake)
local function answers(sock)
  local ok, chan = pcall(vim.fn.sockconnect, "pipe", sock, { rpc = false })
  if ok and type(chan) == "number" and chan > 0 then
    pcall(vim.fn.chanclose, chan)
    return true
  end
  return false
end

local function in_herdr(env)
  env = env or vim.env
  return env.HERDR_ENV == "1" and env.HERDR_PANE_ID ~= nil and env.HERDR_PANE_ID ~= ""
end

local function listening_on(sock)
  for _, s in ipairs(vim.fn.serverlist()) do
    if s == sock then
      return true
    end
  end
  return false
end

--- Start listening on this pane's socket unless we already do. Returns the socket path when this
--- Neovim is (now) registered, nil otherwise (outside Herdr, or another live Neovim owns it).
function M.ensure()
  if not in_herdr() then
    return nil
  end
  local sock = M.socket_path(vim.env.HERDR_PANE_ID)
  local exists = uv.fs_stat(sock) ~= nil
  if vim.v.servername == sock and exists then
    return sock
  end
  if listening_on(sock) then
    if exists then
      return sock
    end
    -- the file vanished (macOS purges $TMPDIR): drop the dead listener and bind again
    pcall(vim.fn.serverstop, sock)
  end
  vim.fn.mkdir(vim.fn.fnamemodify(sock, ":h"), "p")
  if uv.fs_stat(sock) then
    if answers(sock) then
      return nil -- a live Neovim in this pane already owns the registration
    end
    uv.fs_unlink(sock) -- stale file from a crashed Neovim
  end
  local ok = pcall(vim.fn.serverstart, sock)
  if ok and listening_on(sock) then
    return sock
  end
  return nil
end

local registered_group

--- Register now, and again whenever the socket file disappears (FocusGained / VimResume).
function M.setup()
  if not in_herdr() then
    return
  end
  M.ensure()
  registered_group = vim.api.nvim_create_augroup("TetherServer", { clear = true })
  vim.api.nvim_create_autocmd({ "FocusGained", "VimResume" }, {
    group = registered_group,
    callback = function()
      local sock = M.socket_path(vim.env.HERDR_PANE_ID or "")
      if uv.fs_stat(sock) == nil then
        M.ensure()
      end
    end,
    desc = "tether: re-register the Neovim socket if it was purged",
  })
end

return M
