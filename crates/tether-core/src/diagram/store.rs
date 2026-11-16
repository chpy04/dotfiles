//! Per-repo diagram library.
//!
//! Layout under the data dir (`$TETHER_DATA`, else `$XDG_DATA_HOME/tether`, else
//! `~/.local/share/tether`):
//!
//! ```text
//! repos/<repo-name>-<hash8>/
//!   repo.json         repo path, origin URL, created
//!   index.json        per-diagram metadata (id, title, starred, branch, timestamps, ...)
//!   index.json.lock   advisory lock around every read-modify-write of the index
//!   diagrams/<id>.json  the validated spec
//! ```
//!
//! The repo key is the canonicalised `git rev-parse --git-common-dir`, so every worktree of one
//! clone shares a folder. Outside git the canonical cwd is used. The index heals itself: when it is
//! missing or corrupt it is rebuilt from the spec files (a corrupt one is backed up first), and
//! entries and files that disagree are reconciled on every access.

use super::spec::{Diagram, DiagramKind};
use super::validate::{format_errors, normalize, validate_diagram, ValidationError};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default for `[diagrams] keep_days`.
pub const DEFAULT_KEEP_DAYS: u32 = 30;
/// Pruning runs at most this often (seconds).
pub const PRUNE_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Store errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The spec failed semantic validation.
    #[error("invalid diagram spec:\n{}", format_errors(.0))]
    Invalid(Vec<ValidationError>),
    /// No diagram with this id.
    #[error("diagram not found: {0}")]
    NotFound(String),
    /// The id contains characters that cannot appear in a stored id.
    #[error("invalid diagram id \"{0}\" (expected lowercase letters, digits and '-')")]
    BadId(String),
    /// Filesystem error.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// A stored file is not a readable spec.
    #[error("stored diagram {id} is corrupt: {message}")]
    Corrupt {
        /// Diagram id.
        id: String,
        /// Parse error.
        message: String,
    },
}

fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> StoreError {
    let context = context.into();
    move |source| StoreError::Io { context, source }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn secs_of(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ------------------------------------------------------------------------------------ locations

/// The data dir: `$TETHER_DATA`, else `$XDG_DATA_HOME/tether`, else `~/.local/share/tether`.
pub fn data_dir() -> PathBuf {
    data_dir_from(|k| std::env::var_os(k))
}

/// [`data_dir`] with an injectable environment.
pub fn data_dir_from(get: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let set = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(p) = set("TETHER_DATA") {
        return PathBuf::from(p);
    }
    if let Some(x) = set("XDG_DATA_HOME") {
        return PathBuf::from(x).join("tether");
    }
    set("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".local/share/tether")
}

/// Stable 32-bit digest (FNV-1a, folded) as 8 hex digits. `DefaultHasher` is not stable across
/// Rust versions and the result names a folder on disk.
fn hash8(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h ^ (h >> 32)) as u32)
}

fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect();
    let s = s.trim_matches(|c| c == '-' || c == '.').to_string();
    let s: String = s.chars().take(40).collect();
    if s.is_empty() {
        "repo".into()
    } else {
        s
    }
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).current_dir(cwd).output().ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn basename(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Find the worktree root for `cwd` via `git rev-parse --show-toplevel`.
/// Returns `(root, is_git)`; falls back to `cwd` itself when not in a repository.
pub fn worktree_root(cwd: &Path) -> (PathBuf, bool) {
    match git(cwd, &["rev-parse", "--show-toplevel"]) {
        Some(s) => (PathBuf::from(s), true),
        None => (cwd.to_path_buf(), false),
    }
}

/// Which repo a directory belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    key: String,
    name: String,
    /// The main worktree (or the cwd outside git).
    path: PathBuf,
    worktree: PathBuf,
    is_git: bool,
    origin: Option<String>,
}

fn identify(cwd: &Path) -> Identity {
    // one process for both answers; a bare repo has no toplevel, so fall back to the common dir alone
    let both = git(cwd, &["rev-parse", "--show-toplevel", "--git-common-dir"]);
    let (toplevel, common) = match both.as_deref().map(|s| s.lines().collect::<Vec<_>>()) {
        Some(l) if l.len() == 2 => (Some(PathBuf::from(l[0])), Some(l[1].to_string())),
        _ => (None, git(cwd, &["rev-parse", "--git-common-dir"])),
    };
    let Some(common) = common else {
        let c = canon(cwd);
        return Identity {
            key: format!("{}-{}", sanitize_name(&basename(&c)), hash8(&c.to_string_lossy())),
            name: sanitize_name(&basename(&c)),
            path: c.clone(),
            worktree: cwd.to_path_buf(),
            is_git: false,
            origin: None,
        };
    };
    let common = {
        let p = PathBuf::from(&common);
        canon(&if p.is_absolute() { p } else { cwd.join(p) })
    };
    // main worktree: the parent of `<main>/.git`; otherwise (submodule, bare) the toplevel / the dir itself
    let (name_src, path) = if basename(&common) == ".git" {
        let main = common.parent().map(Path::to_path_buf).unwrap_or_else(|| common.clone());
        (basename(&main), main)
    } else if let Some(t) = &toplevel {
        (basename(t), canon(t))
    } else {
        (basename(&common).trim_end_matches(".git").to_string(), common.clone())
    };
    let name = sanitize_name(&name_src);
    Identity {
        key: format!("{name}-{}", hash8(&common.to_string_lossy())),
        name,
        path,
        worktree: toplevel.unwrap_or_else(|| cwd.to_path_buf()),
        is_git: true,
        origin: git(cwd, &["config", "--get", "remote.origin.url"]),
    }
}

fn current_branch(cwd: &Path) -> Option<String> {
    git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).or_else(|| git(cwd, &["symbolic-ref", "--short", "HEAD"]))
}

// ------------------------------------------------------------------------------------ records

/// `repo.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    /// Folder name under `repos/`.
    #[serde(default)]
    pub key: String,
    /// Sanitised basename of the main worktree.
    #[serde(default)]
    pub name: String,
    /// Path of the main worktree (or of the directory outside git).
    pub path: String,
    /// `remote.origin.url`, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
}

/// One diagram's metadata (a row of `index.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    id: String,
    title: String,
    kind: DiagramKind,
    created: u64,
    updated: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_opened: Option<u64>,
    #[serde(default)]
    starred: bool,
    #[serde(default)]
    worktree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

