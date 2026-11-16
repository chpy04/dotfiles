//! Comment payload formatting (F2.4) and delivery (`agent.prompt`, F2.3/F2.6).
//!
//! Slash-command policy (F2.6, decided): tether sends plain text only.
//! * A comment body whose first non-blank character is `/` is **rejected** (exit code 2) rather
//!   than rewritten, so the user's words are never silently altered; the message says how to
//!   rephrase. This body rule applies only when the template puts `{text}` first
//!   (`clean_comment_text_for`); with the default template the submitted message starts with
//!   `[comment]`, so a leading `/` in the body is harmless and allowed.
//! * The final rendered payload must also not start with `/` or `!` (Claude Code's slash-command
//!   and bash-mode triggers), guarding against custom templates.
//! * C0/C1 control characters (notably ESC) are stripped from the body and code lines, so text
//!   can never smuggle a bracketed-paste terminator `ESC[201~` and have the rest typed as keys.
//!   `\n` and `\t` are kept; `\r\n` and lone `\r` become `\n`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use crate::config::CommentConfig;
use crate::error::{Result, TetherError};
use crate::herdr::Client;
use crate::workspace::AgentRow;

// ---------------------------------------------------------------------------------------------
// Code references
// ---------------------------------------------------------------------------------------------

/// `path:line[-endLine]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeRef {
    pub path: String,
    pub line: u32,
    /// Only set when greater than `line`.
    pub end_line: Option<u32>,
}

impl CodeRef {
    /// `"142"` or `"142-148"`.
    pub fn range_string(&self) -> String {
        match self.end_line {
            Some(e) => format!("{}-{}", self.line, e),
            None => self.line.to_string(),
        }
    }
}

impl FromStr for CodeRef {
    type Err = TetherError;

    fn from_str(s: &str) -> Result<CodeRef> {
        let bad = || TetherError::Input(format!("invalid --ref `{s}`: expected path:line or path:line-endLine"));
        let (path, tail) = s.rsplit_once(':').ok_or_else(bad)?;
        if path.is_empty() {
            return Err(bad());
        }
        let (a, b) = match tail.split_once('-') {
            Some((a, b)) => (a, Some(b)),
            None => (tail, None),
        };
        let digits = |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit());
        if !digits(a) || b.is_some_and(|b| !digits(b)) {
            return Err(bad());
        }
        let line: u32 = a.parse().map_err(|_| bad())?;
        if line == 0 {
            return Err(TetherError::Input(format!("invalid --ref `{s}`: line numbers start at 1")));
        }
        let end_line = match b {
            Some(b) => {
                let e: u32 = b.parse().map_err(|_| bad())?;
                if e < line {
                    return Err(TetherError::Input(format!("invalid --ref `{s}`: end line {e} is before start line {line}")));
                }
                (e > line).then_some(e)
            }
            None => None,
        };
        Ok(CodeRef { path: path.to_string(), line, end_line })
    }
}

// ---------------------------------------------------------------------------------------------
// Text hygiene
// ---------------------------------------------------------------------------------------------

/// Strip control characters (keep `\n`, `\t`), normalise line endings.
pub fn sanitize(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control()))
        .collect()
}

/// Validate and clean a comment body: non-empty, no leading `/` (see module docs). The strict
/// form, for callers that do not know the template; prefer `clean_comment_text_for`.
pub fn clean_comment_text(text: &str) -> Result<String> {
    clean_body(text, true)
}

/// True when the rendered message would begin with the comment body itself.
pub fn template_starts_with_text(template: &str) -> bool {
    template.trim_start().starts_with("{text}")
}

/// Like `clean_comment_text`, but the leading-`/` body check is kept only when `template` puts
/// `{text}` first. (The final rendered payload is always checked by `check_payload`.)
pub fn clean_comment_text_for(template: &str, text: &str) -> Result<String> {
    clean_body(text, template_starts_with_text(template))
}

fn clean_body(text: &str, reject_leading_slash: bool) -> Result<String> {
    let t = sanitize(text);
    let t = t.trim_end().to_string();
    let t = t.trim_start_matches('\n').to_string();
    if t.trim().is_empty() {
        return Err(TetherError::Input("empty comment text".into()));
    }
    if reject_leading_slash && t.trim_start().starts_with('/') {
        return Err(TetherError::Input(
            "comment text must not start with `/` (tether sends plain text only, never slash commands); rephrase, e.g. start with a word".into(),
        ));
    }
    Ok(t)
}

