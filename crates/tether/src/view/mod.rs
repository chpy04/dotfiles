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
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyEventKind,
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

struct Runner {
    exe: PathBuf,
    cwd: PathBuf,
    tx: Sender<Msg>,
}

impl Runner {
    /// Perform an effect; returns true when the viewer should quit.
    fn perform(&self, effect: Effect) -> bool {
        let Some(argv) = effect.argv() else { return matches!(effect, Effect::Quit) };
        let stdin = effect.stdin().map(str::to_string);
        let (exe, cwd, tx) = (self.exe.clone(), self.cwd.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let out = run_tether(&exe, &cwd, &argv, stdin.as_deref());
            let msg = match effect {
                Effect::OpenCode { .. } => Msg::OpenDone(out),
                Effect::Send { node, to, text, .. } => Msg::SendDone { result: out, node: ObjectId::from(node), to, text },
                Effect::FetchAgents => Msg::Agents(out.and_then(|s| {
                    serde_json::from_str::<Vec<AgentRow>>(&s).map_err(|e| format!("unexpected `tether agents` output: {e}"))
                })),
                Effect::Quit => return,
            };
            let _ = tx.send(msg);
        });
        false
    }
}

/// Run `tether <argv>`; Ok(stdout) or Err(first useful line of stderr).
fn run_tether(exe: &PathBuf, cwd: &PathBuf, argv: &[String], stdin: Option<&str>) -> std::result::Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(argv)
        .current_dir(cwd)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    let (mut out, mut err) = (String::new(), String::new());
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
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
    let runner = Runner { exe, cwd: store.worktree_root().to_path_buf(), tx };
    // learn the primary agent's tab for the comment float title (fire and forget)
    runner.perform(Effect::FetchAgents);
    event_loop(app, runner, rx, cfg.view.mouse).map_err(|e| TetherError::Other(format!("terminal error: {e}")))
}

fn event_loop(mut app: App, runner: Runner, rx: mpsc::Receiver<Msg>, mouse: bool) -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    if mouse {
        execute!(stdout, EnableMouseCapture)?;
    }
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev_hook(info);
    }));
    let result = (|| {
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
                dirty = true;
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
    })();
    restore_terminal();
    result
}

fn restore_terminal() {
    let mut out = std::io::stdout();
    let _ = execute!(out, DisableMouseCapture, DisableBracketedPaste, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}