impl Entry {
    fn activity(&self) -> u64 {
        self.updated.max(self.last_opened.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    version: u32,
    /// Unix seconds of the last automatic prune.
    #[serde(default)]
    last_prune: u64,
    /// Worktrees whose legacy `.tether/diagrams/` was imported.
    #[serde(default)]
    imported: Vec<String>,
    #[serde(default)]
    diagrams: Vec<Entry>,
}

/// Summary row returned by [`DiagramStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramInfo {
    /// Stored id.
    pub id: String,
    /// Diagram title.
    pub title: String,
    /// Diagram type.
    pub kind: DiagramKind,
    /// Unix seconds.
    pub created: u64,
    /// Unix seconds of the last change of the spec (or of the metadata for imported files).
    pub updated: u64,
    /// Unix seconds of the last time the viewer opened it.
    pub last_opened: Option<u64>,
    /// Starred diagrams are never pruned.
    pub starred: bool,
    /// Basename of the worktree it was created in.
    pub worktree: String,
    /// Branch checked out when it was created.
    pub branch: Option<String>,
    /// Free-form note.
    pub note: Option<String>,
    /// Repo name (basename of the main worktree).
    pub repo: String,
    /// Repo folder name under `repos/`.
    pub repo_key: String,
}

impl DiagramInfo {
    /// Latest of `updated` and `last_opened`: what sorting and retention look at.
    pub fn activity(&self) -> u64 {
        self.updated.max(self.last_opened.unwrap_or(0))
    }
}

/// Options of [`DiagramStore::create_with`].
#[derive(Debug, Clone, Default)]
pub struct CreateOpts {
    /// Star it right away.
    pub starred: bool,
    /// Free-form note.
    pub note: Option<String>,
}

/// Compact relative age: `now`, `5m`, `3h`, `2d`, `6w`, `4mo`, `2y`.
pub fn format_age(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    let (m, h, d) = (60, 3600, 86_400);
    if s < m {
        "now".into()
    } else if s < h {
        format!("{}m", s / m)
    } else if s < d {
        format!("{}h", s / h)
    } else if s < 14 * d {
        format!("{}d", s / d)
    } else if s < 63 * d {
        format!("{}w", s / (7 * d))
    } else if s < 365 * d {
        format!("{}mo", s / (30 * d))
    } else {
        format!("{}y", s / (365 * d))
    }
}

/// Pick the diagrams a user query means: an exact id, else an exact title (case-insensitive), else
/// every row whose title (or id) contains the query, case-insensitively. Several results mean the
/// query is ambiguous.
pub fn match_query<'a>(rows: &'a [DiagramInfo], query: &str) -> Vec<&'a DiagramInfo> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    if let Some(r) = rows.iter().find(|r| r.id == q) {
        return vec![r];
    }
    let ql = q.to_lowercase();
    let exact: Vec<_> = rows.iter().filter(|r| r.title.to_lowercase() == ql).collect();
    if !exact.is_empty() {
        return exact;
    }
    rows.iter().filter(|r| r.title.to_lowercase().contains(&ql) || r.id.contains(&ql)).collect()
}

// ------------------------------------------------------------------------------------ store

/// A store of validated diagram specs for one repo.
#[derive(Debug, Clone)]
pub struct DiagramStore {
    data: PathBuf,
    id: Identity,
    keep_days: u32,
    migrate: bool,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

fn slugify(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(40).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "diagram".into()
    } else {
        s
    }
}

fn next_counter() -> u64 {
    static C: AtomicU64 = AtomicU64::new(0);
    C.fetch_add(1, Ordering::Relaxed)
}

/// Held for the duration of one read-modify-write; the lock is released when the file closes.
#[allow(dead_code)]
struct Lock(std::fs::File);

/// What a locked section found and did besides its own work.
#[derive(Default)]
struct Side {
    pruned: Vec<DiagramInfo>,
}

impl DiagramStore {
    /// Open the store of the repo containing `cwd`, under [`data_dir`]. Does no I/O besides `git`.
    pub fn open(cwd: &Path) -> Self {
        Self::open_at(cwd, &data_dir())
    }

    /// Like [`DiagramStore::open`] with an explicit data dir (tests).
    pub fn open_at(cwd: &Path, data: &Path) -> Self {
        Self { data: data.to_path_buf(), id: identify(cwd), keep_days: DEFAULT_KEEP_DAYS, migrate: true }
    }

    /// Open a store rooted at an explicit worktree root.
    pub fn at_root(root: impl Into<PathBuf>) -> Self {
        Self::open(&root.into())
    }