/// Final guard on the rendered message.
pub fn check_payload(payload: &str) -> Result<()> {
    match payload.trim_start().chars().next() {
        Some('/') | Some('!') => Err(TetherError::Input(
            "the rendered comment starts with `/` or `!`, which could trigger a slash command or shell mode; fix [comment] template (it should start with a literal like `[comment]`)".into(),
        )),
        Some(_) => Ok(()),
        None => Err(TetherError::Input("empty comment payload".into())),
    }
}

// ---------------------------------------------------------------------------------------------
// Payload formatting
// ---------------------------------------------------------------------------------------------

/// Values substituted into the template.
#[derive(Debug, Clone, Default)]
pub struct PayloadParts {
    pub path: String,
    pub line: u32,
    pub range: String,
    pub worktree: String,
    /// Already formatted with `format_code` ("" when unavailable).
    pub code: String,
    pub text: String,
}

/// Longest code line (in characters) put into a comment; longer lines are cut and end with `…`.
/// Configurable as `[comment] max_code_line_width`; this is the default.
pub use crate::config::DEFAULT_MAX_CODE_LINE_WIDTH as MAX_CODE_LINE_WIDTH;

fn clip_line(l: &str, max_width: usize) -> String {
    let l = sanitize(l);
    let l = l.trim_end();
    match l.char_indices().nth(max_width) {
        Some((i, _)) => format!("{}…", &l[..i]),
        None => l.to_string(),
    }
}

/// Format code lines as `  142 │ text` (numbers right-aligned to the widest). Lines longer than
/// `max_width` characters are cut and end with `…`. If `hidden > 0` a final `  … (N more lines)`
/// line is added.
pub fn format_code(lines: &[(u32, String)], hidden: usize, max_width: usize) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let width = lines.iter().map(|(n, _)| n.to_string().len()).max().unwrap_or(1);
    let mut out: Vec<String> = lines
        .iter()
        .map(|(n, l)| format!("  {n:>width$} │ {}", clip_line(l, max_width)))
        .collect();
    if hidden > 0 {
        out.push(format!("  … ({hidden} more line{})", if hidden == 1 { "" } else { "s" }));
    }
    out.join("\n")
}

/// Render a template. Single pass: substituted values are never re-scanned, unknown `{...}`
/// stays literal. A line consisting only of `{code}` is dropped when there is no code.
pub fn render_template(template: &str, p: &PayloadParts) -> String {
    let template: String = if p.code.is_empty() {
        template.split('\n').filter(|l| l.trim() != "{code}").collect::<Vec<_>>().join("\n")
    } else {
        template.to_string()
    };
    let mut out = String::new();
    let mut rest = template.as_str();
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(j) = after.find('}') else {
            out.push_str(after);
            rest = "";
            break;
        };
        let name = &after[1..j];
        let line;
        let val: Option<&str> = match name {
            "path" => Some(&p.path),
            "line" => {
                line = p.line.to_string();
                Some(&line)
            }
            "range" => Some(&p.range),
            "worktree" => Some(&p.worktree),
            "code" => Some(&p.code),
            "text" => Some(&p.text),
            _ => None,
        };
        match val {
            Some(v) => {
                out.push_str(v);
                rest = &after[j + 1..];
            }
            None => {
                out.push('{');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------------------------
// Building a comment from a file on disk
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Prepared {
    pub payload: String,
    pub worktree: String,
    /// Path relative to the worktree root.
    pub rel_path: String,
}

fn canonical_or_self(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    // nonexistent file: canonicalize the nearest existing ancestor
    let mut tail = Vec::new();
    let mut cur = p;
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_os_string());
        }
        if let Ok(c) = parent.canonicalize() {
            let mut out = c;
            for n in tail.iter().rev() {
                out.push(n);
            }
            return out;
        }
        cur = parent;
    }
    p.to_path_buf()
}

