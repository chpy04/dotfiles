//! The diagram viewer (`tether view <id>`, PRD Feature 3).
//!
//! Architecture:
//! * [`app`]: the object model ([`app::App`]: diagram, layout, focus, expanded set, scroll, modes)
//!   and `handle_key` & co, which return [`app::Effect`]s instead of doing I/O.
//! * [`nav`]: hjkl semantics. [`keys`]: key notation and the configurable keymap (leader
//!   sequences time out after 1 s). [`input`]: the multi-line editor of the comment float.
//! * [`draw`]: renders the app into a ratatui buffer every frame (rounded/heavy boxes, theme
//!   colors from `tether_core::theme`, icons from [`icons`], overlays). [`export`]: the same
//!   buffer as text or ANSI, for golden tests and `tether diagram render`.
//! * this file: the terminal loop and the effect runner, which spawns `tether` subprocesses
//!   (`code open`, `send`, `agents`) on worker threads and reports back through a channel, so the
//!   UI never blocks.

pub mod app;
pub mod draw;
pub mod export;
pub mod icons;
pub mod input;
pub mod keys;
pub mod nav;

#[cfg(test)]
mod tests;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use tether_core::config::Config;
use tether_core::diagram::{Diagram, DiagramStore};
use tether_core::error::{Result, TetherError};
use tether_core::layout::{Engine, ObjectId};
use tether_core::theme::Theme;
use tether_core::workspace::AgentRow;

use app::{App, AppInit, Effect, Msg};
use icons::Icons;

/// Build the app state shared by the live viewer and static rendering.
pub fn build_app(
    diagram: Diagram,
    diagram_id: &str,
    worktree: PathBuf,
    cfg: &Config,
    icons_override: Option<bool>,
    term: (u16, u16),
) -> Result<App> {
    let theme = Theme::from_config(&cfg.theme).map_err(TetherError::Config)?;
    let icons = match icons_override {
        Some(b) => Icons::new(b),
        None => Icons::resolve(cfg.view.icons, std::env::var("TETHER_ICONS").ok().as_deref()),
    };
    let pane_id = std::env::var("HERDR_PANE_ID").ok().filter(|s| !s.is_empty());
    App::new(AppInit {
        diagram,
        diagram_id: diagram_id.to_string(),
        worktree,
        keys: cfg.keys.clone(),
        theme,
        icons,
        pane_id,
        term,
        engine: Engine::Auto,
    })
    .map_err(|e| TetherError::Other(format!("cannot lay out the diagram: {e}")))
}

/// Options of a static (non-interactive) render.
#[derive(Debug, Clone, Default)]
pub struct RenderOpts {
    /// Object to focus (default: the initial focus).
    pub focus: Option<String>,
    /// Objects whose details are expanded.
    pub expand: Vec<String>,
    /// Force icons on/off (default: config / `TETHER_ICONS`).
    pub icons: Option<bool>,
    /// Minimum terminal width of the render.
    pub min_width: u16,
    /// Resolve code links against the files (stale links get a badge). Off for docs renders of
    /// diagrams whose sources do not exist here.
    pub resolve_links: bool,
}

/// Render a diagram without a terminal. Returns the text and ANSI renderings.
pub fn render_static(diagram: Diagram, worktree: PathBuf, cfg: &Config, opts: &RenderOpts) -> Result<(String, String)> {
    let mut app = build_app(diagram, "static", worktree, cfg, opts.icons, (200, 60))?;
    let expand: Vec<ObjectId> = opts.expand.iter().map(|s| ObjectId::from(s.as_str())).collect();
    if let Some(id) = &opts.expand.first() {
        if app.layout.object(id).is_none() {
            return Err(TetherError::Input(format!("no object `{id}` in the diagram")));
        }
    }
    app.set_expanded(&expand).map_err(TetherError::Other)?;
    if let Some(f) = &opts.focus {
        if app.layout.object(f).is_none() {
            return Err(TetherError::Input(format!("no object `{f}` in the diagram")));
        }
        app.set_focus(ObjectId::from(f.as_str()));
    }
    let buf = export::render_full(&mut app, opts.min_width.max(60), opts.resolve_links);
    Ok((export::buffer_to_text(&buf), export::buffer_to_ansi(&buf)))
}

// ------------------------------------------------------------------------------------ runner

/// Most worker threads running `tether` subprocesses at once; further effects are refused with
/// a status message (the draft of a refused `send` is kept).
const MAX_IN_FLIGHT: usize = 4;

/// Child timeouts per effect kind.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(15);
const AGENTS_TIMEOUT: Duration = Duration::from_secs(5);