    /// The store of an existing repo folder (as listed by [`DiagramStore::all_repos`]).
    pub fn for_key(data: &Path, key: &str) -> Self {
        let meta: RepoInfo = std::fs::read_to_string(data.join("repos").join(key).join("repo.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let name = if meta.name.is_empty() { key.rsplit_once('-').map(|(n, _)| n).unwrap_or(key).to_string() } else { meta.name.clone() };
        let path = PathBuf::from(&meta.path);
        Self {
            data: data.to_path_buf(),
            id: Identity { key: key.to_string(), name, worktree: path.clone(), path, is_git: false, origin: meta.origin },
            keep_days: DEFAULT_KEEP_DAYS,
            migrate: false,
        }
    }

    /// Every repo that has a folder under the data dir, by name.
    pub fn all_repos(data: &Path) -> Vec<DiagramStore> {
        let mut out: Vec<DiagramStore> = std::fs::read_dir(data.join("repos"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .map(|k| Self::for_key(data, &k))
            .collect();
        out.sort_by(|a, b| a.id.name.cmp(&b.id.name).then_with(|| a.id.key.cmp(&b.id.key)));
        out
    }

    /// Retention in days for unstarred diagrams (0 = never prune).
    pub fn with_keep_days(mut self, days: u32) -> Self {
        self.keep_days = days;
        self
    }

    /// The worktree root (the main worktree for stores opened with [`DiagramStore::for_key`]).
    pub fn worktree_root(&self) -> &Path {
        &self.id.worktree
    }

    /// Whether the directory is inside a git repository.
    pub fn is_git(&self) -> bool {
        self.id.is_git
    }

    /// Repo name (basename of the main worktree).
    pub fn repo_name(&self) -> &str {
        &self.id.name
    }

    /// Folder name under `repos/`.
    pub fn repo_key(&self) -> &str {
        &self.id.key
    }

    /// Path of the main worktree (or of the directory outside git).
    pub fn repo_path(&self) -> &Path {
        &self.id.path
    }

    /// The repo's folder under the data dir.
    pub fn repo_dir(&self) -> PathBuf {
        self.data.join("repos").join(&self.id.key)
    }

    /// The directory holding the spec JSON files.
    pub fn dir(&self) -> PathBuf {
        self.repo_dir().join("diagrams")
    }

    fn index_path(&self) -> PathBuf {
        self.repo_dir().join("index.json")
    }

    fn legacy_dir(&self) -> PathBuf {
        self.id.worktree.join(".tether").join("diagrams")
    }

    fn path_for(&self, id: &str) -> Result<PathBuf, StoreError> {
        if !valid_id(id) {
            return Err(StoreError::BadId(id.to_string()));
        }
        Ok(self.dir().join(format!("{id}.json")))
    }

    fn prepare(spec: &Diagram) -> Result<Diagram, StoreError> {
        let mut d = spec.clone();
        normalize(&mut d);
        let errs = validate_diagram(&d);
        if errs.is_empty() {
            Ok(d)
        } else {
            Err(StoreError::Invalid(errs))
        }
    }

    fn write_text_atomic(dir: &Path, path: &Path, text: &str) -> Result<(), StoreError> {
        std::fs::create_dir_all(dir).map_err(io(format!("create {}", dir.display())))?;
        let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), next_counter()));
        let res = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res.map_err(io(format!("write {}", path.display())))
    }

    fn write_spec(&self, id: &str, d: &Diagram) -> Result<(), StoreError> {
        let path = self.path_for(id)?;
        let mut text = serde_json::to_string_pretty(d)
            .map_err(|e| StoreError::Corrupt { id: path.display().to_string(), message: e.to_string() })?;
        text.push('\n');
        Self::write_text_atomic(&self.dir(), &path, &text)
    }

    fn lock(&self) -> Result<Lock, StoreError> {
        let dir = self.repo_dir();
        std::fs::create_dir_all(&dir).map_err(io(format!("create {}", dir.display())))?;
        let path = dir.join("index.json.lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io(format!("open {}", path.display())))?;
        f.lock().map_err(io(format!("lock {}", path.display())))?;
        Ok(Lock(f))
    }

    fn write_repo_json(&self, now: u64) {
        let path = self.repo_dir().join("repo.json");
        if path.exists() || self.id.path.as_os_str().is_empty() {
            return;
        }
        let meta = RepoInfo {
            key: self.id.key.clone(),
            name: self.id.name.clone(),
            path: self.id.path.to_string_lossy().into_owned(),
            origin: self.id.origin.clone(),
            created: now,
        };
        if let Ok(mut t) = serde_json::to_string_pretty(&meta) {
            t.push('\n');
            let _ = Self::write_text_atomic(&self.repo_dir(), &path, &t);
        }
    }

    /// Read the index; a missing or corrupt one is replaced by an empty one. Returns
    /// `(index, rebuilt)`; a corrupt file is backed up and reported on stderr.
    fn load_index(&self, now: u64) -> (Index, bool) {
        let path = self.index_path();
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => return (Index { version: 1, ..Index::default() }, true),
        };
        match serde_json::from_str::<Index>(&body) {
            Ok(mut i) => {
                i.version = 1;
                (i, false)
            }
            Err(e) => {
                let backup = path.with_file_name(format!("index.json.corrupt-{now}"));
                let saved = std::fs::copy(&path, &backup).is_ok();
                eprintln!(
                    "tether: warning: {} is corrupt ({e}); {} and rebuilding it from the diagram files",
                    path.display(),
                    if saved { format!("backed it up to {}", backup.display()) } else { "could not back it up".to_string() }
                );
                (Index { version: 1, ..Index::default() }, true)
            }
        }
    }

    /// Make the index agree with the spec files: drop rows whose file is gone, add rows for files
    /// nobody indexed (title and type read from the spec, times from the file's mtime).
    fn reconcile(&self, idx: &mut Index) -> usize {
        let dir = self.dir();
        let mut files: Vec<(String, u64)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)) else { continue };
                let m = e.metadata().and_then(|m| m.modified()).map(secs_of).unwrap_or(0);
                files.push((id.to_string(), m));
            }
        }
        idx.diagrams.retain(|d| files.iter().any(|(id, _)| *id == d.id));
        let mut added = 0;
        for (id, mtime) in files {
            if idx.diagrams.iter().any(|d| d.id == id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(dir.join(format!("{id}.json"))) else { continue };
            let Ok(d) = serde_json::from_str::<Diagram>(&text) else { continue };
            idx.diagrams.push(Entry {
                id,
                title: d.title().to_string(),
                kind: d.kind(),
                created: mtime,
                updated: mtime,
                last_opened: None,
                starred: false,
                worktree: String::new(),
                branch: None,
                note: None,
            });
            added += 1;
        }
        added
    }

    /// One-time import of `<worktree>/.tether/diagrams/*.json`. Old files stay where they are.
    fn import_legacy(&self, idx: &mut Index, now: u64) {
        if !self.migrate {
            return;
        }
        let legacy = self.legacy_dir();
        let key = canon(&self.id.worktree).to_string_lossy().into_owned();
        if !legacy.is_dir() || idx.imported.contains(&key) {
            return;
        }
        let mut count = 0;
        let mut names: Vec<_> = std::fs::read_dir(&legacy).into_iter().flatten().flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)).map(str::to_string) else { continue };
            if idx.diagrams.iter().any(|d| d.id == id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            let Ok(mut d) = serde_json::from_str::<Diagram>(&text) else { continue };
            normalize(&mut d);
            let mtime = e.metadata().and_then(|m| m.modified()).map(secs_of).unwrap_or(now);
            if self.write_spec(&id, &d).is_err() {
                continue;
            }
            idx.diagrams.push(Entry {
                id,
                title: d.title().to_string(),
                kind: d.kind(),
                created: mtime,
                updated: mtime,
                last_opened: None,
                starred: false,
                worktree: basename(&self.id.worktree),
                branch: None,
                note: None,
            });
            count += 1;
        }
        idx.imported.push(key);
        if count > 0 {
            // first prune a day from now, so an import can never delete what it just brought in
            idx.last_prune = now;
            eprintln!(
                "tether: imported {count} diagram{} from {} into {}; the old folder can be deleted",
                if count == 1 { "" } else { "s" },
                legacy.display(),
                self.dir().display()
            );
        }
    }

    fn info_of(&self, e: &Entry) -> DiagramInfo {
        DiagramInfo {
            id: e.id.clone(),
            title: e.title.clone(),
            kind: e.kind,
            created: e.created,
            updated: e.updated,
            last_opened: e.last_opened,
            starred: e.starred,
            worktree: e.worktree.clone(),
            branch: e.branch.clone(),
            note: e.note.clone(),
            repo: self.id.name.clone(),
            repo_key: self.id.key.clone(),
        }
    }

    fn prunable(&self, idx: &Index, now: u64) -> Vec<String> {
        if self.keep_days == 0 {
            return Vec::new();
        }
        let limit = u64::from(self.keep_days) * 86_400;
        idx.diagrams.iter().filter(|e| !e.starred && now.saturating_sub(e.activity()) > limit).map(|e| e.id.clone()).collect()
    }

    fn remove_entries(&self, idx: &mut Index, ids: &[String]) -> Vec<DiagramInfo> {
        let mut gone = Vec::new();
        for id in ids {
            if let Some(pos) = idx.diagrams.iter().position(|e| e.id == *id) {
                let e = idx.diagrams.remove(pos);
                if let Ok(p) = self.path_for(&e.id) {
                    let _ = std::fs::remove_file(p);
                }
                gone.push(self.info_of(&e));
            }
        }
        gone
    }

    /// The locked read-modify-write every operation goes through: take the lock, load (and heal)
    /// the index, import the legacy folder, optionally prune (at most daily), run `f`, and write
    /// the index back atomically when it changed.
    fn with_index<T>(
        &self,
        now: u64,
        prune: bool,
        f: impl FnOnce(&mut Index) -> Result<T, StoreError>,
    ) -> Result<(T, Side), StoreError> {
        let _lock = self.lock()?;
        self.write_repo_json(now);
        let (mut idx, rebuilt) = self.load_index(now);
        let before = serde_json::to_string(&idx).unwrap_or_default();
        let added = self.reconcile(&mut idx);
        if rebuilt && added > 0 {
            idx.last_prune = now; // healed rows carry file mtimes; give them a day before any prune
        }
        self.import_legacy(&mut idx, now);
        let mut side = Side::default();
        if prune && self.keep_days > 0 && now.saturating_sub(idx.last_prune) >= PRUNE_INTERVAL_SECS {
            let ids = self.prunable(&idx, now);
            side.pruned = self.remove_entries(&mut idx, &ids);
            idx.last_prune = now;
        }
        let out = f(&mut idx);
        // write even when `f` failed: the reconcile/import/prune work above already touched the files
        if rebuilt || serde_json::to_string(&idx).unwrap_or_default() != before {
            let mut text = serde_json::to_string_pretty(&idx)
                .map_err(|e| StoreError::Corrupt { id: "index.json".into(), message: e.to_string() })?;
            text.push('\n');
            Self::write_text_atomic(&self.repo_dir(), &self.index_path(), &text)?;
        }
        out.map(|v| (v, side))
    }

    fn report_pruned(&self, side: &Side) {
        if !side.pruned.is_empty() {
            eprintln!(
                "tether: pruned {} unstarred diagram{} not opened for {}+ days ({}); star a diagram to keep it",
                side.pruned.len(),
                if side.pruned.len() == 1 { "" } else { "s" },
                self.keep_days,
                side.pruned.iter().map(|p| p.title.as_str()).collect::<Vec<_>>().join(", ")
            );
        }
    }

    /// True when a read can skip the locked section: this repo has no folder and nothing to import.
    fn nothing_here(&self) -> bool {
        !self.repo_dir().exists() && !(self.migrate && self.legacy_dir().is_dir())
    }

    // -------------------------------------------------------------------------------- operations

    /// Validate and store a new diagram; returns its id (`<slug>-<6 hex>`).
    pub fn create(&self, spec: &Diagram) -> Result<String, StoreError> {
        self.create_with(spec, &CreateOpts::default())
    }

    /// [`DiagramStore::create`] with options.
    pub fn create_with(&self, spec: &Diagram, opts: &CreateOpts) -> Result<String, StoreError> {
        self.create_at(now_secs(), spec, opts)
    }

    fn create_at(&self, now: u64, spec: &Diagram, opts: &CreateOpts) -> Result<String, StoreError> {
        let d = Self::prepare(spec)?;
        let branch = if self.id.is_git { current_branch(&self.id.worktree) } else { None };
        let (id, side) = self.with_index(now, true, |idx| {
            let slug = slugify(d.title());
            let mut id;
            loop {
                let mut h = DefaultHasher::new();
                d.title().hash(&mut h);
                std::process::id().hash(&mut h);
                next_counter().hash(&mut h);
                SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0).hash(&mut h);
                id = format!("{slug}-{:06x}", h.finish() & 0xff_ffff);
                if !idx.diagrams.iter().any(|e| e.id == id) && !self.path_for(&id)?.exists() {
                    break;
                }
            }
            self.write_spec(&id, &d)?;
            idx.diagrams.push(Entry {
                id: id.clone(),
                title: d.title().to_string(),
                kind: d.kind(),
                created: now,
                updated: now,
                last_opened: None,
                starred: opts.starred,
                worktree: basename(&self.id.worktree),
                branch,
                note: opts.note.clone(),
            });
            Ok(id)
        })?;
        self.report_pruned(&side);
        Ok(id)
    }

    /// Read a spec file without importing anything (safe inside the index lock).
    fn read_spec(&self, id: &str) -> Result<Diagram, StoreError> {
        let path = self.path_for(id)?;
        match std::fs::read_to_string(&path) {
            Ok(t) => Self::parse(id, &t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StoreError::NotFound(id.to_string())),
            Err(e) => Err(io(format!("read {}", path.display()))(e)),
        }
    }

    /// Load a stored diagram.
    pub fn get(&self, id: &str) -> Result<Diagram, StoreError> {
        let path = self.path_for(id)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // maybe it is still in the legacy folder of this worktree: importing is part of any access
                if self.migrate && self.legacy_dir().is_dir() {
                    self.with_index(now_secs(), false, |_| Ok(()))?;
                    if let Ok(t) = std::fs::read_to_string(&path) {
                        return Self::parse(id, &t);
                    }
                }
                return Err(StoreError::NotFound(id.to_string()));
            }
            Err(e) => return Err(io(format!("read {}", path.display()))(e)),
        };
        Self::parse(id, &text)
    }

    fn parse(id: &str, text: &str) -> Result<Diagram, StoreError> {
        let mut d: Diagram = serde_json::from_str(text).map_err(|e| StoreError::Corrupt { id: id.to_string(), message: e.to_string() })?;
        normalize(&mut d);
        Ok(d)
    }

    /// List stored diagrams, newest activity first. Prunes opportunistically (at most daily) and
    /// says so on stderr when it removed something; see [`DiagramStore::rows`] for a quiet read.
    pub fn list(&self) -> Result<Vec<DiagramInfo>, StoreError> {
        self.snapshot(now_secs(), true)
    }

    /// Like [`DiagramStore::list`] without ever pruning (for UIs that own the terminal).
    pub fn rows(&self) -> Result<Vec<DiagramInfo>, StoreError> {
        self.snapshot(now_secs(), false)
    }

    fn snapshot(&self, now: u64, prune: bool) -> Result<Vec<DiagramInfo>, StoreError> {
        if self.nothing_here() {
            return Ok(Vec::new());
        }
        let (rows, side) = self.with_index(now, prune, |idx| Ok(idx.diagrams.iter().map(|e| self.info_of(e)).collect::<Vec<_>>()))?;
        self.report_pruned(&side);
        let mut rows = rows;
        rows.sort_by(|a, b| b.activity().cmp(&a.activity()).then_with(|| b.created.cmp(&a.created)).then_with(|| a.id.cmp(&b.id)));
        Ok(rows)
    }

    /// Metadata of one diagram.
    pub fn info(&self, id: &str) -> Result<DiagramInfo, StoreError> {
        if !valid_id(id) {
            return Err(StoreError::BadId(id.to_string()));
        }
        self.rows()?.into_iter().find(|r| r.id == id).ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// Diagrams matching a user query (see [`match_query`]).
    pub fn find(&self, query: &str) -> Result<Vec<DiagramInfo>, StoreError> {
        let rows = self.rows()?;
        Ok(match_query(&rows, query).into_iter().cloned().collect())
    }

    /// Validate an id for a mutation and fail early (creating nothing) when this repo has no library yet.
    fn guard(&self, id: &str) -> Result<(), StoreError> {
        self.path_for(id)?;
        if self.nothing_here() {
            return Err(StoreError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn entry_mut<'a>(idx: &'a mut Index, id: &str) -> Result<&'a mut Entry, StoreError> {
        idx.diagrams.iter_mut().find(|e| e.id == id).ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// Replace the spec stored under `id` (the id does not change).
    pub fn update(&self, id: &str, spec: &Diagram) -> Result<(), StoreError> {
        self.guard(id)?;
        let d = Self::prepare(spec)?;
        self.with_index(now_secs(), false, |idx| {
            let e = Self::entry_mut(idx, id)?;
            e.title = d.title().to_string();
            e.kind = d.kind();
            e.updated = now_secs();
            self.write_spec(id, &d)
        })
        .map(|_| ())
    }

    /// Change the title (in the spec and the index); the id stays.
    pub fn rename(&self, id: &str, title: &str) -> Result<(), StoreError> {
        self.guard(id)?;
        let title = title.trim();
        self.with_index(now_secs(), false, |idx| {
            let e = Self::entry_mut(idx, id)?;
            let mut d = self.read_spec(id)?;
            d.set_title(title);
            let d = Self::prepare(&d)?;
            e.title = d.title().to_string();
            e.updated = now_secs();
            self.write_spec(id, &d)
        })
        .map(|_| ())
    }

    /// Star or unstar. Returns whether the state changed.
    pub fn set_starred(&self, id: &str, starred: bool) -> Result<bool, StoreError> {
        self.guard(id)?;
        self.with_index(now_secs(), false, |idx| {
            let e = Self::entry_mut(idx, id)?;
            let changed = e.starred != starred;
            e.starred = starred;
            Ok(changed)
        })
        .map(|(c, _)| c)
    }

    /// Set or clear the note.
    pub fn set_note(&self, id: &str, note: Option<&str>) -> Result<(), StoreError> {
        self.guard(id)?;
        self.with_index(now_secs(), false, |idx| {
            Self::entry_mut(idx, id)?.note = note.map(str::to_string).filter(|n| !n.is_empty());
            Ok(())
        })
        .map(|_| ())
    }

    /// Record that the viewer opened the diagram now.
    pub fn touch_opened(&self, id: &str) -> Result<(), StoreError> {
        self.touch_opened_at(id, now_secs())
    }

    fn touch_opened_at(&self, id: &str, now: u64) -> Result<(), StoreError> {
        self.guard(id)?;
        self.with_index(now, false, |idx| {
            Self::entry_mut(idx, id)?.last_opened = Some(now);
            Ok(())
        })
        .map(|_| ())
    }

    /// Delete a stored diagram.
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.guard(id)?;
        self.with_index(now_secs(), false, |idx| {
            let gone = self.remove_entries(idx, &[id.to_string()]);
            if gone.is_empty() {
                return Err(StoreError::NotFound(id.to_string()));
            }
            Ok(())
        })
        .map(|_| ())
    }

    /// Remove unstarred diagrams older than `keep_days` (by `last_opened`/`updated`), regardless of
    /// the once-a-day guard. With `dry_run` nothing is touched. Returns what was (or would be) removed.
    pub fn prune(&self, dry_run: bool) -> Result<Vec<DiagramInfo>, StoreError> {
        self.prune_at(now_secs(), dry_run)
    }

    fn prune_at(&self, now: u64, dry_run: bool) -> Result<Vec<DiagramInfo>, StoreError> {
        if self.nothing_here() {
            return Ok(Vec::new());
        }
        self.with_index(now, false, |idx| {
            let ids = self.prunable(idx, now);
            if dry_run {
                return Ok(idx.diagrams.iter().filter(|e| ids.contains(&e.id)).map(|e| self.info_of(e)).collect());
            }
            let gone = self.remove_entries(idx, &ids);
            idx.last_prune = now;
            Ok(gone)
        })
        .map(|(v, _)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::validate::validate;
    use std::sync::Arc;

    const DAY: u64 = 86_400;

    fn git_in(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn git_repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        git_in(d.path(), &["init", "-q", "-b", "main"]);
        git_in(d.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        d
    }

    fn sample(title: &str) -> Diagram {
        validate(&format!(r#"{{"type":"flowchart","title":"{title}","nodes":[{{"id":"a","label":"A"}}],"edges":[]}}"#)).unwrap()
    }

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, DiagramStore) {
        let repo = git_repo();
        let data = tempfile::tempdir().unwrap();
        let store = DiagramStore::open_at(repo.path(), data.path());
        (repo, data, store)
    }

    fn now() -> u64 {
        now_secs()
    }

    #[test]
    fn data_dir_precedence() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
        };
        assert_eq!(data_dir_from(env(&[("TETHER_DATA", "/t"), ("XDG_DATA_HOME", "/x"), ("HOME", "/h")])), PathBuf::from("/t"));
        assert_eq!(data_dir_from(env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")])), PathBuf::from("/x/tether"));
        assert_eq!(data_dir_from(env(&[("TETHER_DATA", ""), ("HOME", "/h")])), PathBuf::from("/h/.local/share/tether"));
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slugify("Session refresh!"), "session-refresh");
        assert_eq!(slugify("  --  "), "diagram");
        assert_eq!(slugify("Ünï cødé"), "n-c-d");
        assert!(slugify(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn age_format() {
        assert_eq!(format_age(100, 100), "now");
        assert_eq!(format_age(10_000, 10_000 - 300), "5m");
        assert_eq!(format_age(100_000, 100_000 - 3 * 3600), "3h");
        assert_eq!(format_age(10 * DAY, 8 * DAY), "2d");
        assert_eq!(format_age(100 * DAY, 100 * DAY - 21 * DAY), "3w");
        assert_eq!(format_age(400 * DAY, 400 * DAY - 120 * DAY), "4mo");
        assert_eq!(format_age(1000 * DAY, 100 * DAY), "2y");
    }

    #[test]
    fn query_matching_tiers() {
        let (_r, _d, store) = setup();
        let a = store.create(&sample("Session refresh")).unwrap();
        let b = store.create(&sample("Session logout")).unwrap();
        let c = store.create(&sample("Cache layout")).unwrap();
        let rows = store.rows().unwrap();
        let ids = |q: &str| {
            let mut v: Vec<String> = match_query(&rows, q).into_iter().map(|r| r.id.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(ids(&a), vec![a.clone()]);
        assert_eq!(ids("cache"), vec![c.clone()]);
        assert_eq!(ids("SESSION REFRESH"), vec![a.clone()]);
        let mut both = vec![a.clone(), b.clone()];
        both.sort();
        assert_eq!(ids("session"), both);
        assert!(ids("nope").is_empty());
        assert!(ids("  ").is_empty());
    }

    #[test]
    fn round_trip_in_git_repo_from_subdir() {
        let repo = git_repo();
        let data = tempfile::tempdir().unwrap();
        let sub = repo.path().join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        let store = DiagramStore::open_at(&sub, data.path());
        assert_eq!(store.worktree_root().canonicalize().unwrap(), repo.path().canonicalize().unwrap());
        assert!(store.list().unwrap().is_empty());
        assert!(!data.path().join("repos").exists(), "reads create nothing for an unused repo");

        let d = sample("Session refresh");
        let id = store.create(&d).unwrap();
        assert!(id.starts_with("session-refresh-"), "{id}");
        assert_eq!(id.len(), "session-refresh-".len() + 6);
        assert_eq!(store.get(&id).unwrap(), d);
        assert!(store.dir().starts_with(data.path().join("repos")));
        assert!(store.dir().join(format!("{id}.json")).is_file());
        assert!(store.repo_dir().join("repo.json").is_file());
        assert!(store.repo_dir().join("index.json").is_file());

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].title, "Session refresh");
        assert_eq!(list[0].kind, DiagramKind::Flowchart);
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[0].worktree, basename(&repo.path().canonicalize().unwrap()));
        assert!(!list[0].starred);

        let d2 = sample("Renamed");
        store.update(&id, &d2).unwrap();
        assert_eq!(store.get(&id).unwrap().title(), "Renamed");
        assert_eq!(store.rows().unwrap()[0].title, "Renamed");

        store.delete(&id).unwrap();
        assert!(matches!(store.get(&id), Err(StoreError::NotFound(_))));
        assert!(matches!(store.delete(&id), Err(StoreError::NotFound(_))));
        assert!(matches!(store.update(&id, &d2), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn repo_json_records_path_origin_and_name() {
        let repo = git_repo();
        git_in(repo.path(), &["remote", "add", "origin", "git@example.com:me/proj.git"]);
        let data = tempfile::tempdir().unwrap();
        let store = DiagramStore::open_at(repo.path(), data.path());
        store.create(&sample("X")).unwrap();
        let meta: RepoInfo = serde_json::from_str(&std::fs::read_to_string(store.repo_dir().join("repo.json")).unwrap()).unwrap();
        assert_eq!(meta.origin.as_deref(), Some("git@example.com:me/proj.git"));
        assert_eq!(PathBuf::from(&meta.path), repo.path().canonicalize().unwrap());
        assert_eq!(meta.name, sanitize_name(&basename(&repo.path().canonicalize().unwrap())));
        assert!(meta.created > 0);
        let key = store.repo_key().to_string();
        assert!(key.starts_with(&format!("{}-", meta.name)), "{key}");
        assert_eq!(key.len(), meta.name.len() + 1 + 8);
    }

    #[test]
    fn worktrees_of_one_clone_share_a_folder() {
        let repo = git_repo();
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("feature-wt");
        git_in(repo.path(), &["worktree", "add", "-q", "-b", "feat-x", wt.to_str().unwrap()]);
        let data = tempfile::tempdir().unwrap();
        let main = DiagramStore::open_at(repo.path(), data.path());
        let linked = DiagramStore::open_at(&wt, data.path());
        assert_eq!(main.repo_key(), linked.repo_key());
        assert_eq!(main.dir(), linked.dir());
        assert_eq!(linked.repo_name(), main.repo_name());
        assert_ne!(main.worktree_root(), linked.worktree_root());

        let id = linked.create(&sample("From the worktree")).unwrap();
        let rows = main.list().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].branch.as_deref(), Some("feat-x"));
        assert_eq!(rows[0].worktree, "feature-wt");
        assert_eq!(main.get(&id).unwrap().title(), "From the worktree");
        // a different clone gets a different folder, even with the same basename
        let other = git_repo();
        assert_ne!(DiagramStore::open_at(other.path(), data.path()).repo_key(), main.repo_key());
    }

    #[test]
    fn non_git_dir_uses_the_canonical_cwd() {
        let d = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let (_, is_git) = worktree_root(d.path());
        if is_git {
            return; // odd setup: tempdirs inside a repo
        }
        let store = DiagramStore::open_at(d.path(), data.path());
        assert!(!store.is_git());
        let id = store.create(&sample("Loose")).unwrap();
        assert_eq!(store.rows().unwrap()[0].branch, None);
        assert_eq!(store.repo_path(), d.path().canonicalize().unwrap());
        // the same dir reached through a symlink maps to the same folder
        let link_dir = tempfile::tempdir().unwrap();
        let link = link_dir.path().join("lnk");
        std::os::unix::fs::symlink(d.path(), &link).unwrap();
        let again = DiagramStore::open_at(&link, data.path());
        assert_eq!(again.repo_key(), store.repo_key());
        assert_eq!(again.get(&id).unwrap().title(), "Loose");
        assert!(!d.path().join(".gitignore").exists());
        assert!(!d.path().join(".tether").exists());
    }

    #[test]
    fn create_never_touches_the_repo() {
        let (repo, _d, store) = setup();
        store.create(&sample("X")).unwrap();
        assert!(!repo.path().join(".gitignore").exists());
        assert!(!repo.path().join(".tether").exists());
        let out = Command::new("git").args(["status", "--porcelain", "--untracked-files=all"]).current_dir(repo.path()).output().unwrap();
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn same_title_gets_unique_ids() {
        let (_r, _d, store) = setup();
        let d = sample("Same");
        let ids: std::collections::HashSet<String> = (0..20).map(|_| store.create(&d).unwrap()).collect();
        assert_eq!(ids.len(), 20);
        assert_eq!(store.list().unwrap().len(), 20);
    }

    #[test]
    fn invalid_spec_rejected_and_bad_ids_refused() {
        let (_r, _d, store) = setup();
        let mut d = sample("X");
        if let Diagram::Flowchart(f) = &mut d {
            f.edges.push(crate::diagram::Edge { from: "a".into(), to: "ghost".into(), label: None, style: None });
        }
        assert!(matches!(store.create(&d), Err(StoreError::Invalid(_))));
        assert!(store.list().unwrap().is_empty());
        for bad in ["../x", "a/b", "", "UP", "-x"] {
            assert!(matches!(store.get(bad), Err(StoreError::BadId(_))), "{bad}");
            assert!(matches!(store.set_starred(bad, true), Err(StoreError::BadId(_))), "{bad}");
        }
    }

    #[test]
    fn sequence_ids_persist_after_create() {
        let (_r, _d, store) = setup();
        let d = validate(
            r#"{"type":"sequence","title":"S","participants":[{"id":"a","label":"A"},{"id":"b","label":"B"}],
            "items":[{"type":"message","from":"a","to":"b","label":"hi"}]}"#,
        )
        .unwrap();
        let id = store.create(&d).unwrap();
        let text = std::fs::read_to_string(store.dir().join(format!("{id}.json"))).unwrap();
        assert!(text.contains("\"m1\""));
    }

    #[test]
    fn star_rename_note_touch_rm() {
        let (_r, _d, store) = setup();
        let id = store.create(&sample("Alpha")).unwrap();
        assert!(store.set_starred(&id, true).unwrap());
        assert!(!store.set_starred(&id, true).unwrap(), "already starred");
        assert!(store.info(&id).unwrap().starred);

        store.rename(&id, "  Beta  ").unwrap();
        assert_eq!(store.info(&id).unwrap().title, "Beta");
        assert_eq!(store.get(&id).unwrap().title(), "Beta", "the spec changes too");
        assert!(store.info(&id).unwrap().starred, "rename keeps metadata");
        assert!(matches!(store.rename(&id, "   "), Err(StoreError::Invalid(_))));
        assert_eq!(store.info(&id).unwrap().title, "Beta", "a refused rename changes nothing");
        assert!(id.starts_with("alpha-"), "the id stays");

        store.set_note(&id, Some("remember this")).unwrap();
        assert_eq!(store.info(&id).unwrap().note.as_deref(), Some("remember this"));
        assert_eq!(store.info(&id).unwrap().last_opened, None);
        store.touch_opened(&id).unwrap();
        assert!(store.info(&id).unwrap().last_opened.unwrap() >= now() - 2);

        store.set_starred(&id, false).unwrap();
        assert!(!store.info(&id).unwrap().starred);
        assert!(matches!(store.set_starred("ghost-000000", true), Err(StoreError::NotFound(_))));
        assert!(matches!(store.rename("ghost-000000", "x"), Err(StoreError::NotFound(_))));
        assert!(matches!(store.touch_opened("ghost-000000"), Err(StoreError::NotFound(_))));

        store.delete(&id).unwrap();
        assert!(store.rows().unwrap().is_empty());
        assert!(!store.dir().join(format!("{id}.json")).exists());
    }

    #[test]
    fn create_with_star_and_list_order() {
        let (_r, _d, store) = setup();
        let t = now();
        let a = store.create_at(t - 300, &sample("Old"), &CreateOpts::default()).unwrap();
        let b = store.create_at(t - 100, &sample("Mid"), &CreateOpts { starred: true, note: Some("n".into()) }).unwrap();
        let c = store.create_at(t - 200, &sample("Opened"), &CreateOpts::default()).unwrap();
        store.touch_opened_at(&c, t).unwrap();
        let rows = store.rows().unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![c.as_str(), b.as_str(), a.as_str()]);
        assert!(rows[1].starred);
        assert_eq!(rows[1].note.as_deref(), Some("n"));
    }

    // ---------------------------------------------------------------- self-healing

    #[test]
    fn missing_index_is_rebuilt_from_spec_files() {
        let (_r, _d, store) = setup();
        let a = store.create(&sample("One")).unwrap();
        let b = store.create(&sample("Two")).unwrap();
        std::fs::remove_file(store.repo_dir().join("index.json")).unwrap();
        let rows = store.rows().unwrap();
        let mut ids: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(ids, want);
        assert!(rows.iter().any(|r| r.title == "One") && rows.iter().any(|r| r.title == "Two"));
        assert!(store.repo_dir().join("index.json").is_file(), "the rebuilt index is written back");
    }

    #[test]
    fn corrupt_index_is_backed_up_and_rebuilt() {
        let (_r, _d, store) = setup();
        let a = store.create(&sample("One")).unwrap();
        store.set_starred(&a, true).unwrap();
        std::fs::write(store.repo_dir().join("index.json"), "{ not json").unwrap();
        let rows = store.rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, a);
        let backups: Vec<_> = std::fs::read_dir(store.repo_dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("index.json.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), "{ not json");
        // and the index is valid again
        serde_json::from_str::<Index>(&std::fs::read_to_string(store.repo_dir().join("index.json")).unwrap()).unwrap();
    }

    #[test]
    fn index_and_files_are_reconciled() {
        let (_r, _d, store) = setup();
        let a = store.create(&sample("Keep")).unwrap();
        let b = store.create(&sample("Vanishes")).unwrap();
        std::fs::remove_file(store.dir().join(format!("{b}.json"))).unwrap();
        // a stray valid spec copied in by hand, and junk that must be ignored
        std::fs::write(store.dir().join("hand-made-abc123.json"), serde_json::to_string(&sample("Hand made")).unwrap()).unwrap();
        std::fs::write(store.dir().join("junk.json"), "nope").unwrap();
        std::fs::write(store.dir().join("notes.txt"), "x").unwrap();
        let rows = store.rows().unwrap();
        let mut ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, {
            let mut v = vec![a.as_str(), "hand-made-abc123"];
            v.sort();
            v
        });
    }

    // ---------------------------------------------------------------- concurrency

    #[test]
    fn concurrent_creates_and_stars_lose_nothing() {
        let (_r, _d, store) = setup();
        let store = Arc::new(store);
        let first = store.create(&sample("Seed")).unwrap();
        let mut handles = Vec::new();
        for t in 0..6 {
            let s = store.clone();
            let first = first.clone();
            handles.push(std::thread::spawn(move || {
                let mut mine = Vec::new();
                for i in 0..8 {
                    mine.push(s.create(&sample(&format!("T{t} n{i}"))).unwrap());
                    s.touch_opened(&first).unwrap();
                    s.set_starred(&mine[i], i % 2 == 0).unwrap();
                }
                mine
            }));
        }
        let mut all: Vec<String> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        all.push(first);
        let rows = store.rows().unwrap();
        assert_eq!(rows.len(), 6 * 8 + 1);
        let mut got: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        got.sort();
        all.sort();
        assert_eq!(got, all);
        assert_eq!(rows.iter().filter(|r| r.starred).count(), 6 * 4);
        let files = std::fs::read_dir(store.dir()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".json")).count();
        assert_eq!(files, 6 * 8 + 1);
        assert!(rows.iter().find(|r| r.title == "Seed").unwrap().last_opened.is_some());
    }

    // ---------------------------------------------------------------- retention

    /// Rewrite the timestamps of a stored diagram directly in the index.
    fn age_entry(store: &DiagramStore, id: &str, updated: u64, last_opened: Option<u64>) {
        let p = store.repo_dir().join("index.json");
        let mut idx: Index = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let e = idx.diagrams.iter_mut().find(|e| e.id == id).unwrap();
        e.updated = updated;
        e.created = updated;
        e.last_opened = last_opened;
        std::fs::write(p, serde_json::to_string(&idx).unwrap()).unwrap();
    }

    fn set_last_prune(store: &DiagramStore, t: u64) {
        let p = store.repo_dir().join("index.json");
        let mut idx: Index = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        idx.last_prune = t;
        std::fs::write(p, serde_json::to_string(&idx).unwrap()).unwrap();
    }

    fn last_prune(store: &DiagramStore) -> u64 {
        let idx: Index = serde_json::from_str(&std::fs::read_to_string(store.repo_dir().join("index.json")).unwrap()).unwrap();
        idx.last_prune
    }

    #[test]
    fn prune_removes_old_unstarred_only_and_respects_keep_days() {
        let (_r, _d, store) = setup();
        let t = now();
        let old = store.create(&sample("Old")).unwrap();
        let old_starred = store.create(&sample("Old starred")).unwrap();
        let reopened = store.create(&sample("Old but reopened")).unwrap();
        let fresh = store.create(&sample("Fresh")).unwrap();
        let edge = store.create(&sample("Just inside")).unwrap();
        store.set_starred(&old_starred, true).unwrap();
        age_entry(&store, &old, t - 40 * DAY, None);
        age_entry(&store, &old_starred, t - 400 * DAY, None);
        age_entry(&store, &reopened, t - 90 * DAY, Some(t - 2 * DAY));
        age_entry(&store, &edge, t - 29 * DAY, None);

        let dry = store.prune_at(t, true).unwrap();
        assert_eq!(dry.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec![old.as_str()]);
        assert_eq!(store.rows().unwrap().len(), 5, "dry run removes nothing");
        assert!(store.dir().join(format!("{old}.json")).exists());

        let gone = store.prune_at(t, false).unwrap();
        assert_eq!(gone.len(), 1);
        let left: Vec<_> = store.rows().unwrap().into_iter().map(|r| r.id).collect();
        assert!(!left.contains(&old));
        for id in [&old_starred, &reopened, &fresh, &edge] {
            assert!(left.contains(id), "{id}");
        }
        assert!(!store.dir().join(format!("{old}.json")).exists());

        // keep_days = 0 never prunes; a longer window keeps what a shorter one removes
        age_entry(&store, &edge, t - 300 * DAY, None);
        assert!(store.clone().with_keep_days(0).prune_at(t, false).unwrap().is_empty());
        assert!(store.clone().with_keep_days(365).prune_at(t, false).unwrap().is_empty());
        assert_eq!(store.clone().with_keep_days(7).prune_at(t, true).unwrap().len(), 1);
    }

    #[test]
    fn automatic_prune_runs_on_list_and_create_at_most_daily() {
        let (_r, _d, store) = setup();
        let t = now();
        let a = store.create_at(t, &sample("Stale a"), &CreateOpts::default()).unwrap();
        let b = store.create_at(t, &sample("Stale b"), &CreateOpts::default()).unwrap();
        age_entry(&store, &a, t - 60 * DAY, None);
        age_entry(&store, &b, t - 60 * DAY, None);

        // pruned a minute ago: the guard holds even though both are stale
        set_last_prune(&store, t - 60);
        assert_eq!(store.snapshot(t, true).unwrap().len(), 2);
        assert_eq!(last_prune(&store), t - 60, "no prune, no new timestamp");
        // rows() never prunes, however old the guard
        set_last_prune(&store, 1);
        assert_eq!(store.snapshot(t, false).unwrap().len(), 2);

        // a day later list prunes and records it
        set_last_prune(&store, t - PRUNE_INTERVAL_SECS);
        assert!(store.snapshot(t, true).unwrap().is_empty());
        assert_eq!(last_prune(&store), t);

        // create prunes too (before adding its own diagram), and also only once a day
        let c = store.create_at(t, &sample("Stale c"), &CreateOpts::default()).unwrap();
        age_entry(&store, &c, t - 60 * DAY, None);
        set_last_prune(&store, t - 100);
        store.create_at(t, &sample("New 1"), &CreateOpts::default()).unwrap();
        assert_eq!(store.rows().unwrap().len(), 2, "guarded: c survives");
        set_last_prune(&store, t - 2 * PRUNE_INTERVAL_SECS);
        store.create_at(t, &sample("New 2"), &CreateOpts::default()).unwrap();
        let titles: Vec<_> = store.rows().unwrap().into_iter().map(|r| r.title).collect();
        assert!(!titles.contains(&"Stale c".to_string()), "{titles:?}");
        assert!(titles.contains(&"New 1".to_string()) && titles.contains(&"New 2".to_string()));
    }

    #[test]
    fn automatic_prune_never_removes_starred_or_runs_with_keep_days_zero() {
        let (_r, _d, store) = setup();
        let t = now();
        let a = store.create_at(t, &sample("Starred"), &CreateOpts { starred: true, note: None }).unwrap();
        let b = store.create_at(t, &sample("Plain"), &CreateOpts::default()).unwrap();
        age_entry(&store, &a, t - 500 * DAY, None);
        age_entry(&store, &b, t - 500 * DAY, None);
        set_last_prune(&store, 0);
        assert_eq!(store.clone().with_keep_days(0).snapshot(t, true).unwrap().len(), 2);
        let rows = store.snapshot(t, true).unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![a.as_str()]);
    }

    // ---------------------------------------------------------------- migration

    fn write_legacy(root: &Path, id: &str, title: &str) -> PathBuf {
        let dir = root.join(".tether/diagrams");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{id}.json"));
        std::fs::write(&p, serde_json::to_string_pretty(&sample(title)).unwrap()).unwrap();
        p
    }

    fn set_mtime(p: &Path, secs: u64) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn legacy_diagrams_are_imported_once_keeping_ids_and_old_files() {
        let (repo, _d, store) = setup();
        let p1 = write_legacy(repo.path(), "first-aaaaaa", "First");
        let p2 = write_legacy(repo.path(), "second-bbbbbb", "Second");
        std::fs::write(repo.path().join(".tether/diagrams/broken-cccccc.json"), "{").unwrap();
        let t = now();
        set_mtime(&p1, t - 10 * DAY);
        set_mtime(&p2, t - 3 * DAY);

        let rows = store.list().unwrap();
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["second-bbbbbb", "first-aaaaaa"]);
        assert_eq!(rows[1].updated, t - 10 * DAY, "updated comes from the file mtime");
        assert_eq!(rows[0].worktree, basename(&repo.path().canonicalize().unwrap()));
        assert_eq!(store.get("first-aaaaaa").unwrap().title(), "First");
        assert!(p1.exists() && p2.exists(), "old files stay");

        // once only: a deleted diagram does not come back, a new legacy file is left alone
        store.delete("first-aaaaaa").unwrap();
        write_legacy(repo.path(), "late-dddddd", "Late");
        let ids: Vec<_> = store.rows().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["second-bbbbbb".to_string()]);
    }

    #[test]
    fn get_imports_on_a_miss_and_import_defers_the_first_prune() {
        let (repo, _d, store) = setup();
        let p = write_legacy(repo.path(), "ancient-eeeeee", "Ancient");
        set_mtime(&p, now() - 200 * DAY);
        assert_eq!(store.get("ancient-eeeeee").unwrap().title(), "Ancient");
        // a stale import survives the next list: the first prune is a day away
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(matches!(store.get("nope-ffffff"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn two_worktrees_import_their_own_legacy_folders() {
        let repo = git_repo();
        let wt_dir = tempfile::tempdir().unwrap();
        let wt = wt_dir.path().join("wt2");
        git_in(repo.path(), &["worktree", "add", "-q", "-b", "other", wt.to_str().unwrap()]);
        let data = tempfile::tempdir().unwrap();
        write_legacy(repo.path(), "main-aaaaaa", "In main");
        write_legacy(&wt, "wt-bbbbbb", "In wt");
        let main = DiagramStore::open_at(repo.path(), data.path());
        assert_eq!(main.rows().unwrap().len(), 1);
        let linked = DiagramStore::open_at(&wt, data.path());
        let mut ids: Vec<_> = linked.rows().unwrap().into_iter().map(|r| r.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["main-aaaaaa".to_string(), "wt-bbbbbb".to_string()]);
    }

    #[test]
    fn data_dir_override_isolation_for_other_repos() {
        let (_r, data, store) = setup();
        store.create(&sample("Mine")).unwrap();
        let other = git_repo();
        let o = DiagramStore::open_at(other.path(), data.path());
        o.create(&sample("Theirs")).unwrap();
        let all = DiagramStore::all_repos(data.path());
        assert_eq!(all.len(), 2);
        let mut titles: Vec<_> = all.iter().flat_map(|s| s.rows().unwrap()).map(|r| r.title).collect();
        titles.sort();
        assert_eq!(titles, vec!["Mine".to_string(), "Theirs".to_string()]);
        let theirs = all.iter().find(|s| s.repo_key() == o.repo_key()).unwrap();
        assert_eq!(theirs.repo_path(), o.repo_path());
        assert_eq!(theirs.rows().unwrap()[0].repo, o.repo_name());
        assert_eq!(store.rows().unwrap().len(), 1, "a repo only sees its own diagrams");
    }
}
