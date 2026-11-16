//! Test support: a fake Herdr socket server answering canned JSON. Public (but hidden) so that
//! integration tests in other crates can use it too.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub id: String,
    pub method: String,
    pub params: Value,
}

type Handler = dyn Fn(&str, &Value) -> Result<Value, (String, String)> + Send + Sync + 'static;

/// Listens on a temp unix socket; each connection gets one request and one response line.
/// The handler returns `Ok(result)` or `Err((code, message))`.
pub struct FakeHerdr {
    dir: PathBuf,
    path: PathBuf,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeHerdr {
    pub fn start<F>(handler: F) -> Self
    where
        F: Fn(&str, &Value) -> Result<Value, (String, String)> + Send + Sync + 'static,
    {
        Self::start_with_delay(Duration::ZERO, handler)
    }

    pub fn start_with_delay<F>(delay: Duration, handler: F) -> Self
    where
        F: Fn(&str, &Value) -> Result<Value, (String, String)> + Send + Sync + 'static,
    {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // keep the path short: unix socket paths are limited to 104 bytes on macOS
        let dir = std::env::temp_dir().join(format!("tfh{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::default();
        let handler: Arc<Handler> = Arc::new(handler);
        let reqs = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let handler = handler.clone();
                let reqs = reqs.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let Ok(req) = serde_json::from_str::<Value>(line.trim_end()) else { return };
                    let id = req["id"].as_str().unwrap_or("").to_string();
                    let method = req["method"].as_str().unwrap_or("").to_string();
                    let params = req["params"].clone();
                    reqs.lock().unwrap().push(RecordedRequest { id: id.clone(), method: method.clone(), params: params.clone() });
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    let resp = match handler(&method, &params) {
                        Ok(result) => json!({"id": id, "result": result}),
                        Err((code, message)) => json!({"id": id, "error": {"code": code, "message": message}}),
                    };
                    let mut s = stream;
                    let _ = s.write_all(format!("{resp}\n").as_bytes());
                });
            }
        });
        FakeHerdr { dir, path, requests }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn requests_for(&self, method: &str) -> Vec<RecordedRequest> {
        self.requests().into_iter().filter(|r| r.method == method).collect()
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
