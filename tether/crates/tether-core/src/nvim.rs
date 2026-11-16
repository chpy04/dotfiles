//! Neovim side: the socket-path convention (`<runtime>/tether/<pane>.sock`, created by
//! `bin/tether-nvim`) and a small native msgpack-rpc client (`nvim_exec_lua`) used to jump a
//! running Neovim to `file:line` (Phase 2, recipe from `spike/FINDINGS.md`).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Runtime dir for tether sockets/state: `$XDG_RUNTIME_DIR`, else `$TMPDIR`, else `/tmp`,
/// with trailing slashes stripped (macOS `$TMPDIR` ends in `/`).
pub fn runtime_dir() -> PathBuf {
    runtime_dir_from(|k| std::env::var_os(k))
}

/// `runtime_dir` with an injectable environment (for tests).
pub fn runtime_dir_from(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let base = ["XDG_RUNTIME_DIR", "TMPDIR"]
        .iter()
        .filter_map(|k| get(k))
        .find(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    strip_trailing_slashes(&base)
}

fn strip_trailing_slashes(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        PathBuf::from("/")
    } else {
        PathBuf::from(t)
    }
}

/// `<runtime>/tether`.
pub fn socket_dir() -> PathBuf {
    runtime_dir().join("tether")
}

/// `<runtime>/tether/<pane-id with ':' replaced by '-'>.sock`. The colon must go: nvim parses
/// `a:b` in `--listen`/`--server` as host:port.
pub fn socket_path(pane_id: &str) -> PathBuf {
    socket_path_in(&runtime_dir(), pane_id)
}

pub fn socket_path_in(runtime: &Path, pane_id: &str) -> PathBuf {
    runtime.join("tether").join(format!("{}.sock", pane_id.replace(':', "-")))
}

// ---------------------------------------------------------------------------------------------
// Minimal msgpack (only what msgpack-rpc with Neovim needs), mapped onto serde_json::Value
// ---------------------------------------------------------------------------------------------

mod mp {
    use serde_json::Value;

