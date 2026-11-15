//! On-disk diagram store at `<worktree>/.tether/diagrams/`.

use super::spec::{Diagram, DiagramKind};
use super::validate::{format_errors, normalize, validate_diagram, ValidationError};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Summary row returned by [`DiagramStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramInfo {
    /// Stored id.
    pub id: String,
    /// Diagram title.
    pub title: String,
    /// Diagram type.
    pub kind: DiagramKind,
    /// File modification time.
    pub mtime: SystemTime,
}

/// A store of validated diagram specs under one worktree.
#[derive(Debug, Clone)]
pub struct DiagramStore {
    root: PathBuf,
    is_git: bool,
}

/// Find the worktree root for `cwd` via `git rev-parse --show-toplevel`.
/// Returns `(root, is_git)`; falls back to `cwd` itself when not in a repository.
pub fn worktree_root(cwd: &Path) -> (PathBuf, bool) {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() {
                return (PathBuf::from(s), true);
            }
        }
    }
    (cwd.to_path_buf(), false)
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

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

impl DiagramStore {
    /// Open the store for the worktree containing `cwd`.
    pub fn open(cwd: &Path) -> Self {
        let (root, is_git) = worktree_root(cwd);
        Self { root, is_git }
    }

    /// Open a store rooted at an explicit worktree root.
    pub fn at_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let is_git = root.join(".git").exists();
        Self { root, is_git }
    }

    /// The worktree root.
    pub fn worktree_root(&self) -> &Path {
        &self.root
    }

    /// The directory holding the JSON files.
    pub fn dir(&self) -> PathBuf {
        self.root.join(".tether").join("diagrams")
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

    fn write_atomic(&self, path: &Path, d: &Diagram) -> Result<(), StoreError> {
        let dir = self.dir();
        std::fs::create_dir_all(&dir).map_err(io(format!("create {}", dir.display())))?;
        let mut text = serde_json::to_string_pretty(d).map_err(|e| StoreError::Corrupt {
            id: path.display().to_string(),
            message: e.to_string(),
        })?;
        text.push('\n');
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

    /// Validate and store a new diagram; returns its id (`<slug>-<6 hex>`).
    ///
    /// On the first create this also makes sure `.tether/` is in the worktree's `.gitignore`.
    pub fn create(&self, spec: &Diagram) -> Result<String, StoreError> {
        let d = Self::prepare(spec)?;
        self.ensure_gitignore()?;
        let slug = slugify(d.title());
        let mut id;
        loop {
            let mut h = DefaultHasher::new();
            d.title().hash(&mut h);
            std::process::id().hash(&mut h);
            next_counter().hash(&mut h);
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|t| t.as_nanos())
                .unwrap_or(0)
                .hash(&mut h);
            id = format!("{slug}-{:06x}", h.finish() & 0xff_ffff);
            if !self.path_for(&id)?.exists() {
                break;
            }
        }
        let path = self.path_for(&id)?;
        self.write_atomic(&path, &d)?;
        Ok(id)
    }

    /// Load a stored diagram.
    pub fn get(&self, id: &str) -> Result<Diagram, StoreError> {
        let path = self.path_for(id)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound(id.to_string()))
            }
            Err(e) => return Err(io(format!("read {}", path.display()))(e)),
        };
        let mut d: Diagram = serde_json::from_str(&text).map_err(|e| StoreError::Corrupt {
            id: id.to_string(),
            message: e.to_string(),
        })?;
        normalize(&mut d);
        Ok(d)
    }

    /// List stored diagrams, newest first. Unreadable files are skipped.
    pub fn list(&self) -> Result<Vec<DiagramInfo>, StoreError> {
        let dir = self.dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(format!("read {}", dir.display()))(e)),
        };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
                .filter(|id| valid_id(id))
            else {
                continue;
            };
            let Ok(d) = self.get(id) else { continue };
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            out.push(DiagramInfo {
                id: id.to_string(),
                title: d.title().to_string(),
                kind: d.kind(),
                mtime,
            });
        }
        out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Replace the spec stored under `id` (the id does not change).
    pub fn update(&self, id: &str, spec: &Diagram) -> Result<(), StoreError> {
        let path = self.path_for(id)?;
        if !path.exists() {
            return Err(StoreError::NotFound(id.to_string()));
        }
        let d = Self::prepare(spec)?;
        self.write_atomic(&path, &d)
    }

    /// Delete a stored diagram.
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        let path = self.path_for(id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StoreError::NotFound(id.to_string()))
            }
            Err(e) => Err(io(format!("delete {}", path.display()))(e)),
        }
    }

    /// Append `.tether/` to the worktree `.gitignore` if no equivalent line exists.
    /// Does nothing outside a git worktree.
    fn ensure_gitignore(&self) -> Result<(), StoreError> {
        if !self.is_git {
            return Ok(());
        }
        let path = self.root.join(".gitignore");
        let existing = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(io(format!("read {}", path.display()))(e)),
        };
        let covered = existing
            .lines()
            .map(str::trim)
            .any(|l| matches!(l, ".tether" | ".tether/" | "/.tether" | "/.tether/"));
        if covered {
            return Ok(());
        }
        let mut add = String::new();
        if !existing.is_empty() && !existing.ends_with('\n') {
            add.push('\n');
        }
        add.push_str(".tether/\n");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(io(format!("open {}", path.display())))?;
        f.write_all(add.as_bytes())
            .map_err(io(format!("write {}", path.display())))
    }
}

fn next_counter() -> u64 {
    static C: AtomicU64 = AtomicU64::new(0);
    C.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::validate::validate;

    fn git_repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status()
            .unwrap()
            .success();
        assert!(ok);
        d
    }

    fn sample(title: &str) -> Diagram {
        validate(&format!(
            r#"{{"type":"flowchart","title":"{title}","nodes":[{{"id":"a","label":"A"}}],"edges":[]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slugify("Session refresh!"), "session-refresh");
        assert_eq!(slugify("  --  "), "diagram");
        assert_eq!(slugify("Ünï cødé"), "n-c-d");
        assert!(slugify(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn round_trip_in_git_repo_from_subdir() {
        let repo = git_repo();
        let sub = repo.path().join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        let store = DiagramStore::open(&sub);
        assert_eq!(
            store.worktree_root().canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );
        assert!(store.list().unwrap().is_empty());

        let d = sample("Session refresh");
        let id = store.create(&d).unwrap();
        assert!(id.starts_with("session-refresh-"), "{id}");
        assert_eq!(id.len(), "session-refresh-".len() + 6);
        assert_eq!(store.get(&id).unwrap(), d);

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].title, "Session refresh");
        assert_eq!(list[0].kind, DiagramKind::Flowchart);

        let d2 = sample("Renamed");
        store.update(&id, &d2).unwrap();
        assert_eq!(store.get(&id).unwrap().title(), "Renamed");

        store.delete(&id).unwrap();
        assert!(matches!(store.get(&id), Err(StoreError::NotFound(_))));
        assert!(matches!(store.delete(&id), Err(StoreError::NotFound(_))));
        assert!(matches!(
            store.update(&id, &d2),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn same_title_gets_unique_ids() {
        let repo = git_repo();
        let store = DiagramStore::open(repo.path());
        let d = sample("Same");
        let ids: std::collections::HashSet<String> =
            (0..20).map(|_| store.create(&d).unwrap()).collect();
        assert_eq!(ids.len(), 20);
        assert_eq!(store.list().unwrap().len(), 20);
    }

    #[test]
    fn gitignore_created_appended_and_not_duplicated() {
        let repo = git_repo();
        let gi = repo.path().join(".gitignore");
        let store = DiagramStore::open(repo.path());
        assert!(!gi.exists());
        store.create(&sample("One")).unwrap();
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), ".tether/\n");
        store.create(&sample("Two")).unwrap();
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), ".tether/\n");

        let repo2 = git_repo();
        let gi2 = repo2.path().join(".gitignore");
        std::fs::write(&gi2, "target").unwrap();
        DiagramStore::open(repo2.path())
            .create(&sample("X"))
            .unwrap();
        assert_eq!(std::fs::read_to_string(&gi2).unwrap(), "target\n.tether/\n");

        let repo3 = git_repo();
        let gi3 = repo3.path().join(".gitignore");
        std::fs::write(&gi3, "/.tether\n").unwrap();
        DiagramStore::open(repo3.path())
            .create(&sample("X"))
            .unwrap();
        assert_eq!(std::fs::read_to_string(&gi3).unwrap(), "/.tether\n");
    }

    #[test]
    fn git_actually_ignores_the_store() {
        let repo = git_repo();
        DiagramStore::open(repo.path())
            .create(&sample("X"))
            .unwrap();
        let out = Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(!s.contains(".tether"), "{s}");
    }

    #[test]
    fn non_git_dir_falls_back_to_cwd_without_gitignore() {
        let d = tempfile::tempdir().unwrap();
        // tempdirs live outside any repo on CI; guard for odd setups.
        let (_, is_git) = worktree_root(d.path());
        let store = DiagramStore::open(d.path());
        store.create(&sample("X")).unwrap();
        if !is_git {
            assert_eq!(store.worktree_root(), d.path());
            assert!(!d.path().join(".gitignore").exists());
        }
    }

    #[test]
    fn invalid_spec_rejected_and_bad_ids_refused() {
        let repo = git_repo();
        let store = DiagramStore::open(repo.path());
        let mut d = sample("X");
        if let Diagram::Flowchart(f) = &mut d {
            f.edges.push(crate::diagram::Edge {
                from: "a".into(),
                to: "ghost".into(),
                label: None,
                style: None,
            });
        }
        assert!(matches!(store.create(&d), Err(StoreError::Invalid(_))));
        assert!(store.list().unwrap().is_empty());
        for bad in ["../x", "a/b", "", "UP", "-x"] {
            assert!(matches!(store.get(bad), Err(StoreError::BadId(_))), "{bad}");
        }
    }

    #[test]
    fn sequence_ids_persist_after_create() {
        let repo = git_repo();
        let store = DiagramStore::open(repo.path());
        let d = validate(
            r#"{"type":"sequence","title":"S","participants":[{"id":"a","label":"A"},{"id":"b","label":"B"}],
            "items":[{"type":"message","from":"a","to":"b","label":"hi"}]}"#,
        )
        .unwrap();
        let id = store.create(&d).unwrap();
        let text = std::fs::read_to_string(store.dir().join(format!("{id}.json"))).unwrap();
        assert!(text.contains("\"m1\""));
    }
}