struct Runner {
    exe: PathBuf,
    cwd: PathBuf,
    tx: Sender<Msg>,
    in_flight: Arc<AtomicUsize>,
}

/// Decrements the in-flight counter when a worker ends, however it ends.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Runner {
    fn new(exe: PathBuf, cwd: PathBuf, tx: Sender<Msg>) -> Runner {
        Runner { exe, cwd, tx, in_flight: Arc::new(AtomicUsize::new(0)) }
    }

    /// Perform an effect; returns true when the viewer should quit.
    fn perform(&self, effect: Effect) -> bool {
        let Some(argv) = effect.argv() else { return matches!(effect, Effect::Quit) };
        let timeout = match effect {
            Effect::OpenCode { .. } => OPEN_TIMEOUT,
            Effect::Send { .. } => SEND_TIMEOUT,
            _ => AGENTS_TIMEOUT,
        };
        let stdin = effect.stdin().map(str::to_string);
        let (exe, cwd, tx) = (self.exe.clone(), self.cwd.clone(), self.tx.clone());
        // reserve a slot; over the cap the effect fails right away instead of piling up threads
        let slot = Slot(self.in_flight.clone());
        if self.in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
            drop(slot);
            let _ = tx.send(finish(effect, Err("too many actions in flight; try again in a moment".to_string())));
            return false;
        }
        std::thread::spawn(move || {
            let _slot = slot;
            let out = run_tether(&exe, &cwd, &argv, stdin.as_deref(), timeout);
            if matches!(effect, Effect::Quit) {
                return;
            }
            let _ = tx.send(finish(effect, out));
        });
        false
    }
}

/// Turn the outcome of an effect's subprocess into the message the app expects.
fn finish(effect: Effect, out: std::result::Result<String, String>) -> Msg {
    match effect {
        Effect::OpenCode { .. } => Msg::OpenDone(out),
        Effect::Send { node, to, text, .. } => Msg::SendDone { result: out, node: ObjectId::from(node), to, text },
        Effect::FetchAgents => Msg::Agents(out.and_then(|s| {
            serde_json::from_str::<Vec<AgentRow>>(&s).map_err(|e| format!("unexpected `tether agents` output: {e}"))
        })),
        Effect::Quit => Msg::OpenDone(out),
    }
}

/// Read a pipe to the end on its own thread; the text arrives on the returned channel.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    rx
}

/// Run `tether <argv>` with a deadline; Ok(stdout) or Err(first useful line of stderr). On
/// timeout the child is killed and the error says so. Both pipes are drained concurrently with
/// the wait, so a chatty child cannot deadlock on a full pipe.
fn run_tether(
    exe: &PathBuf,
    cwd: &PathBuf,
    argv: &[String],
    stdin: Option<&str>,
    timeout: Duration,
) -> std::result::Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(argv)
        .current_dir(cwd)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let (out_rx, err_rx) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let what = argv.first().map(String::as_str).unwrap_or("tether");
                return Err(format!("`tether {what}` timed out after {}s and was stopped", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.to_string()),
        }
    };
    // a grandchild that inherited the pipes must not hold us up
    let grace = Duration::from_millis(500);
    let out = out_rx.recv_timeout(grace).unwrap_or_default();
    let err = err_rx.recv_timeout(grace).unwrap_or_default();
    if status.success() {
        Ok(out.trim().to_string())
    } else {
        let line = err
            .lines()
            .map(|l| l.trim().trim_start_matches("tether: error: ").trim_start_matches("error: "))
            .find(|l| !l.is_empty())
            .unwrap_or("failed")
            .to_string();
        Err(line)
    }
}

// ------------------------------------------------------------------------------------ entry point

/// Run the interactive viewer for a stored diagram in the current pane.
pub fn run(diagram_id: &str) -> Result<()> {
    use std::io::IsTerminal;
    let cwd = std::env::current_dir().map_err(|e| TetherError::Other(format!("cannot determine cwd: {e}")))?;
    let store = DiagramStore::open(&cwd);
    let diagram = store.get(diagram_id).map_err(|e| TetherError::Input(e.to_string()))?;
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(TetherError::Input(
            "the viewer needs an interactive terminal; run it in a pane (`tether view <id> --placement tab`, or `tether diagram create spec.json` which opens a tab)".into(),
        ));
    }
    let cfg = Config::load()?;
    let term = ratatui::crossterm::terminal::size().unwrap_or((100, 30));
    let app = build_app(diagram, diagram_id, store.worktree_root().to_path_buf(), &cfg, None, term)?;
    let exe = std::env::current_exe().map_err(|e| TetherError::Other(format!("cannot locate the tether binary: {e}")))?;
    let (tx, rx) = mpsc::channel();
    let runner = Runner::new(exe, store.worktree_root().to_path_buf(), tx);
    // learn the primary agent's tab for the comment float title (fire and forget)
    runner.perform(Effect::FetchAgents);
    event_loop(app, runner, rx, cfg.view.mouse).map_err(|e| TetherError::Other(format!("terminal error: {e}")))
}

