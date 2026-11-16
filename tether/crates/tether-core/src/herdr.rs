//! Synchronous NDJSON client for the Herdr socket API.
//!
//! One request line, one response line, on a fresh connection per call (see `spike/FINDINGS.md`).
//! `Client::call` is the raw escape hatch; the typed wrappers below cover the methods tether
//! needs in Phases 1-2. Field names follow `docs/herdr/api-schema.json` and real responses.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Default per-request timeout (connect is instant on a unix socket; this bounds the read).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum HerdrError {
    #[error("cannot connect to Herdr socket {path}: {source} (is Herdr running? set HERDR_SOCKET_PATH to override)")]
    Connect { path: PathBuf, source: io::Error },
    #[error("I/O error talking to Herdr: {0}")]
    Io(#[from] io::Error),
    #[error("timed out after {0:?} waiting for Herdr")]
    Timeout(Duration),
    #[error("unexpected response from Herdr: {0}")]
    Protocol(String),
    /// Herdr answered with `{"error":{"code":..,"message":..}}`.
    #[error("{message} (herdr error: {code})")]
    Api { code: String, message: String },
}

impl HerdrError {
    /// Herdr's error code (`agent_not_ready`, `pane_not_found`, ...), if this is an API error.
    pub fn code(&self) -> Option<&str> {
        match self {
            HerdrError::Api { code, .. } => Some(code),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, HerdrError>;

/// Socket path: `HERDR_SOCKET_PATH`, else `~/.config/herdr/herdr.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HERDR_SOCKET_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    home.join(".config/herdr/herdr.sock")
}

#[derive(Debug)]
pub struct Client {
    socket: PathBuf,
    timeout: Duration,
    next_id: AtomicU64,
}

// ---------------------------------------------------------------------------------------------
// Response types (lenient: unknown fields ignored, optional fields default)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: u32,
    #[serde(default)]
    pub tab_count: u32,
    #[serde(default)]
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub agent_status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: String,
    /// 1-based display index.
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: u32,
    #[serde(default)]
    pub agent_status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    #[serde(default)]
    pub terminal_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    /// Detected/reported agent kind, e.g. `claude`; absent for plain panes.
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: String,
    #[serde(default)]
    pub terminal_title: Option<String>,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub pane_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub terminal_id: String,
    /// Agent kind (`claude`, `codex`, ...).
    #[serde(default)]
    pub agent: String,
    /// `idle | working | blocked | done | unknown`.
    #[serde(default)]
    pub agent_status: String,
    /// Only for agents started/renamed with a name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub terminal_title: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub state_change_seq: u64,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ForegroundProcess {
    #[serde(default)]
    pub pid: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub argv0: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub cmdline: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub shell_pid: Option<u64>,
    #[serde(default)]
    pub foreground_process_group_id: Option<u64>,
    #[serde(default)]
    pub foreground_processes: Vec<ForegroundProcess>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaneRead {
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub protocol: u32,
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub agents: Vec<Agent>,
    /// Per-tab layout objects (area, panes[].rect, splits[]); kept raw for the layout code.
    #[serde(default)]
    pub layouts: Vec<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pong {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub protocol: u32,
}

/// Parameters for `tab.create`. All optional.
#[derive(Debug, Clone, Default)]
pub struct TabCreate<'a> {
    pub workspace_id: Option<&'a str>,
    pub label: Option<&'a str>,
    pub cwd: Option<&'a str>,
    /// Defaults to false (never steal focus).
    pub focus: bool,
}

/// Parameters for `pane.split`. Always pass `target_pane_id`; omitting it splits the focused pane.
#[derive(Debug, Clone)]
pub struct PaneSplit<'a> {
    pub target_pane_id: &'a str,
    /// `"right"` or `"down"`.
    pub direction: &'a str,
    /// Fraction given to the EXISTING pane.
    pub ratio: Option<f32>,
    pub cwd: Option<&'a str>,
    pub focus: bool,
}

/// `wait` options for `agent.prompt`. tether never uses this for comments.
#[derive(Debug, Clone, Default)]
pub struct PromptWait {
    pub timeout_ms: Option<u64>,
    pub until: Vec<String>,
}

fn take<T: DeserializeOwned>(result: &Value, key: &str) -> Result<T> {
    let v = result
        .get(key)
        .ok_or_else(|| HerdrError::Protocol(format!("response has no `{key}` field: {result}")))?;
    serde_json::from_value(v.clone())
        .map_err(|e| HerdrError::Protocol(format!("cannot parse `{key}`: {e}: {v}")))
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client { socket: socket.into(), timeout: DEFAULT_TIMEOUT, next_id: AtomicU64::new(1) }
    }