    pub fn encode(v: &Value, out: &mut Vec<u8>) {
        match v {
            Value::Null => out.push(0xc0),
            Value::Bool(false) => out.push(0xc2),
            Value::Bool(true) => out.push(0xc3),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    int(i, out)
                } else if let Some(u) = n.as_u64() {
                    out.push(0xcf);
                    out.extend_from_slice(&u.to_be_bytes());
                } else {
                    out.push(0xcb);
                    out.extend_from_slice(&n.as_f64().unwrap_or(0.0).to_be_bytes());
                }
            }
            Value::String(s) => {
                let b = s.as_bytes();
                match b.len() {
                    n if n < 32 => out.push(0xa0 | n as u8),
                    n if n < 256 => out.extend_from_slice(&[0xd9, n as u8]),
                    n if n < 65536 => {
                        out.push(0xda);
                        out.extend_from_slice(&(n as u16).to_be_bytes());
                    }
                    n => {
                        out.push(0xdb);
                        out.extend_from_slice(&(n as u32).to_be_bytes());
                    }
                }
                out.extend_from_slice(b);
            }
            Value::Array(a) => {
                match a.len() {
                    n if n < 16 => out.push(0x90 | n as u8),
                    n if n < 65536 => {
                        out.push(0xdc);
                        out.extend_from_slice(&(n as u16).to_be_bytes());
                    }
                    n => {
                        out.push(0xdd);
                        out.extend_from_slice(&(n as u32).to_be_bytes());
                    }
                }
                for x in a {
                    encode(x, out);
                }
            }
            Value::Object(m) => {
                match m.len() {
                    n if n < 16 => out.push(0x80 | n as u8),
                    n if n < 65536 => {
                        out.push(0xde);
                        out.extend_from_slice(&(n as u16).to_be_bytes());
                    }
                    n => {
                        out.push(0xdf);
                        out.extend_from_slice(&(n as u32).to_be_bytes());
                    }
                }
                for (k, x) in m {
                    encode(&Value::String(k.clone()), out);
                    encode(x, out);
                }
            }
        }
    }

    fn int(i: i64, out: &mut Vec<u8>) {
        if (0..128).contains(&i) {
            out.push(i as u8);
        } else if (-32..0).contains(&i) {
            out.push(i as i8 as u8);
        } else {
            out.push(0xd3);
            out.extend_from_slice(&i.to_be_bytes());
        }
    }

    /// Incremental decoder over a byte buffer: `Ok(None)` means "need more bytes".
    pub fn decode(buf: &[u8]) -> Result<Option<(Value, usize)>, String> {
        let mut pos = 0;
        Ok(value(buf, &mut pos)?.map(|v| (v, pos)))
    }

    fn take<'a>(buf: &'a [u8], pos: &mut usize, n: usize) -> Option<&'a [u8]> {
        let end = pos.checked_add(n)?;
        let s = buf.get(*pos..end)?;
        *pos = end;
        Some(s)
    }

    fn be(buf: &[u8], pos: &mut usize, n: usize) -> Option<u64> {
        take(buf, pos, n).map(|b| b.iter().fold(0u64, |a, x| (a << 8) | *x as u64))
    }

    fn value(buf: &[u8], pos: &mut usize) -> Result<Option<Value>, String> {
        let Some(&t) = buf.get(*pos) else { return Ok(None) };
        *pos += 1;
        macro_rules! need {
            ($e:expr) => {
                match $e {
                    Some(x) => x,
                    None => return Ok(None),
                }
            };
        }
        let v = match t {
            0x00..=0x7f => Value::from(t as i64),
            0xe0..=0xff => Value::from(t as i8 as i64),
            0xc0 => Value::Null,
            0xc2 => Value::Bool(false),
            0xc3 => Value::Bool(true),
            0xcc => Value::from(need!(be(buf, pos, 1))),
            0xcd => Value::from(need!(be(buf, pos, 2))),
            0xce => Value::from(need!(be(buf, pos, 4))),
            0xcf => Value::from(need!(be(buf, pos, 8))),
            0xd0 => Value::from(need!(be(buf, pos, 1)) as u8 as i8 as i64),
            0xd1 => Value::from(need!(be(buf, pos, 2)) as u16 as i16 as i64),
            0xd2 => Value::from(need!(be(buf, pos, 4)) as u32 as i32 as i64),
            0xd3 => Value::from(need!(be(buf, pos, 8)) as i64),
            0xca => Value::from(f32::from_bits(need!(be(buf, pos, 4)) as u32) as f64),
            0xcb => Value::from(f64::from_bits(need!(be(buf, pos, 8)))),
            0xa0..=0xbf => return str_(buf, pos, (t & 0x1f) as usize),
            0xd9 | 0xc4 => {
                let n = need!(be(buf, pos, 1)) as usize;
                return str_(buf, pos, n);
            }
            0xda | 0xc5 => {
                let n = need!(be(buf, pos, 2)) as usize;
                return str_(buf, pos, n);
            }
            0xdb | 0xc6 => {
                let n = need!(be(buf, pos, 4)) as usize;
                return str_(buf, pos, n);
            }
            0x90..=0x9f => return arr(buf, pos, (t & 0x0f) as usize),
            0xdc => {
                let n = need!(be(buf, pos, 2)) as usize;
                return arr(buf, pos, n);
            }
            0xdd => {
                let n = need!(be(buf, pos, 4)) as usize;
                return arr(buf, pos, n);
            }
            0x80..=0x8f => return map(buf, pos, (t & 0x0f) as usize),
            0xde => {
                let n = need!(be(buf, pos, 2)) as usize;
                return map(buf, pos, n);
            }
            0xdf => {
                let n = need!(be(buf, pos, 4)) as usize;
                return map(buf, pos, n);
            }
            // ext types (Neovim buffer/window handles): skip the payload, report null
            0xd4..=0xd8 => {
                let n = 1usize << (t - 0xd4);
                need!(take(buf, pos, n + 1));
                Value::Null
            }
            0xc7..=0xc9 => {
                let w = 1usize << (t - 0xc7);
                let n = need!(be(buf, pos, w)) as usize;
                need!(take(buf, pos, n + 1));
                Value::Null
            }
            other => return Err(format!("unsupported msgpack type 0x{other:02x}")),
        };
        Ok(Some(v))
    }

    fn str_(buf: &[u8], pos: &mut usize, n: usize) -> Result<Option<Value>, String> {
        Ok(take(buf, pos, n).map(|b| Value::String(String::from_utf8_lossy(b).into_owned())))
    }

    fn arr(buf: &[u8], pos: &mut usize, n: usize) -> Result<Option<Value>, String> {
        let mut v = Vec::new();
        for _ in 0..n {
            match value(buf, pos)? {
                Some(x) => v.push(x),
                None => return Ok(None),
            }
        }
        Ok(Some(Value::Array(v)))
    }

    fn map(buf: &[u8], pos: &mut usize, n: usize) -> Result<Option<Value>, String> {
        let mut m = serde_json::Map::new();
        for _ in 0..n {
            let (Some(k), Some(x)) = (value(buf, pos)?, value(buf, pos)?) else { return Ok(None) };
            let key = match k {
                Value::String(s) => s,
                other => other.to_string(),
            };
            m.insert(key, x);
        }
        Ok(Some(Value::Object(m)))
    }
}