/// `git rev-parse --show-toplevel` run in the file's directory (or nearest existing ancestor).
pub fn git_toplevel(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    while !dir.is_dir() {
        dir = dir.parent()?;
    }
    let out = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "--show-toplevel"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

fn basename(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// Read lines `[start, end]` (1-based, clamped to the file). Returns the shown lines plus how
/// many more were hidden by `max_lines`.
pub fn read_code_lines(file: &Path, start: u32, end: u32, max_lines: usize) -> (Vec<(u32, String)>, usize) {
    let Ok(bytes) = std::fs::read(file) else { return (vec![], 0) };
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    if start as usize > all.len() {
        return (vec![], 0);
    }
    let end = (end as usize).min(all.len());
    let total = end - start as usize + 1;
    let shown = total.min(max_lines);
    let lines = (0..shown).map(|i| (start + i as u32, all[start as usize - 1 + i].to_string())).collect();
    (lines, total - shown)
}

/// Build the message for a code-ref comment. `cwd` resolves relative paths and is the fallback
/// "worktree" when the file is not in a git repository.
pub fn prepare_comment(cfg: &CommentConfig, r: &CodeRef, text: &str, cwd: &Path) -> Result<Prepared> {
    let text = clean_comment_text_for(&cfg.template, text)?;
    let given = Path::new(&r.path);
    let abs = canonical_or_self(&if given.is_absolute() { given.to_path_buf() } else { cwd.join(given) });
    let (worktree, rel_path) = match git_toplevel(&abs) {
        Some(top) => {
            let top_c = canonical_or_self(&top);
            let rel = abs.strip_prefix(&top_c).map(|p| p.to_path_buf()).unwrap_or_else(|_| abs.clone());
            (basename(&top_c), rel.to_string_lossy().into_owned())
        }
        None => {
            let cwd_c = canonical_or_self(cwd);
            let rel = abs.strip_prefix(&cwd_c).map(|p| p.to_path_buf()).unwrap_or_else(|_| abs.clone());
            (basename(&cwd_c), rel.to_string_lossy().into_owned())
        }
    };
    let end = r.end_line.unwrap_or(r.line);
    let (lines, hidden) = read_code_lines(&abs, r.line, end, cfg.max_code_lines);
    let parts = PayloadParts {
        path: sanitize(&rel_path),
        line: r.line,
        range: r.range_string(),
        worktree: sanitize(&worktree),
        code: format_code(&lines, hidden, cfg.max_code_line_width),
        text,
    };
    let payload = render_template(&cfg.template, &parts);
    check_payload(&payload)?;
    Ok(Prepared { payload, worktree: parts.worktree, rel_path: parts.path })
}

// ---------------------------------------------------------------------------------------------
// Comments on diagram elements (F3.3)
// ---------------------------------------------------------------------------------------------

/// The diagram element a comment is about (F3.3).
#[derive(Debug, Clone)]
pub struct DiagramTarget<'a> {
    /// Diagram title.
    pub diagram_title: &'a str,
    /// `node`, `participant`, `message` or `note`.
    pub kind: &'a str,
    /// The element's label (node label, message label, note text).
    pub label: &'a str,
    /// Its code link, if any. Re-resolved against the files so the line shown is the live one.
    pub code: Option<&'a crate::diagram::CodeRef>,
}

fn one_line(s: &str) -> String {
    sanitize(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build the message for a comment on a diagram element:
///
/// ```text
/// [comment] diagram "Session refresh" › node "Token expired?"  src/auth/session.ts:142  (worktree: feat-login)
///   142 │ if (token.expiresAt < Date.now()) {
/// <text>
/// ```
///
/// The code ref is re-resolved with its anchor first: a moved link shows the new line; a stale or
/// missing link keeps the stored `path:line`, gets a `[stale link]` / `[file missing]` marker and
/// no code lines (we never show the wrong line). Elements without code omit the ref and the code
/// lines. The `[comment] template` does not apply (this format is fixed). `worktree_root` is the
/// worktree the diagram store lives in.
pub fn prepare_diagram_comment(cfg: &CommentConfig, t: &DiagramTarget<'_>, text: &str, worktree_root: &Path) -> Result<Prepared> {
    use crate::diagram::Resolved;
    let text = clean_comment_text(text)?;
    let root = canonical_or_self(worktree_root);
    let worktree = sanitize(&basename(&root));
    let mut header = format!("[comment] diagram \"{}\" › {} \"{}\"", one_line(t.diagram_title), t.kind, one_line(t.label));
    let mut code_block = String::new();
    let mut rel_path = String::new();
    if let Some(c) = t.code {
        rel_path = sanitize(&c.path);
        let resolved = crate::diagram::resolve(c, &root);
        let (line, usable, marker) = match resolved {
            Resolved::Exact(l) => (l, true, None),
            Resolved::Moved(l) => (l, true, None),
            Resolved::Stale => (c.line, false, Some("stale link")),
            Resolved::MissingFile => (c.line, false, Some("file missing")),
        };
        let end = c.end_line.map(|e| ((e as i64) + (line as i64 - c.line as i64)).max(line as i64) as u32);
        let range = match end {
            Some(e) if e > line => format!("{line}-{e}"),
            _ => line.to_string(),
        };
        header.push_str(&format!("  {rel_path}:{range}"));
        if let Some(m) = marker {
            header.push_str(&format!("  [{m}]"));
        }
        if usable {
            let (lines, hidden) = read_code_lines(&root.join(&c.path), line, end.unwrap_or(line), cfg.max_code_lines);
            code_block = format_code(&lines, hidden, cfg.max_code_line_width);
        }
    }
    header.push_str(&format!("  (worktree: {worktree})"));
    let payload = if code_block.is_empty() { format!("{header}\n{text}") } else { format!("{header}\n{code_block}\n{text}") };
    check_payload(&payload)?;
    Ok(Prepared { payload, worktree, rel_path })
}

#[cfg(test)]
mod diagram_comment_tests {
    use super::*;
    use crate::diagram::CodeRef as SpecRef;

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("feat-login");
        std::fs::create_dir_all(root.join("src/auth")).unwrap();
        let body = (1..=200)
            .map(|n| if n == 142 { "if (token.expiresAt < Date.now()) {".to_string() } else { format!("line {n}") })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("src/auth/session.ts"), body).unwrap();
        (d, root)
    }

    fn sref(line: u32, anchor: Option<&str>) -> SpecRef {
        SpecRef { path: "src/auth/session.ts".into(), line, end_line: None, anchor: anchor.map(Into::into) }
    }

    fn target<'a>(code: Option<&'a SpecRef>) -> DiagramTarget<'a> {
        DiagramTarget { diagram_title: "Session refresh", kind: "node", label: "Token expired?", code }
    }

    #[test]
    fn matches_the_prd_example() {
        let (_d, root) = repo();
        let c = sref(142, Some("token.expiresAt <"));
        let p = prepare_diagram_comment(&CommentConfig::default(), &target(Some(&c)), "Why client clock?", &root).unwrap();
        assert_eq!(
            p.payload,
            "[comment] diagram \"Session refresh\" › node \"Token expired?\"  src/auth/session.ts:142  (worktree: feat-login)\n  142 │ if (token.expiresAt < Date.now()) {\nWhy client clock?"
        );
        assert_eq!(p.worktree, "feat-login");
        assert_eq!(p.rel_path, "src/auth/session.ts");
    }

    #[test]
    fn moved_link_shows_the_live_line_and_stale_link_shows_no_code() {
        let (_d, root) = repo();
        let c = sref(150, Some("token.expiresAt <"));
        let p = prepare_diagram_comment(&CommentConfig::default(), &target(Some(&c)), "q", &root).unwrap();
        assert!(p.payload.contains("src/auth/session.ts:142  (worktree"), "{}", p.payload);
        assert!(p.payload.contains("\n  142 │ if (token.expiresAt"), "{}", p.payload);

        let c = sref(150, Some("no such anchor anywhere"));
        let p = prepare_diagram_comment(&CommentConfig::default(), &target(Some(&c)), "q", &root).unwrap();
        assert_eq!(
            p.payload,
            "[comment] diagram \"Session refresh\" › node \"Token expired?\"  src/auth/session.ts:150  [stale link]  (worktree: feat-login)\nq"
        );
        let c = SpecRef { path: "src/gone.ts".into(), line: 3, end_line: None, anchor: None };
        let p = prepare_diagram_comment(&CommentConfig::default(), &target(Some(&c)), "q", &root).unwrap();
        assert!(p.payload.contains("[file missing]"), "{}", p.payload);
    }

    #[test]
    fn no_code_ranges_and_hygiene() {
        let (_d, root) = repo();
        let p = prepare_diagram_comment(&CommentConfig::default(), &target(None), "plain", &root).unwrap();
        assert_eq!(p.payload, "[comment] diagram \"Session refresh\" › node \"Token expired?\"  (worktree: feat-login)\nplain");
        assert_eq!(p.rel_path, "");

        let mut c = sref(140, None);
        c.end_line = Some(142);
        let p = prepare_diagram_comment(&CommentConfig::default(), &target(Some(&c)), "r", &root).unwrap();
        assert!(p.payload.contains("src/auth/session.ts:140-142"), "{}", p.payload);
        assert!(p.payload.contains("  140 │ line 140\n  141 │ line 141\n  142 │ if (token"), "{}", p.payload);

        // multi-line labels collapse, control chars vanish, slash policy still applies
        let t = DiagramTarget { diagram_title: "A\nB\x1b[0m", kind: "message", label: "x\n y", code: None };
        let p = prepare_diagram_comment(&CommentConfig::default(), &t, "ok", &root).unwrap();
        assert!(p.payload.starts_with("[comment] diagram \"A B[0m\" › message \"x y\""), "{}", p.payload);
        assert!(prepare_diagram_comment(&CommentConfig::default(), &target(None), "/clear", &root).is_err());
        assert!(prepare_diagram_comment(&CommentConfig::default(), &target(None), "  ", &root).is_err());
    }
}