    /// Client for `HERDR_SOCKET_PATH` (or the default socket).
    pub fn from_env() -> Self {
        Self::new(default_socket_path())
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Raw call. Returns the `result` object on success; Herdr errors become `HerdrError::Api`.
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let params = if params.is_null() { json!({}) } else { params };
        let id = format!("tether_{}_{}", std::process::id(), self.next_id.fetch_add(1, Ordering::Relaxed));
        let req = json!({"id": id, "method": method, "params": params});

        let mut stream = UnixStream::connect(&self.socket)
            .map_err(|source| HerdrError::Connect { path: self.socket.clone(), source })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let mut line = serde_json::to_vec(&req).map_err(|e| HerdrError::Protocol(e.to_string()))?;
        line.push(b'\n');
        stream.write_all(&line)?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut resp = String::new();
        match reader.read_line(&mut resp) {
            Ok(0) => return Err(HerdrError::Protocol("connection closed without a response".into())),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Err(HerdrError::Timeout(self.timeout))
            }
            Err(e) => return Err(e.into()),
        }
        let v: Value = serde_json::from_str(resp.trim_end())
            .map_err(|e| HerdrError::Protocol(format!("invalid JSON: {e}: {resp}")))?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(Value::as_str).unwrap_or("unknown").to_string();
            let message = err.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            return Err(HerdrError::Api { code, message });
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| HerdrError::Protocol(format!("response has neither result nor error: {v}")))
    }

    // ---- ping / session -------------------------------------------------------------------

    pub fn ping(&self) -> Result<Pong> {
        let r = self.call("ping", json!({}))?;
        serde_json::from_value(r).map_err(|e| HerdrError::Protocol(e.to_string()))
    }

    pub fn session_snapshot(&self) -> Result<Snapshot> {
        take(&self.call("session.snapshot", json!({}))?, "snapshot")
    }

    // ---- workspaces -----------------------------------------------------------------------

    pub fn workspace_list(&self) -> Result<Vec<Workspace>> {
        take(&self.call("workspace.list", json!({}))?, "workspaces")
    }

    pub fn workspace_get(&self, workspace_id: &str) -> Result<Workspace> {
        take(&self.call("workspace.get", json!({"workspace_id": workspace_id}))?, "workspace")
    }

    // ---- tabs -----------------------------------------------------------------------------

    pub fn tab_list(&self, workspace_id: Option<&str>) -> Result<Vec<Tab>> {
        let p = match workspace_id {
            Some(w) => json!({"workspace_id": w}),
            None => json!({}),
        };
        take(&self.call("tab.list", p)?, "tabs")
    }

    pub fn tab_get(&self, tab_id: &str) -> Result<Tab> {
        take(&self.call("tab.get", json!({"tab_id": tab_id}))?, "tab")
    }

    /// Returns the new tab and its root pane.
    pub fn tab_create(&self, p: &TabCreate) -> Result<(Tab, Pane)> {
        let mut m = vec![("focus", json!(p.focus))];
        if let Some(v) = p.workspace_id {
            m.push(("workspace_id", json!(v)));
        }
        if let Some(v) = p.label {
            m.push(("label", json!(v)));
        }
        if let Some(v) = p.cwd {
            m.push(("cwd", json!(v)));
        }
        let r = self.call("tab.create", obj(&m))?;
        Ok((take(&r, "tab")?, take(&r, "root_pane")?))
    }

    pub fn tab_rename(&self, tab_id: &str, label: &str) -> Result<()> {
        self.call("tab.rename", json!({"tab_id": tab_id, "label": label})).map(|_| ())
    }

    pub fn tab_focus(&self, tab_id: &str) -> Result<()> {
        self.call("tab.focus", json!({"tab_id": tab_id})).map(|_| ())
    }

    pub fn tab_close(&self, tab_id: &str) -> Result<()> {
        self.call("tab.close", json!({"tab_id": tab_id})).map(|_| ())
    }

    // ---- panes ----------------------------------------------------------------------------

    pub fn pane_list(&self, workspace_id: Option<&str>) -> Result<Vec<Pane>> {
        let p = match workspace_id {
            Some(w) => json!({"workspace_id": w}),
            None => json!({}),
        };
        take(&self.call("pane.list", p)?, "panes")
    }

    pub fn pane_get(&self, pane_id: &str) -> Result<Pane> {
        take(&self.call("pane.get", json!({"pane_id": pane_id}))?, "pane")
    }

    pub fn pane_split(&self, p: &PaneSplit) -> Result<Pane> {
        let mut m = vec![
            ("target_pane_id", json!(p.target_pane_id)),
            ("direction", json!(p.direction)),
            ("focus", json!(p.focus)),
        ];
        if let Some(r) = p.ratio {
            m.push(("ratio", json!(r)));
        }
        if let Some(c) = p.cwd {
            m.push(("cwd", json!(c)));
        }
        take(&self.call("pane.split", obj(&m))?, "pane")
    }

    pub fn pane_process_info(&self, pane_id: &str) -> Result<ProcessInfo> {
        take(&self.call("pane.process_info", json!({"pane_id": pane_id}))?, "process_info")
    }

    /// `pane.send_input`: optional literal text (bracketed if the pane enabled paste mode) and
    /// named keys (`"enter"`, `"esc"`, `"ctrl+c"`). Not an agent-safe path: use `agent_prompt`.
    pub fn pane_send_input(&self, pane_id: &str, text: Option<&str>, keys: &[&str]) -> Result<()> {
        let mut m = vec![("pane_id", json!(pane_id))];
        if let Some(t) = text {
            m.push(("text", json!(t)));
        }
        if !keys.is_empty() {
            m.push(("keys", json!(keys)));
        }
        self.call("pane.send_input", obj(&m)).map(|_| ())
    }

    /// `source`: `visible | recent | recent_unwrapped | detection`. Omit `lines` (see FINDINGS quirk 1).
    pub fn pane_read(&self, pane_id: &str, source: &str, lines: Option<u32>) -> Result<PaneRead> {
        let mut m = vec![("pane_id", json!(pane_id)), ("source", json!(source))];
        if let Some(l) = lines {
            m.push(("lines", json!(l)));
        }
        take(&self.call("pane.read", obj(&m))?, "read")
    }

    pub fn pane_focus(&self, pane_id: &str) -> Result<()> {
        self.call("pane.focus", json!({"pane_id": pane_id})).map(|_| ())
    }

    pub fn pane_close(&self, pane_id: &str) -> Result<()> {
        self.call("pane.close", json!({"pane_id": pane_id})).map(|_| ())
    }

    // ---- agents ---------------------------------------------------------------------------

    /// Global: every workspace. Filter by `workspace_id` yourself.
    pub fn agent_list(&self) -> Result<Vec<Agent>> {
        take(&self.call("agent.list", json!({}))?, "agents")
    }

    /// `target` is a pane id or an agent name.
    pub fn agent_get(&self, target: &str) -> Result<Agent> {
        take(&self.call("agent.get", json!({"target": target}))?, "agent")
    }

    /// Submit `text` as a prompt (bracketed paste + Enter 300 ms later, done by Herdr).
    /// Without `wait` returns as soon as the text is written; the returned `agent_status` is the
    /// state BEFORE submission.
    pub fn agent_prompt(&self, target: &str, text: &str, wait: Option<&PromptWait>) -> Result<Agent> {
        let mut m = vec![("target", json!(target)), ("text", json!(text))];
        if let Some(w) = wait {
            let mut wm = Map::new();
            if let Some(t) = w.timeout_ms {
                wm.insert("timeout_ms".into(), json!(t));
            }
            if !w.until.is_empty() {
                wm.insert("until".into(), json!(w.until));
            }
            m.push(("wait", Value::Object(wm)));
        }
        take(&self.call("agent.prompt", obj(&m))?, "agent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHerdr;

    #[test]
    fn ping_and_raw_call() {
        let fake = FakeHerdr::start(|method, _| match method {
            "ping" => Ok(json!({"type":"pong","version":"0.9.5","protocol":22})),
            _ => Err(("invalid_request".into(), "nope".into())),
        });
        let c = Client::new(fake.path());
        let p = c.ping().unwrap();
        assert_eq!(p.protocol, 22);
        assert_eq!(p.version, "0.9.5");
        let raw = c.call("ping", json!({})).unwrap();
        assert_eq!(raw["type"], "pong");
    }

    #[test]
    fn api_error_keeps_code() {
        let fake = FakeHerdr::start(|_, _| {
            Err(("agent_not_ready".into(), "agent w8:p1 is no longer the pane foreground process".into()))
        });
        let c = Client::new(fake.path());
        let e = c.agent_prompt("w8:p1", "hi", None).unwrap_err();
        assert_eq!(e.code(), Some("agent_not_ready"));
        assert!(e.to_string().contains("foreground process"));
    }

    #[test]
    fn connect_error_is_typed() {
        let c = Client::new("/nonexistent/herdr.sock");
        let e = c.ping().unwrap_err();
        assert!(matches!(e, HerdrError::Connect { .. }), "{e:?}");
        assert!(e.code().is_none());
    }

    #[test]
    fn timeout_is_typed() {
        let fake = FakeHerdr::start_with_delay(std::time::Duration::from_millis(600), |_, _| Ok(json!({"type":"pong"})));
        let c = Client::new(fake.path()).with_timeout(Duration::from_millis(100));
        assert!(matches!(c.ping().unwrap_err(), HerdrError::Timeout(_)));
    }

    #[test]
    fn request_ids_increment_and_params_never_null() {
        let fake = FakeHerdr::start(|_, _| Ok(json!({"type":"pong"})));
        let c = Client::new(fake.path());
        c.call("ping", Value::Null).unwrap();
        c.call("ping", json!({})).unwrap();
        let reqs = fake.requests();
        assert_eq!(reqs.len(), 2);
        assert!(reqs[0].params.is_object());
        assert_ne!(reqs[0].id, reqs[1].id);
    }

    #[test]
    fn typed_wrappers_send_exact_params() {
        let fake = FakeHerdr::start(|method, _| match method {
            "agent.prompt" => Ok(json!({"type":"agent_prompted","agent":{"pane_id":"w8:p2","agent":"claude","agent_status":"idle","workspace_id":"w8","tab_id":"w8:t1"}})),
            "tab.create" => Ok(json!({"type":"tab_created",
                "tab":{"tab_id":"w8:t2","workspace_id":"w8","number":2,"label":"second","focused":false,"pane_count":1,"agent_status":"unknown"},
                "root_pane":{"pane_id":"w8:p3","workspace_id":"w8","tab_id":"w8:t2"}})),
            "pane.split" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w8:p4","workspace_id":"w8","tab_id":"w8:t1"}})),
            "pane.read" => Ok(json!({"type":"pane_read","read":{"pane_id":"w8:p1","source":"recent","format":"text","text":"hi","revision":1,"truncated":false}})),
            "pane.send_input" | "tab.rename" | "tab.focus" | "tab.close" | "pane.focus" | "pane.close" => Ok(json!({"type":"ok"})),
            _ => Err(("invalid_request".into(), "?".into())),
        });
        let c = Client::new(fake.path());
        let a = c.agent_prompt("w8:p2", "x\ny", None).unwrap();
        assert_eq!(a.agent, "claude");
        let (t, p) = c.tab_create(&TabCreate { workspace_id: Some("w8"), label: Some("second"), ..Default::default() }).unwrap();
        assert_eq!((t.number, p.pane_id.as_str()), (2, "w8:p3"));
        let np = c.pane_split(&PaneSplit { target_pane_id: "w8:p1", direction: "right", ratio: Some(0.3), cwd: None, focus: false }).unwrap();
        assert_eq!(np.pane_id, "w8:p4");
        assert_eq!(c.pane_read("w8:p1", "recent", None).unwrap().text, "hi");
        c.pane_send_input("w8:p1", Some("ls"), &["enter"]).unwrap();
        c.tab_rename("w8:t2", "x").unwrap();
        c.tab_focus("w8:t2").unwrap();
        c.tab_close("w8:t2").unwrap();
        c.pane_focus("w8:p1").unwrap();
        c.pane_close("w8:p4").unwrap();

        let reqs = fake.requests();
        assert_eq!(reqs[0].method, "agent.prompt");
        assert_eq!(reqs[0].params, json!({"target":"w8:p2","text":"x\ny"}));
        assert_eq!(reqs[1].params, json!({"workspace_id":"w8","label":"second","focus":false}));
        assert_eq!(reqs[2].params["direction"], "right");
        assert_eq!(reqs[2].params["target_pane_id"], "w8:p1");
        assert_eq!(reqs[2].params["focus"], false);
        assert_eq!(reqs[4].params, json!({"pane_id":"w8:p1","text":"ls","keys":["enter"]}));
    }

    #[test]
    fn snapshot_and_lists_parse() {
        let fake = FakeHerdr::start(|method, _| match method {
            "session.snapshot" => Ok(json!({"type":"session_snapshot","snapshot":{
                "version":"0.9.5","protocol":22,"focused_workspace_id":"w7",
                "workspaces":[{"workspace_id":"w7","number":1,"label":"x","focused":true,"pane_count":1,"tab_count":1,"active_tab_id":"w7:t1","agent_status":"working"}],
                "tabs":[{"tab_id":"w7:t1","workspace_id":"w7","number":1,"label":"1","focused":true,"pane_count":1,"agent_status":"working"}],
                "panes":[{"pane_id":"w7:p1","workspace_id":"w7","tab_id":"w7:t1","agent":"claude","agent_status":"working"}],
                "agents":[{"pane_id":"w7:p1","workspace_id":"w7","tab_id":"w7:t1","agent":"claude","agent_status":"working","state_change_seq":153,"revision":2}],
                "layouts":[{"tab_id":"w7:t1"}]}})),
            "pane.process_info" => Ok(json!({"type":"pane_process_info","process_info":{"pane_id":"w7:p1","shell_pid":1,"foreground_processes":[{"pid":2,"name":"nvim","argv":["nvim","--listen","/x"]}]}})),
            _ => Err(("invalid_request".into(), "?".into())),
        });
        let c = Client::new(fake.path());
        let s = c.session_snapshot().unwrap();
        assert_eq!(s.agents[0].state_change_seq, 153);
        assert_eq!(s.workspaces[0].tab_count, 1);
        assert_eq!(s.layouts.len(), 1);
        let pi = c.pane_process_info("w7:p1").unwrap();
        assert_eq!(pi.foreground_processes[0].argv[1], "--listen");
    }
}
