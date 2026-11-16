-- tether.float: the comment input. A bordered float drawn ON TOP of the code, anchored just
-- below the commented line (relative to the cursor), with its own highlight groups. No virtual
-- lines. Insert-mode keys: <CR> send, <Esc>/<C-c> cancel, <S-CR>/<C-j> newline.
local M = {}

local api = vim.api

function M.define_highlights()
  api.nvim_set_hl(0, "TetherFloat", { link = "NormalFloat", default = true })
  api.nvim_set_hl(0, "TetherFloatBorder", { link = "Special", default = true })
  api.nvim_set_hl(0, "TetherFloatTitle", { link = "Title", default = true })
  api.nvim_set_hl(0, "TetherFloatFooter", { link = "Comment", default = true })
end

local WINHL = table.concat({
  "Normal:TetherFloat",
  "NormalFloat:TetherFloat",
  "FloatBorder:TetherFloatBorder",
  "FloatTitle:TetherFloatTitle",
  "FloatFooter:TetherFloatFooter",
}, ",")

local FOOTER = { { " ⏎ send · esc cancel ", "TetherFloatFooter" } }

local function title_chunks(title)
  return { { " " .. title .. " ", "TetherFloatTitle" } }
end

--- Screen-row offset (relative to the cursor row) of the line just below `end_lnum`, taking wrapped
--- lines into account. Used so a visual-range comment floats under the LAST selected line.
function M.row_offset(win, end_lnum)
  local cur = api.nvim_win_get_cursor(win)
  local cur_pos = vim.fn.screenpos(win, cur[1], math.max(cur[2] + 1, 1))
  if cur_pos.row == 0 then
    return 1
  end
  local buf = api.nvim_win_get_buf(win)
  local last = api.nvim_buf_get_lines(buf, end_lnum - 1, end_lnum, false)[1] or ""
  local end_pos = vim.fn.screenpos(win, end_lnum, math.max(#last, 1))
  if end_pos.row == 0 then
    return 1
  end
  return math.max(end_pos.row - cur_pos.row, 0) + 1
end

--- Open the float.
--- opts: win (source window), end_lnum, title, on_submit(text), on_cancel(), width, max_height, border
--- Returns a handle { win, buf, set_title(t), close(), is_open() }.
function M.open(opts)
  M.define_highlights()
  local src_win = opts.win or api.nvim_get_current_win()
  local saved_cursor = api.nvim_win_get_cursor(src_win)
  local info = vim.fn.getwininfo(src_win)[1] or { textoff = 0 }
  local wincol = vim.fn.wincol()
  local avail = api.nvim_win_get_width(src_win) - (info.textoff or 0) - 2
  local width = math.max(20, math.min(opts.width or 64, avail))
  local max_height = opts.max_height or 10

  local buf = api.nvim_create_buf(false, true)
  vim.bo[buf].buftype = "nofile"
  vim.bo[buf].bufhidden = "wipe"
  vim.bo[buf].swapfile = false
  vim.bo[buf].filetype = "tetherinput"
  vim.b[buf].completion = false -- blink.cmp: no completion popups in the comment box

  local win = api.nvim_open_win(buf, true, {
    relative = "cursor",
    row = M.row_offset(src_win, opts.end_lnum or saved_cursor[1]),
    col = -(wincol - 1 - (info.textoff or 0)),
    width = width,
    height = 3,
    style = "minimal",
    border = opts.border or "rounded",
    title = title_chunks(opts.title or "comment"),
    title_pos = "left",
    footer = FOOTER,
    footer_pos = "right",
    zindex = 60,
  })
  vim.wo[win].winhighlight = WINHL
  vim.wo[win].wrap = true
  vim.wo[win].linebreak = true

  local closed = false
  local group = api.nvim_create_augroup("TetherFloat" .. buf, { clear = true })
  local handle = {}

  -- Closing happens in two steps. `stopinsert` only takes effect when the current mapping
  -- returns; if we switched windows first, Neovim's "leave insert" cursor adjustment (one column
  -- left) would hit the SOURCE window. So: request stopinsert, and finish (close the float, go
  -- back, restore the cursor) once insert mode has really ended.
  local finished = false
  local function finish()
    if finished then
      return
    end
    finished = true
    if api.nvim_win_is_valid(win) then
      pcall(api.nvim_win_close, win, true)
    end
    if api.nvim_win_is_valid(src_win) then
      pcall(api.nvim_set_current_win, src_win)
      pcall(api.nvim_win_set_cursor, src_win, saved_cursor)
    end
  end

  local function close()
    if closed then
      return
    end
    closed = true
    pcall(api.nvim_del_augroup_by_id, group)
    if api.nvim_get_mode().mode:sub(1, 1) == "i" then
      api.nvim_create_autocmd("ModeChanged", {
        pattern = "i*:*",
        once = true,
        callback = function()
          vim.schedule(finish)
        end,
      })
      vim.defer_fn(finish, 300) -- safety net
      vim.cmd("stopinsert")
    else
      finish()
    end
  end

  local function cancel()
    if closed then
      return
    end
    close()
    if opts.on_cancel then
      opts.on_cancel()
    end
  end

  local function submit()
    if closed then
      return
    end
    local text = table.concat(api.nvim_buf_get_lines(buf, 0, -1, false), "\n")
    text = text:gsub("%s+$", "")
    if text:match("^%s*$") then
      vim.notify("tether: empty comment, type something or <Esc> to cancel", vim.log.levels.INFO)
      return
    end
    close()
    opts.on_submit(text)
  end

  local function newline()
    -- noremap feed: inserts a real line break without re-triggering our <CR> mapping
    api.nvim_feedkeys(api.nvim_replace_termcodes("<CR>", true, false, true), "ni", false)
  end

  local function map(modes, lhs, fn)
    vim.keymap.set(modes, lhs, fn, { buffer = buf, nowait = true, silent = true })
  end
  map({ "i", "n" }, "<CR>", submit)
  map({ "i", "n" }, "<Esc>", cancel)
  map({ "i", "n" }, "<C-c>", cancel)
  map("i", "<S-CR>", newline)
  map("i", "<C-j>", newline) -- fallback where the terminal does not distinguish S-CR
  map("n", "q", cancel)

  -- grow with content (wrapped lines count), up to max_height
  local function resize()
    if closed or not api.nvim_win_is_valid(win) then
      return
    end
    local ok, th = pcall(api.nvim_win_text_height, win, {})
    local h = ok and th.all or api.nvim_buf_line_count(buf)
    api.nvim_win_set_config(win, { height = math.max(1, math.min(h, max_height)) })
  end
  api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, { group = group, buffer = buf, callback = resize })
  -- leaving the float by any other means (mouse click, :wincmd) cancels
  api.nvim_create_autocmd("WinLeave", {
    group = group,
    buffer = buf,
    callback = function()
      vim.schedule(cancel)
    end,
  })

  vim.cmd("startinsert")

  handle.win = win
  handle.buf = buf
  function handle.is_open()
    return not closed and api.nvim_win_is_valid(win)
  end
  function handle.set_title(t)
    if not closed and api.nvim_win_is_valid(win) then
      pcall(api.nvim_win_set_config, win, { title = title_chunks(t), title_pos = "left" })
    end
  end
  handle.close = close
  handle.cancel = cancel
  handle.submit = submit
  return handle
end

return M
