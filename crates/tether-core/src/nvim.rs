//! Neovim side conventions. Phase 1 only needs the socket-path convention; the RPC jump
//! (msgpack-rpc `nvim_exec_lua`, see `spike/FINDINGS.md`) is added in Phase 2.

use std::path::{Path, PathBuf};

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
}
