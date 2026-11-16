//! Anchor re-resolution (PRD F3.4): re-find a code link after the code shifts.

use super::spec::CodeRef;
use std::path::{Component, Path};

/// Default search window around the stored line, in lines each way.
pub const DEFAULT_WINDOW: usize = 200;

/// Result of re-resolving a [`CodeRef`] against the files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// The link is still valid at this (1-based) line.
    Exact(u32),
    /// The anchor was found at a different (1-based) line.
    Moved(u32),
    /// The anchor could not be found (or the line no longer exists). Do not jump.
    Stale,
    /// The file is missing, unreadable, or its path escapes the worktree.
    MissingFile,
}

impl Resolved {
    /// The line to jump to, if the link is usable.
    pub fn line(self) -> Option<u32> {
        match self {
            Resolved::Exact(l) | Resolved::Moved(l) => Some(l),
            _ => None,
        }
    }
}

/// Resolve with the default window (+-200 lines, then the whole file).
pub fn resolve(code: &CodeRef, worktree_root: &Path) -> Resolved {
    match resolve_with_window(code, worktree_root, Some(DEFAULT_WINDOW)) {
        Resolved::Stale => resolve_with_window(code, worktree_root, None),
        r => r,
    }
}

/// Resolve searching only `window` lines each way from the stored line (`None` = whole file).
///
/// Nearest match wins; on a tie the earlier line wins.
pub fn resolve_with_window(
    code: &CodeRef,
    worktree_root: &Path,
    window: Option<usize>,
) -> Resolved {
    let rel = Path::new(&code.path);
    if code.path.is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Resolved::MissingFile;
    }
    let Ok(bytes) = std::fs::read(worktree_root.join(rel)) else {
        return Resolved::MissingFile;
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    resolve_in_lines(code, &lines, window)
}

fn resolve_in_lines(code: &CodeRef, lines: &[&str], window: Option<usize>) -> Resolved {
    let stored = code.line as usize; // 1-based
    let anchor = code
        .anchor
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty());
    let Some(anchor) = anchor else {
        return if stored >= 1 && stored <= lines.len() {
            Resolved::Exact(code.line)
        } else {
            Resolved::Stale
        };
    };
    let hit = |line1: usize| line1 >= 1 && lines.get(line1 - 1).is_some_and(|l| l.contains(anchor));
    if hit(stored) {
        return Resolved::Exact(code.line);
    }
    // O(lines): scan the real line numbers only, never the distance range (the stored line may
    // be arbitrarily far past EOF). Nearest wins, the earlier line on a tie.
    let window = window.unwrap_or(usize::MAX);
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains(anchor))
        .map(|(i, _)| i + 1)
        .filter(|&l| l.abs_diff(stored) <= window)
        .min_by_key(|&l| (l.abs_diff(stored), l))
        .map_or(Resolved::Stale, |l| Resolved::Moved(l as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cr(line: u32, anchor: Option<&str>) -> CodeRef {
        CodeRef {
            path: "f.txt".into(),
            line,
            end_line: None,
            anchor: anchor.map(Into::into),
            diff: None,
        }
    }

    fn file(dir: &Path, content: &str) {
        std::fs::write(dir.join("f.txt"), content).unwrap();
    }

    fn numbered(n: usize, special: &[(usize, &str)]) -> String {
        (1..=n)
            .map(|i| {
                special
                    .iter()
                    .find(|(l, _)| *l == i)
                    .map(|(_, t)| t.to_string())
                    .unwrap_or_else(|| format!("filler {i}"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn exact_when_anchor_on_stored_line() {
        let d = tempfile::tempdir().unwrap();
        file(
            d.path(),
            &numbered(20, &[(7, "  if token.expiresAt < now {")]),
        );
        assert_eq!(
            resolve(&cr(7, Some("token.expiresAt <")), d.path()),
            Resolved::Exact(7)
        );
    }

    #[test]
    fn moved_down_and_up() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), &numbered(30, &[(12, "needle()")]));
        assert_eq!(
            resolve(&cr(7, Some("needle()")), d.path()),
            Resolved::Moved(12)
        );
        assert_eq!(
            resolve(&cr(25, Some("needle()")), d.path()),
            Resolved::Moved(12)
        );
    }

    #[test]
    fn duplicate_anchors_nearest_wins() {
        let d = tempfile::tempdir().unwrap();
        file(
            d.path(),
            &numbered(60, &[(5, "dup"), (30, "dup"), (50, "dup")]),
        );
        assert_eq!(resolve(&cr(28, Some("dup")), d.path()), Resolved::Moved(30));
        // tie at distance 10 prefers the earlier line
        assert_eq!(resolve(&cr(40, Some("dup")), d.path()), Resolved::Moved(30));
    }

    #[test]
    fn beyond_window_falls_back_to_whole_file() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), &numbered(1000, &[(900, "far away")]));
        let c = cr(10, Some("far away"));
        assert_eq!(
            resolve_with_window(&c, d.path(), Some(200)),
            Resolved::Stale
        );
        assert_eq!(resolve(&c, d.path()), Resolved::Moved(900));
    }

    #[test]
    fn missing_anchor_is_stale() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), &numbered(10, &[]));
        assert_eq!(resolve(&cr(3, Some("nope")), d.path()), Resolved::Stale);
    }

    #[test]
    fn stored_line_past_eof_still_searches() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), &numbered(10, &[(4, "target")]));
        assert_eq!(
            resolve(&cr(99, Some("target")), d.path()),
            Resolved::Moved(4)
        );
    }

    #[test]
    fn huge_stored_line_is_bounded_by_the_file() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), &numbered(50, &[]));
        let t = std::time::Instant::now();
        assert_eq!(
            resolve(&cr(u32::MAX, Some("missing anchor")), d.path()),
            Resolved::Stale
        );
        assert!(t.elapsed() < std::time::Duration::from_millis(100), "{:?}", t.elapsed());
        file(d.path(), &numbered(50, &[(4, "target")]));
        assert_eq!(
            resolve(&cr(u32::MAX, Some("target")), d.path()),
            Resolved::Moved(4)
        );
    }

    #[test]
    fn no_anchor_checks_line_exists() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), "a\nb\nc\n");
        assert_eq!(resolve(&cr(3, None), d.path()), Resolved::Exact(3));
        assert_eq!(resolve(&cr(4, None), d.path()), Resolved::Stale);
    }

    #[test]
    fn missing_file_and_escaping_paths() {
        let d = tempfile::tempdir().unwrap();
        let mut c = cr(1, None);
        c.path = "nope.rs".into();
        assert_eq!(resolve(&c, d.path()), Resolved::MissingFile);
        c.path = "../etc/passwd".into();
        assert_eq!(resolve(&c, d.path()), Resolved::MissingFile);
        c.path = "/etc/passwd".into();
        assert_eq!(resolve(&c, d.path()), Resolved::MissingFile);
    }

    #[test]
    fn line_accessor() {
        assert_eq!(Resolved::Moved(4).line(), Some(4));
        assert_eq!(Resolved::Stale.line(), None);
    }
}