// ---------------------------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Delivery {
    pub target: AgentRow,
    /// Agent status Herdr reported at submission (before the turn started).
    pub state_before: String,
    pub warnings: Vec<String>,
    pub bytes: usize,
}

/// Deliver with `agent.prompt` (no `wait`): Herdr writes the bracketed paste, pauses ~300 ms and
/// submits. `idle`/`done`/`working` (Claude queues input while working) are normal; `blocked`
/// still goes to Herdr (which is the authority and refuses with `agent_blocked`, nothing sent)
/// but is flagged. We deliberately do NOT fall back to raw `pane.send_input`: typing into an
/// approval dialog could confirm it.
pub fn deliver(client: &Client, target: &AgentRow, payload: &str) -> Result<Delivery> {
    check_payload(payload)?;
    let mut warnings = Vec::new();
    match target.state.as_str() {
        "idle" | "done" | "working" => {}
        "blocked" => warnings.push(format!(
            "agent {} (tab {}) is blocked on an approval prompt; delivery will probably be refused",
            target.pane_id, target.tab_index
        )),
        other => warnings.push(format!("agent {} reports state `{other}`", target.pane_id)),
    }
    let agent = client.agent_prompt(&target.pane_id, payload, None)?;
    Ok(Delivery { target: target.clone(), state_before: agent.agent_status, warnings, bytes: payload.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CommentConfig, DEFAULT_TEMPLATE};
    use crate::testing::FakeHerdr;
    use serde_json::json;

    fn cfg() -> CommentConfig {
        CommentConfig::default()
    }

    // ---- ref parsing --------------------------------------------------------------------

    #[test]
    fn ref_parsing() {
        let r: CodeRef = "src/auth/session.ts:142".parse().unwrap();
        assert_eq!(r, CodeRef { path: "src/auth/session.ts".into(), line: 142, end_line: None });
        assert_eq!(r.range_string(), "142");
        let r: CodeRef = "a/b.rs:10-20".parse().unwrap();
        assert_eq!((r.line, r.end_line), (10, Some(20)));
        assert_eq!(r.range_string(), "10-20");
        // equal endpoints collapse to a single line
        let r: CodeRef = "a.rs:7-7".parse().unwrap();
        assert_eq!(r.end_line, None);
        // colons in the path, spaces
        let r: CodeRef = "/tmp/we:ird dir/x.rs:3".parse().unwrap();
        assert_eq!(r.path, "/tmp/we:ird dir/x.rs");
        // path containing a dash is fine
        let r: CodeRef = "my-file.rs:5".parse().unwrap();
        assert_eq!(r.path, "my-file.rs");
    }

    #[test]
    fn ref_parsing_errors() {
        for bad in ["", "foo", "foo:", ":12", "foo:abc", "foo:1-", "foo:-3", "foo:0", "foo:5-2", "foo:1-2-3", "foo:1:2x"] {
            let e = bad.parse::<CodeRef>().unwrap_err();
            assert_eq!(e.exit_code(), crate::error::exit::USAGE, "{bad}");
        }
    }

    // ---- formatting ---------------------------------------------------------------------

    fn parts(code: &str) -> PayloadParts {
        PayloadParts {
            path: "src/auth/session.ts".into(),
            line: 142,
            range: "142".into(),
            worktree: "feat-login".into(),
            code: code.into(),
            text: "Why client clock instead of server time here?".into(),
        }
    }

    #[test]
    fn single_line_matches_prd_example() {
        let code = format_code(&[(142, "if (token.expiresAt < Date.now()) {".into())], 0, MAX_CODE_LINE_WIDTH);
        let out = render_template(DEFAULT_TEMPLATE, &parts(&code));
        assert_eq!(
            out,
            "[comment] src/auth/session.ts:142  (worktree: feat-login)\n  142 │ if (token.expiresAt < Date.now()) {\nWhy client clock instead of server time here?"
        );
    }

    #[test]
    fn range_numbers_are_aligned_and_truncated() {
        let lines: Vec<(u32, String)> = (98..=101).map(|n| (n, format!("line {n}"))).collect();
        assert_eq!(format_code(&lines, 0, 200), "   98 │ line 98\n   99 │ line 99\n  100 │ line 100\n  101 │ line 101");
        assert!(format_code(&lines, 3, 200).ends_with("\n  … (3 more lines)"));
        assert!(format_code(&lines, 1, 200).ends_with("(1 more line)"));
        assert_eq!(format_code(&[], 0, 200), "");
    }

    #[test]
    fn template_override_and_unknown_placeholders() {
        let mut p = parts("  142 │ x");
        p.range = "142-148".into();
        let out = render_template("{text} @ {path}#{line} ({range}) [{worktree}] {nope} {", &p);
        assert_eq!(out, "Why client clock instead of server time here? @ src/auth/session.ts#142 (142-148) [feat-login] {nope} {");
        // values are not re-scanned
        let mut p = parts("");
        p.text = "{path} {code}".into();
        assert_eq!(render_template("{text}", &p), "{path} {code}");
    }

    #[test]
    fn code_line_dropped_when_absent() {
        let out = render_template(DEFAULT_TEMPLATE, &parts(""));
        assert_eq!(
            out,
            "[comment] src/auth/session.ts:142  (worktree: feat-login)\nWhy client clock instead of server time here?"
        );
    }

    #[test]
    fn unicode_survives() {
        let code = format_code(&[(7, "let s = \"héllo → 世界 🦀\";".into())], 0, 200);
        let mut p = parts(&code);
        p.text = "Pourquoi « ça » ? 日本語 ✓".into();
        let out = render_template(DEFAULT_TEMPLATE, &p);
        assert!(out.contains("  7 │ let s = \"héllo → 世界 🦀\";"));
        assert!(out.ends_with("Pourquoi « ça » ? 日本語 ✓"));
    }

    // ---- hygiene / slash policy ---------------------------------------------------------

    #[test]
    fn slash_prefixed_body_is_rejected() {
        for t in ["/clear", "  /compact now", "\n/exit", "/etc/passwd perms?"] {
            let e = clean_comment_text(t).unwrap_err();
            assert_eq!(e.exit_code(), crate::error::exit::USAGE, "{t}");
        }
        // a slash later in the text is fine
        assert_eq!(clean_comment_text("see /etc/hosts").unwrap(), "see /etc/hosts");
        assert_eq!(clean_comment_text("a\n/b").unwrap(), "a\n/b");
    }

    #[test]
    fn leading_slash_body_is_allowed_unless_the_template_puts_text_first() {
        // default template starts with `[comment]`: a body beginning with `/` is harmless
        assert_eq!(clean_comment_text_for(DEFAULT_TEMPLATE, "/etc/hosts is wrong").unwrap(), "/etc/hosts is wrong");
        assert!(template_starts_with_text("{text}\n{path}") && template_starts_with_text("  {text}"));
        assert!(!template_starts_with_text(DEFAULT_TEMPLATE));
        for t in ["{text}", "  {text} @ {path}"] {
            let e = clean_comment_text_for(t, "/clear").unwrap_err();
            assert_eq!(e.exit_code(), crate::error::exit::USAGE, "{t}");
        }
        // end to end: prepare_comment with the default template sends the slash body, with `{text}` first it refuses
        let dir = tempfile::tempdir().unwrap();
        let r: CodeRef = "x.rs:1".parse().unwrap();
        let p = prepare_comment(&cfg(), &r, "/clear please explain", dir.path()).unwrap();
        assert!(p.payload.starts_with("[comment] ") && p.payload.ends_with("/clear please explain"), "{}", p.payload);
        let mut c = cfg();
        c.template = "{text}".into();
        assert!(prepare_comment(&c, &r, "/clear", dir.path()).is_err());
        // empty text is still rejected either way
        assert!(clean_comment_text_for(DEFAULT_TEMPLATE, " \n").is_err());
    }

    #[test]
    fn long_code_lines_are_cut_at_200_chars() {
        let long = "é".repeat(500);
        let out = format_code(&[(1, long.clone()), (2, "short".into()), (3, "x".repeat(200))], 0, MAX_CODE_LINE_WIDTH);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0].chars().filter(|&c| c == 'é').count(), MAX_CODE_LINE_WIDTH);
        assert!(lines[0].ends_with('…'));
        assert_eq!(lines[1], "  2 │ short");
        assert!(!lines[2].ends_with('…'), "exactly 200 chars is not cut");
    }

    #[test]
    fn code_line_width_is_a_parameter() {
        let out = format_code(&[(1, "abcdefghij".into()), (2, "abc".into())], 0, 4);
        assert_eq!(out, "  1 │ abcd…\n  2 │ abc");
    }

    #[test]
    fn configured_width_reaches_comments_from_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), format!("{}\n", "z".repeat(60))).unwrap();
        let r: CodeRef = "a.txt:1".parse().unwrap();
        let mut c = cfg();
        c.max_code_line_width = 30;
        let p = prepare_comment(&c, &r, "q", dir.path()).unwrap();
        assert!(p.payload.contains(&format!("│ {}…\n", "z".repeat(30))), "{}", p.payload);
        // default keeps the whole 60-char line
        let p = prepare_comment(&cfg(), &r, "q", dir.path()).unwrap();
        assert!(p.payload.contains(&format!("│ {}\n", "z".repeat(60))), "{}", p.payload);
    }

    #[test]
    fn empty_text_rejected_and_trailing_whitespace_trimmed() {
        assert!(clean_comment_text("  \n\t ").is_err());
        assert_eq!(clean_comment_text("hello  \n\n").unwrap(), "hello");
        assert_eq!(clean_comment_text("a\r\nb").unwrap(), "a\nb");
    }

    #[test]
    fn control_chars_cannot_escape_the_paste() {
        let evil = "ok\x1b[201~/clear\r\x03done\u{9b}x";
        let c = clean_comment_text(evil).unwrap();
        assert!(!c.contains('\x1b') && !c.contains('\x03') && !c.contains('\u{9b}'));
        assert_eq!(c, "ok[201~/clear\ndonex");
        // tabs and newlines are kept
        assert_eq!(sanitize("a\tb\nc"), "a\tb\nc");
    }

    #[test]
    fn template_that_would_start_with_slash_is_rejected() {
        assert!(check_payload("/clear").is_err());
        assert!(check_payload("  !ls").is_err());
        assert!(check_payload("[comment] ok").is_ok());
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.template = "{text}".into();
        // body can't start with / (rule 1); and a template of "/{text}" trips the final guard
        c.template = "/{text}".into();
        let r: CodeRef = "x.rs:1".parse().unwrap();
        assert!(prepare_comment(&c, &r, "hi", dir.path()).is_err());
    }

    // ---- building from disk -------------------------------------------------------------

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn prepare_in_git_repo_uses_worktree_basename_and_relative_path() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("feat-login");
        std::fs::create_dir_all(repo.join("src/auth")).unwrap();
        git(&repo, &["init", "-q"]);
        let body = (1..=200).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        std::fs::write(repo.join("src/auth/session.ts"), body).unwrap();

        // absolute ref, cwd elsewhere
        let r: CodeRef = format!("{}:142", repo.join("src/auth/session.ts").display()).parse().unwrap();
        let p = prepare_comment(&cfg(), &r, "Why?", d.path()).unwrap();
        assert_eq!(p.worktree, "feat-login");
        assert_eq!(p.rel_path, "src/auth/session.ts");
        assert_eq!(
            p.payload,
            "[comment] src/auth/session.ts:142  (worktree: feat-login)\n  142 │ line 142\nWhy?"
        );

        // relative ref + range, cwd = subdir of the repo
        let r: CodeRef = "session.ts:98-101".parse().unwrap();
        let p = prepare_comment(&cfg(), &r, "range q", &repo.join("src/auth")).unwrap();
        assert!(p.payload.starts_with("[comment] src/auth/session.ts:98-101  (worktree: feat-login)\n"));
        assert!(p.payload.contains("\n   98 │ line 98\n   99 │ line 99\n  100 │ line 100\n  101 │ line 101\n"));
        assert!(p.payload.ends_with("\nrange q"));

        // max_code_lines truncation
        let mut c = cfg();
        c.max_code_lines = 2;
        let r: CodeRef = "src/auth/session.ts:10-19".parse().unwrap();
        let p = prepare_comment(&c, &r, "t", &repo).unwrap();
        assert!(p.payload.contains("  10 │ line 10\n  11 │ line 11\n  … (8 more lines)\n"), "{}", p.payload);

        // past EOF / missing file: no code lines, still sendable
        let r: CodeRef = "src/auth/session.ts:999".parse().unwrap();
        let p = prepare_comment(&cfg(), &r, "eof", &repo).unwrap();
        assert_eq!(p.payload, "[comment] src/auth/session.ts:999  (worktree: feat-login)\neof");
        let r: CodeRef = "src/gone.ts:3".parse().unwrap();
        let p = prepare_comment(&cfg(), &r, "gone", &repo).unwrap();
        assert!(p.payload.starts_with("[comment] src/gone.ts:3  (worktree: feat-login)\ngone"));
    }

    #[test]
    fn prepare_outside_git_falls_back_to_cwd() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().join("plain");
        std::fs::create_dir_all(&w).unwrap();
        std::fs::write(w.join("a.txt"), "one\ntwo\n").unwrap();
        let r: CodeRef = "a.txt:2".parse().unwrap();
        let p = prepare_comment(&cfg(), &r, "q", &w).unwrap();
        // temp dirs live outside any repo (unless TMPDIR is inside one; then skip the strict check)
        if git_toplevel(&w.join("a.txt")).is_none() {
            assert_eq!(p.payload, "[comment] a.txt:2  (worktree: plain)\n  2 │ two\nq");
        }
    }

    // ---- delivery -----------------------------------------------------------------------

    fn row(state: &str) -> AgentRow {
        AgentRow {
            tab_index: 1,
            tab_label: "1".into(),
            tab_id: "w1:t1".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p2".into(),
            kind: "claude".into(),
            state: state.into(),
            name: None,
            is_primary: true,
        }
    }

    fn prompt_ok() -> FakeHerdr {
        FakeHerdr::start(|method, params| match method {
            "agent.prompt" => {
                assert!(params.get("wait").is_none());
                Ok(json!({"type":"agent_prompted","agent":{"pane_id":"w1:p2","agent":"claude","agent_status":"idle","workspace_id":"w1","tab_id":"w1:t1"}}))
            }
            _ => Err(("invalid_request".into(), "?".into())),
        })
    }

    #[test]
    fn deliver_uses_agent_prompt_without_wait_and_exact_text() {
        let fake = prompt_ok();
        let payload = "[comment] a.rs:1  (worktree: w)\n  1 │ x\nhi │ ünï";
        for state in ["idle", "done", "working"] {
            let d = deliver(&Client::new(fake.path()), &row(state), payload).unwrap();
            assert!(d.warnings.is_empty(), "{state}");
        }
        let reqs = fake.requests_for("agent.prompt");
        assert_eq!(reqs.len(), 3);
        for r in reqs {
            assert_eq!(r.params, json!({"target":"w1:p2","text":payload}));
        }
    }

    #[test]
    fn blocked_warns_but_still_sends_and_refusal_is_reported() {
        let fake = prompt_ok();
        let d = deliver(&Client::new(fake.path()), &row("blocked"), "[comment] x").unwrap();
        assert_eq!(d.warnings.len(), 1);
        assert!(d.warnings[0].contains("blocked"));
        assert_eq!(fake.requests_for("agent.prompt").len(), 1);

        let refusing = FakeHerdr::start(|_, _| Err(("agent_blocked".into(), "agent w1:p2 is blocked".into())));
        let e = deliver(&Client::new(refusing.path()), &row("blocked"), "[comment] x").unwrap_err();
        assert_eq!(e.exit_code(), crate::error::exit::REFUSED);
        assert_eq!(e.kind(), "agent_blocked");
        assert!(e.hint().unwrap().contains("approval"));
        assert!(e.to_string().contains("agent_blocked"));
        // never falls back to raw input
        assert!(refusing.requests().iter().all(|r| r.method == "agent.prompt"));
    }
}