/// True while the terminal is in the viewer's raw/alternate-screen state.
static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Restores the terminal when dropped: on normal return, on `?` errors after raw mode was
/// enabled, and during a panic unwind. A panic hook (see [`event_loop`]) restores first so the
/// panic message lands on the normal screen.
struct TerminalGuard;

impl TerminalGuard {
    fn enter(mouse: bool) -> std::io::Result<TerminalGuard> {
        // armed before the first fallible call: a failure half-way still restores everything
        let guard = TerminalGuard;
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        if mouse {
            execute!(stdout, EnableMouseCapture)?;
        }
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Whether an event needs a redraw. Mouse moves (and drags, focus changes, ...) do not.
fn redraws(ev: &Event) -> bool {
    match ev {
        Event::Key(k) => k.kind != KeyEventKind::Release,
        Event::Mouse(m) => matches!(
            m.kind,
            MouseEventKind::Down(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        Event::Paste(_) | Event::Resize(..) => true,
        _ => false,
    }
}

fn event_loop(mut app: App, runner: Runner, rx: mpsc::Receiver<Msg>, mouse: bool) -> std::io::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev_hook(info);
    }));
    let _guard = TerminalGuard::enter(mouse)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut dirty = true;
    loop {
        if dirty {
            app.prepare_frame(Instant::now());
            terminal.draw(|f| {
                let area = f.area();
                if area.width != app.term.0 || area.height != app.term.1 {
                    app.resize(area.width, area.height);
                    app.prepare_frame(Instant::now());
                }
                draw::draw(&app, f.buffer_mut(), area);
            })?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(100))? {
            let ev = event::read()?;
            dirty |= redraws(&ev);
            let effects = match ev {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.handle_key(k),
                Event::Mouse(m) => app.handle_mouse(m),
                Event::Paste(s) => {
                    app.handle_paste(&s);
                    vec![]
                }
                Event::Resize(w, h) => {
                    app.resize(w, h);
                    vec![]
                }
                _ => vec![],
            };
            for e in effects {
                if runner.perform(e) {
                    return Ok(());
                }
            }
        }
        while let Ok(m) = rx.try_recv() {
            app.on_msg(m);
            dirty = true;
        }
        if app.tick(Instant::now()) {
            dirty = true;
        }
    }
}

fn restore_terminal() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = std::io::stdout();
    let _ = execute!(out, DisableMouseCapture, DisableBracketedPaste, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

#[cfg(test)]
mod runner_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, body: &str) -> PathBuf {
        let p = dir.join("fake-tether");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn hung_child_is_killed_and_reported() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "sleep 30");
        let cwd = d.path().to_path_buf();
        let t = Instant::now();
        let r = run_tether(&exe, &cwd, &["agents".into()], None, Duration::from_millis(200));
        assert!(t.elapsed() < Duration::from_secs(5));
        let e = r.unwrap_err();
        assert!(e.contains("timed out"), "{e}");
    }

    #[test]
    fn chatty_child_with_stdin_does_not_deadlock() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "yes x | head -c 300000 >&2; cat >/dev/null; echo done");
        let big = "y".repeat(300_000);
        let r = run_tether(&exe, &d.path().to_path_buf(), &["send".into()], Some(&big), Duration::from_secs(10));
        assert_eq!(r.unwrap(), "done");
    }

    #[test]
    fn effects_over_the_cap_are_refused_not_queued_forever() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "sleep 1; echo ok");
        let (tx, rx) = mpsc::channel();
        let runner = Runner::new(exe, d.path().to_path_buf(), tx);
        let eff = || Effect::OpenCode { abs_path: PathBuf::from("/x"), rel_path: "x".into(), line: 1, from_pane: None };
        for _ in 0..(MAX_IN_FLIGHT + 2) {
            runner.perform(eff());
        }
        // the two over the cap fail immediately, the rest only after the child ends
        let mut refused = 0;
        while let Ok(m) = rx.recv_timeout(Duration::from_millis(300)) {
            match m {
                Msg::OpenDone(Err(e)) if e.contains("too many") => refused += 1,
                other => panic!("unexpected early message {other:?}"),
            }
        }
        assert_eq!(refused, 2);
        // once the workers finish, slots are free again
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(runner.in_flight.load(Ordering::SeqCst), 0);
    }
}