// ---------------------------------------------------------------------------------------------
// msgpack-rpc client
// ---------------------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum NvimError {
    #[error("cannot reach Neovim at {path}: {source}")]
    Connect { path: PathBuf, source: std::io::Error },
    #[error("I/O error talking to Neovim: {0}")]
    Io(#[from] std::io::Error),
    #[error("timed out after {0:?} waiting for Neovim")]
    Timeout(Duration),
    #[error("Neovim RPC protocol error: {0}")]
    Protocol(String),
    #[error("Neovim returned an error: {0}")]
    Remote(String),
}

/// Default time to wait for one RPC answer.
pub const RPC_TIMEOUT: Duration = Duration::from_secs(5);

/// One synchronous request on a fresh connection: `[0, msgid, method, params]` ->
/// `[1, msgid, error, result]`. Notifications and other messages are skipped.
pub fn rpc_call(sock: &Path, method: &str, params: serde_json::Value, timeout: Duration) -> Result<serde_json::Value, NvimError> {
    use serde_json::{json, Value};
    let mut stream = UnixStream::connect(sock).map_err(|source| NvimError::Connect { path: sock.to_path_buf(), source })?;
    stream.set_write_timeout(Some(timeout))?;
    let msgid = 1;
    let mut out = Vec::new();
    mp::encode(&json!([0, msgid, method, params]), &mut out);
    stream.write_all(&out)?;
    let deadline = Instant::now() + timeout;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        while let Some((msg, used)) = mp::decode(&buf).map_err(NvimError::Protocol)? {
            buf.drain(..used);
            if let Value::Array(a) = &msg {
                if a.len() == 4 && a[0] == json!(1) && a[1] == json!(msgid) {
                    return match &a[2] {
                        Value::Null => Ok(a[3].clone()),
                        e => Err(NvimError::Remote(match e {
                            Value::Array(p) if p.len() == 2 => p[1].as_str().unwrap_or("error").to_string(),
                            other => other.to_string(),
                        })),
                    };
                }
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(NvimError::Timeout(timeout));
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut chunk) {
            Ok(0) => return Err(NvimError::Protocol("connection closed before a response".into())),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                return Err(NvimError::Timeout(timeout))
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// `nvim_exec_lua(code, args)`; the Lua chunk receives `args` as `...` and may `return` a value.
pub fn exec_lua(sock: &Path, code: &str, args: serde_json::Value, timeout: Duration) -> Result<serde_json::Value, NvimError> {
    rpc_call(sock, "nvim_exec_lua", serde_json::json!([code, args]), timeout)
}

/// Lua for the jump (recipe from `spike/nvim-jump.sh`, plus an optional line-range highlight and an
/// optional diff against a git revision).
/// Args: `[abs_file, line, end_line_or_null, diff_or_null]` with `diff` = [`DiffBase`].
/// Returns `"<file as nvim knows it>:<line>"`.
///
/// The diff is scoped to this one jump: every jump first closes the diff view an earlier jump made
/// (only windows tether marked, in the current tab page) and resets the gitsigns base it set on a
/// buffer, so a jump without a diff always leaves the editor in its default state.
pub const JUMP_LUA: &str = r#"
local a = ...
local d = a[4]
if d == vim.NIL then d = nil end
local function gitsigns() return package.loaded.gitsigns end
-- undo the previous tether diff (ours only: marked windows and buffers)
for _, w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
  if vim.w[w].tether_diff_base and #vim.api.nvim_tabpage_list_wins(0) > 1 then pcall(vim.api.nvim_win_close, w, true) end
end
for _, w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
  if vim.w[w].tether_diff_work then
    pcall(vim.api.nvim_win_call, w, function() vim.cmd("diffoff") end)
    vim.w[w].tether_diff_work = nil
  end
end
for _, nr in ipairs(vim.api.nvim_list_bufs()) do
  if vim.api.nvim_buf_is_loaded(nr) and vim.b[nr].tether_diff_rev then
    vim.b[nr].tether_diff_rev = nil
    local gs = gitsigns()
    if gs then pcall(vim.api.nvim_buf_call, nr, function() gs.change_base(nil, false) end) end
  end
end
local f = vim.fn.resolve(vim.fn.fnamemodify(a[1], ":p"))
-- compare normalized names; vim.fn.bufnr(f) treats `[slug]` etc. as a glob pattern
local b = -1
for _, nr in ipairs(vim.api.nvim_list_bufs()) do
  local nm = vim.api.nvim_buf_get_name(nr)
  if nm ~= "" and vim.fn.resolve(vim.fn.fnamemodify(nm, ":p")) == f then b = nr break end
end
local w = b > 0 and vim.fn.bufwinid(b) or -1
if w > 0 then vim.api.nvim_set_current_win(w)
else vim.cmd("hide edit " .. vim.fn.fnameescape(f)) end
local buf = vim.api.nvim_get_current_buf()
local last = vim.api.nvim_buf_line_count(buf)
local n = math.max(1, math.min(a[2], last))
vim.api.nvim_win_set_cursor(0, { n, 0 })
vim.cmd("normal! zz")
local ns = vim.api.nvim_create_namespace("tether_range")
vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)
local e = a[3]
if type(e) == "number" and e > n then
  e = math.min(e, last)
  for l = n, e do
    vim.api.nvim_buf_set_extmark(buf, ns, l - 1, 0, { line_hl_group = "Visual", priority = 10 })
  end
  vim.defer_fn(function()
    if vim.api.nvim_buf_is_valid(buf) then vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1) end
  end, 4000)
end
if d then
  local work = vim.api.nvim_get_current_win()
  local ft = vim.bo[buf].filetype
  vim.cmd("leftabove vertical new")
  local base = vim.api.nvim_get_current_buf()
  vim.bo[base].buftype = "nofile"
  vim.bo[base].bufhidden = "wipe"
  vim.bo[base].swapfile = false
  local text = d.text
  if text == vim.NIL then text = nil end
  local lines = text and vim.split(text, "\n", { plain = true }) or {}
  if text and lines[#lines] == "" then table.remove(lines) end
  vim.api.nvim_buf_set_lines(base, 0, -1, false, lines)
  vim.bo[base].modifiable = false
  vim.bo[base].filetype = ft
  pcall(vim.api.nvim_buf_set_name, base, ("tether://%s/%s"):format(d.rev, d.rel))
  vim.w.tether_diff_base = true
  vim.wo.winbar = text and ("%s (%s)"):format(d.rev, d.short) or ("%s (%s): file not in this revision"):format(d.rev, d.short)
  vim.cmd("diffthis")
  vim.api.nvim_set_current_win(work)
  vim.w.tether_diff_work = true
  vim.cmd("diffthis")
  vim.b[buf].tether_diff_rev = d.rev
  -- gitsigns (when installed) attaches to a fresh buffer asynchronously: retry until it has, and
  -- stop if a newer jump replaced or cleared this diff meanwhile
  local tries = 0
  local function set_gs_base()
    local gs = gitsigns()
    if not gs or not vim.api.nvim_buf_is_valid(buf) or vim.b[buf].tether_diff_rev ~= d.rev then return end
    if vim.b[buf].gitsigns_status_dict then
      pcall(vim.api.nvim_buf_call, buf, function() gs.change_base(d.sha, false) end)
    elseif tries < 20 then
      tries = tries + 1
      vim.defer_fn(set_gs_base, 150)
    end
  end
  set_gs_base()
  vim.api.nvim_win_set_cursor(work, { n, 0 })
  vim.cmd("normal! zvzz")
end
return vim.fn.expand("%:p") .. ":" .. n
"#;

/// What to diff the opened file against (see [`JUMP_LUA`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiffBase {
    /// The revision as given (`main`, `HEAD~3`, a sha); shown in the diff window.
    pub rev: String,
    /// The full commit it resolved to.
    pub sha: String,
    /// Abbreviated commit, for display.
    pub short: String,
    /// The file's path relative to the worktree root.
    pub rel: String,
    /// The file's content at `sha`; `None` when the file does not exist in that revision.
    pub text: Option<String>,
}

/// Jump the Neovim listening on `sock` to `file:line` (file must be absolute). `end_line` highlights
/// the range for a few seconds. Returns `"<path>:<line>"` as Neovim reports it.
/// `diff` opens a side-by-side diff against that revision (and closes any earlier tether diff).
pub fn jump(sock: &Path, file: &Path, line: u32, end_line: Option<u32>, diff: Option<&DiffBase>) -> Result<String, NvimError> {
    let args = serde_json::json!([[file.to_string_lossy(), line, end_line, diff]]);
    let v = exec_lua(sock, JUMP_LUA, args, RPC_TIMEOUT)?;
    v.as_str().map(str::to_string).ok_or_else(|| NvimError::Protocol(format!("unexpected jump result: {v}")))
}

/// Where Neovim's cursor is now: `"<path>:<line>"`. Doubles as the liveness probe.
pub fn position(sock: &Path) -> Result<String, NvimError> {
    let v = exec_lua(
        sock,
        r#"return vim.fn.expand("%:p") .. ":" .. vim.fn.line(".")"#,
        serde_json::json!([]),
        Duration::from_millis(1500),
    )?;
    v.as_str().map(str::to_string).ok_or_else(|| NvimError::Protocol(format!("unexpected result: {v}")))
}

/// If a socket file exists and the Neovim behind it answers: its cursor position.
pub fn probe(sock: &Path) -> Option<String> {
    if !sock.exists() {
        return None;
    }
    position(sock).ok()
}

/// Poll until the Neovim on `sock` answers, or `timeout` passes.
pub fn wait_ready(sock: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if probe(sock).is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_replaced_and_dir_layout() {
        let p = socket_path_in(Path::new("/run/user/1"), "w8:p2");
        assert_eq!(p, PathBuf::from("/run/user/1/tether/w8-p2.sock"));
    }

    #[test]
    fn trailing_slash_stripped() {
        assert_eq!(strip_trailing_slashes(Path::new("/var/folders/x/T/")), PathBuf::from("/var/folders/x/T"));
        assert_eq!(strip_trailing_slashes(Path::new("/")), PathBuf::from("/"));
        let p = socket_path_in(&strip_trailing_slashes(Path::new("/var/T/")), "wC:p10");
        assert_eq!(p.to_string_lossy(), "/var/T/tether/wC-p10.sock");
    }

    #[test]
    fn env_precedence() {
        use std::ffi::OsString;
        let env = |xdg: Option<&'static str>, tmp: Option<&'static str>| {
            move |k: &str| -> Option<OsString> {
                match k {
                    "XDG_RUNTIME_DIR" => xdg.map(OsString::from),
                    "TMPDIR" => tmp.map(OsString::from),
                    _ => None,
                }
            }
        };
        assert_eq!(runtime_dir_from(env(Some("/xdg/"), Some("/tmpdir/"))), PathBuf::from("/xdg"));
        assert_eq!(runtime_dir_from(env(None, Some("/tmpdir/"))), PathBuf::from("/tmpdir"));
        assert_eq!(runtime_dir_from(env(Some(""), Some("/tmpdir/"))), PathBuf::from("/tmpdir"));
        assert_eq!(runtime_dir_from(env(None, None)), PathBuf::from("/tmp"));
    }

    // ---- msgpack / rpc ----

    fn roundtrip(v: serde_json::Value) {
        let mut b = Vec::new();
        mp::encode(&v, &mut b);
        let (d, used) = mp::decode(&b).unwrap().unwrap();
        assert_eq!(d, v);
        assert_eq!(used, b.len());
        // every strict prefix is "incomplete", never an error or a wrong value
        for n in 0..b.len() {
            assert!(mp::decode(&b[..n]).unwrap().is_none(), "prefix {n} of {v}");
        }
    }

    #[test]
    fn msgpack_roundtrips() {
        use serde_json::json;
        roundtrip(json!(null));
        roundtrip(json!([true, false, 0, 1, 127, 128, -1, -32, -33, 70000, -70000, i64::MAX, i64::MIN]));
        roundtrip(json!("héllo → 世界"));
        roundtrip(json!("x".repeat(40)));
        roundtrip(json!("y".repeat(300)));
        roundtrip(json!("z".repeat(70000)));
        roundtrip(json!([0, 1, "nvim_exec_lua", ["return 1", [["/a/b.rs", 42, null]]]]));
        roundtrip(json!({"a": [1, 2, {"b": "c"}], "d": null}));
        roundtrip(json!((0..40).collect::<Vec<_>>()));
    }

    #[test]
    fn msgpack_decodes_neovim_style_values() {
        // uint8/16/32/64, int8/16/32, float64, bin8, ext fixed (handle) -> null
        let (v, _) = mp::decode(&[0xcc, 200]).unwrap().unwrap();
        assert_eq!(v, 200);
        let (v, _) = mp::decode(&[0xcd, 0x01, 0x00]).unwrap().unwrap();
        assert_eq!(v, 256);
        let (v, _) = mp::decode(&[0xd0, 0xff]).unwrap().unwrap();
        assert_eq!(v, -1);
        let (v, _) = mp::decode(&[0xd1, 0xff, 0xfe]).unwrap().unwrap();
        assert_eq!(v, -2);
        let (v, _) = mp::decode(&[0xcb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0]).unwrap().unwrap();
        assert_eq!(v, 1.5);
        let (v, _) = mp::decode(&[0xc4, 2, b'h', b'i']).unwrap().unwrap();
        assert_eq!(v, "hi");
        let (v, used) = mp::decode(&[0xd4, 0, 7, 0xc3]).unwrap().unwrap();
        assert_eq!((v, used), (serde_json::Value::Null, 3));
        assert!(mp::decode(&[0xc1]).is_err());
    }

    /// A one-shot fake Neovim: reads a request, answers with `reply` (as [1,id,err,result]).
    fn fake_nvim(sock: &Path, err: serde_json::Value, result: serde_json::Value) -> std::thread::JoinHandle<serde_json::Value> {
        let l = std::os::unix::net::UnixListener::bind(sock).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let req = loop {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some((v, _)) = mp::decode(&buf).unwrap() {
                    break v;
                }
            };
            let mut out = Vec::new();
            // a notification first, which the client must skip
            mp::encode(&serde_json::json!([2, "nvim_buf_lines_event", []]), &mut out);
            mp::encode(&serde_json::json!([1, req[1], err, result]), &mut out);
            s.write_all(&out).unwrap();
            req
        })
    }

    #[test]
    fn rpc_call_roundtrip_error_and_timeout() {
        let d = tempfile::tempdir().unwrap();
        let sock = d.path().join("n.sock");
        let h = fake_nvim(&sock, serde_json::Value::Null, serde_json::json!("/x/a.ts:42"));
        let r = jump(&sock, Path::new("/x/a.ts"), 42, Some(50), None).unwrap();
        assert_eq!(r, "/x/a.ts:42");
        let req = h.join().unwrap();
        assert_eq!(req[0], 0);
        assert_eq!(req[2], "nvim_exec_lua");
        assert_eq!(req[3][0].as_str().unwrap(), JUMP_LUA);
        assert_eq!(req[3][1], serde_json::json!([["/x/a.ts", 42, 50, null]]));

        let sock2 = d.path().join("n2.sock");
        let h = fake_nvim(&sock2, serde_json::json!([0, "E5108: boom"]), serde_json::Value::Null);
        let e = exec_lua(&sock2, "error('x')", serde_json::json!([]), RPC_TIMEOUT).unwrap_err();
        assert!(matches!(e, NvimError::Remote(ref m) if m.contains("boom")), "{e}");
        h.join().unwrap();

        // a listener that never answers -> timeout; a missing socket -> connect error, probe None
        let sock3 = d.path().join("n3.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock3).unwrap();
        let e = rpc_call(&sock3, "nvim_eval", serde_json::json!(["1"]), Duration::from_millis(100)).unwrap_err();
        assert!(matches!(e, NvimError::Timeout(_)), "{e}");
        assert!(probe(&d.path().join("nope.sock")).is_none());
        assert!(!wait_ready(&d.path().join("nope.sock"), Duration::from_millis(150)));
    }

    /// Against a real headless Neovim, when one is installed.
    #[test]
    fn real_neovim_jump_and_probe() {
        let Ok(out) = std::process::Command::new("nvim").arg("--version").output() else { return };
        if !out.status.success() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().canonicalize().unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, (1..=100).map(|n| format!("line {n}\n")).collect::<String>()).unwrap();
        let sock = dir.join("n.sock");
        let mut child = std::process::Command::new("nvim")
            .args(["--headless", "--clean", "-n", "--listen"])
            .arg(&sock)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait_ready(&sock, Duration::from_secs(10)));
        let at = jump(&sock, &file, 42, Some(44), None).unwrap();
        assert_eq!(at, format!("{}:42", file.display()));
        assert_eq!(position(&sock).unwrap(), format!("{}:42", file.display()));
        // second jump: same buffer, cursor moves; past EOF clamps
        assert_eq!(jump(&sock, &file, 7, None, None).unwrap(), format!("{}:7", file.display()));
        assert_eq!(jump(&sock, &file, 9999, None, None).unwrap(), format!("{}:100", file.display()));
        // finding 11: `[slug]` is a glob to vim.fn.bufnr(); the buffer for `s/page.tsx` must not match it
        std::fs::create_dir_all(dir.join("s")).unwrap();
        std::fs::create_dir_all(dir.join("[slug]")).unwrap();
        let plain = dir.join("s/page.tsx");
        let slug = dir.join("[slug]/page.tsx");
        std::fs::write(&plain, "plain\n".repeat(5)).unwrap();
        std::fs::write(&slug, "slug\n".repeat(5)).unwrap();
        assert_eq!(jump(&sock, &plain, 2, None, None).unwrap(), format!("{}:2", plain.display()));
        assert_eq!(jump(&sock, &slug, 3, None, None).unwrap(), format!("{}:3", slug.display()));
        assert_eq!(jump(&sock, &plain, 4, None, None).unwrap(), format!("{}:4", plain.display()));

        // --diff: a read-only base window on the left, both in diff mode, cursor in the working file
        let eval = |code: &str| exec_lua(&sock, code, serde_json::json!([]), RPC_TIMEOUT).unwrap();
        let base = DiffBase {
            rev: "main".into(),
            sha: "0".repeat(40),
            short: "0000000".into(),
            rel: "a.txt".into(),
            text: Some((1..=100).map(|n| if n == 50 { "old 50\n".into() } else { format!("line {n}\n") }).collect()),
        };
        assert_eq!(jump(&sock, &file, 50, None, Some(&base)).unwrap(), format!("{}:50", file.display()));
        let state = r#"
            local wins = vim.api.nvim_tabpage_list_wins(0)
            local out = { n = #wins, cur_diff = vim.wo.diff, base = nil }
            for _, w in ipairs(wins) do
              if vim.w[w].tether_diff_base then
                local b = vim.api.nvim_win_get_buf(w)
                out.base = { diff = vim.wo[w].diff, name = vim.api.nvim_buf_get_name(b), modifiable = vim.bo[b].modifiable,
                             line50 = vim.api.nvim_buf_get_lines(b, 49, 50, false)[1],
                             left = vim.fn.win_screenpos(w)[2] < vim.fn.win_screenpos(0)[2] }
              end
            end
            return out"#;
        let v = eval(state);
        assert_eq!(v["n"], 2, "{v}");
        assert_eq!(v["cur_diff"], true, "{v}");
        assert_eq!(v["base"]["diff"], true, "{v}");
        assert_eq!(v["base"]["modifiable"], false, "{v}");
        assert_eq!(v["base"]["line50"], "old 50", "{v}");
        assert_eq!(v["base"]["left"], true, "{v}");
        assert_eq!(v["base"]["name"], "tether://main/a.txt", "{v}");
        // again with another revision: still exactly one diff (the old one is replaced)
        let other = DiffBase { rev: "v1".into(), text: None, ..base.clone() };
        jump(&sock, &file, 10, None, Some(&other)).unwrap();
        let v = eval(state);
        assert_eq!(v["n"], 2, "{v}");
        assert_eq!(v["base"]["name"], "tether://v1/a.txt", "{v}");
        // a plain jump afterwards: the diff is gone and diff mode is off (back to the default)
        assert_eq!(jump(&sock, &file, 5, None, None).unwrap(), format!("{}:5", file.display()));
        let v = eval(state);
        assert_eq!(v["n"], 1, "{v}");
        assert_eq!(v["cur_diff"], false, "{v}");
        assert!(v["base"].is_null(), "{v}");
        let _ = child.kill();
        let _ = child.wait();
    }
}
