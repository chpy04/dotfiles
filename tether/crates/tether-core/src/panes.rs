//! Pane-level layout operations: shell quoting, running a command in a pane, `pane.split`.
//! All of them are scoped to one workspace (`Scope`, F1.5).

use crate::error::{Result, TetherError};
use crate::herdr::{Client, HerdrError, Pane, PaneSplit};
use crate::workspace::Context;

/// Which workspace an operation may touch, plus where the caller currently is. The default is
/// `HERDR_WORKSPACE_ID`; any other workspace needs an explicit `--workspace` (F1.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub workspace_id: String,
    /// The caller's own tab/pane, only when they are inside `workspace_id`.
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
    /// `--workspace` pointed somewhere other than the caller's own workspace.
    pub explicit_other: bool,
}

impl Scope {
    /// `ctx` is the result of `Context::from_env()`. `workspace_flag` is `--workspace`.
    pub fn resolve(ctx: Result<Context>, workspace_flag: Option<&str>) -> Result<Scope> {
        match (ctx, workspace_flag) {
            (Ok(c), None) => Ok(Scope { workspace_id: c.workspace_id, tab_id: c.tab_id, pane_id: c.pane_id, explicit_other: false }),
            (Ok(c), Some(w)) if c.workspace_id == w => {
                Ok(Scope { workspace_id: c.workspace_id, tab_id: c.tab_id, pane_id: c.pane_id, explicit_other: false })
            }
            (_, Some(w)) => Ok(Scope { workspace_id: w.to_string(), tab_id: None, pane_id: None, explicit_other: true }),
            (Err(e), None) => Err(e),
        }
    }

    pub fn from_env(workspace_flag: Option<&str>) -> Result<Scope> {
        Self::resolve(Context::from_env(), workspace_flag)
    }

    /// The standard refusal for an object that lives in another workspace.
    pub fn refuse_foreign(&self, what: &str, other_ws: &str) -> TetherError {
        TetherError::Refused(format!(
            "{what} belongs to workspace {other_ws}, but tether is scoped to workspace {} (F1.5); \
             pass --workspace {other_ws} to act there explicitly",
            self.workspace_id
        ))
    }
}

/// POSIX single-quote an argument unless it is made of safe characters only.
pub fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'+' | b'@' | b'%' | b','));
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// `["ls", "-la", "my dir"]` -> `ls -la 'my dir'`.
pub fn command_line(args: &[String]) -> String {
    args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
}

/// Type `args` (shell-quoted) into the pane's shell and press Enter.
pub fn run_in_pane(client: &Client, pane_id: &str, args: &[String]) -> Result<()> {
    client.pane_send_input(pane_id, Some(&command_line(args)), &["enter"])?;
    Ok(())
}

/// `pane_get` that maps "not found" to a target error and refuses panes of other workspaces.
pub fn pane_in_scope(client: &Client, scope: &Scope, pane_id: &str) -> Result<Pane> {
    match client.pane_get(pane_id) {
        Ok(p) if p.workspace_id == scope.workspace_id => Ok(p),
        Ok(p) => Err(scope.refuse_foreign(&format!("pane {pane_id}"), &p.workspace_id)),
        Err(HerdrError::Api { code, .. }) if code == "pane_not_found" => {
            Err(TetherError::Target(format!("pane {pane_id} does not exist (see `tether layout`)")))
        }
        Err(e) => Err(e.into()),
    }
}

/// Parsed `split` direction.
pub fn parse_direction(s: &str) -> Result<&'static str> {
    match s {
        "right" => Ok("right"),
        "down" => Ok("down"),
        other => Err(TetherError::Input(format!("invalid direction `{other}`: expected `right` or `down`"))),
    }
}

pub fn check_ratio(r: f32) -> Result<f32> {
    if r.is_finite() && r > 0.0 && r < 1.0 {
        Ok(r)
    } else {
        Err(TetherError::Input(format!("invalid --ratio {r}: expected a fraction strictly between 0 and 1 (share kept by the existing pane)")))
    }
}

/// Result of `split`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Split {
    pub pane_id: String,
    pub split_from: String,
    pub direction: String,
    pub ratio: Option<f32>,
    pub tab_id: String,
    pub ran: Option<String>,
}

/// Options of `split`.
#[derive(Debug, Clone, Default)]
pub struct SplitOpts<'a> {
    pub direction: &'a str,
    pub ratio: Option<f32>,
    /// Pane to split; default is the caller's own pane.
    pub pane: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub focus: bool,
    /// Command to run in the new pane.
    pub cmd: &'a [String],
}

/// Split `pane` (or the caller's own pane) and optionally run `cmd` in the new pane.
pub fn split(client: &Client, scope: &Scope, o: &SplitOpts) -> Result<Split> {
    let direction = parse_direction(o.direction)?;
    let ratio = o.ratio.map(check_ratio).transpose()?;
    let (cwd, focus, cmd) = (o.cwd, o.focus, o.cmd);
    let target = match o.pane.map(str::to_string).or_else(|| scope.pane_id.clone()) {
        Some(p) => p,
        None => {
            return Err(TetherError::Input(
                "no pane to split: HERDR_PANE_ID is not set; pass --pane <id> (see `tether layout`)".into(),
            ))
        }
    };
    // The target must live in the scoped workspace; nothing is created otherwise.
    pane_in_scope(client, scope, &target)?;
    let new = client.pane_split(&PaneSplit { target_pane_id: &target, direction, ratio, cwd, focus })?;
    if !cmd.is_empty() {
        run_in_pane(client, &new.pane_id, cmd)?;
    }
    Ok(Split {
        pane_id: new.pane_id,
        split_from: target,
        direction: direction.to_string(),
        ratio,
        tab_id: new.tab_id,
        ran: (!cmd.is_empty()).then(|| command_line(cmd)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHerdr;
    use serde_json::json;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("abc-1.2/x"), "abc-1.2/x");
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("$(rm -rf)"), "'$(rm -rf)'");
        assert_eq!(command_line(&["tail".into(), "-f".into(), "a b.log".into()]), "tail -f 'a b.log'");
    }

    #[test]
    fn scope_resolution() {
        let ctx = || Ok(Context { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p1".into()) });
        let s = Scope::resolve(ctx(), None).unwrap();
        assert_eq!((s.workspace_id.as_str(), s.pane_id.as_deref(), s.explicit_other), ("w1", Some("w1:p1"), false));
        // --workspace equal to our own keeps the current tab/pane
        assert_eq!(Scope::resolve(ctx(), Some("w1")).unwrap().tab_id.as_deref(), Some("w1:t1"));
        // another workspace: explicit, and our tab/pane no longer apply
        let o = Scope::resolve(ctx(), Some("w2")).unwrap();
        assert_eq!((o.workspace_id.as_str(), o.tab_id, o.pane_id, o.explicit_other), ("w2", None, None, true));
        // outside Herdr: --workspace is enough, otherwise a clear error
        let no = || Err(TetherError::NotInHerdr("not inside".into()));
        assert_eq!(Scope::resolve(no(), Some("w3")).unwrap().workspace_id, "w3");
        assert!(matches!(Scope::resolve(no(), None).unwrap_err(), TetherError::NotInHerdr(_)));
    }

    fn fake() -> FakeHerdr {
        FakeHerdr::start(|method, params| match method {
            "pane.get" => match params["pane_id"].as_str().unwrap() {
                "w1:p1" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1"}})),
                "w2:p1" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1"}})),
                _ => Err(("pane_not_found".into(), "no".into())),
            },
            "pane.split" => Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p7","workspace_id":"w1","tab_id":"w1:t1"}})),
            "pane.send_input" => Ok(json!({"type":"ok"})),
            _ => Err(("invalid_request".into(), "?".into())),
        })
    }

    fn scope() -> Scope {
        Scope { workspace_id: "w1".into(), tab_id: Some("w1:t1".into()), pane_id: Some("w1:p1".into()), explicit_other: false }
    }

    #[test]
    fn split_runs_command_and_defaults_to_current_pane() {
        let f = fake();
        let c = Client::new(f.path());
        let r = split(&c, &scope(), &SplitOpts { direction: "right", ratio: Some(0.3), cmd: &["tail".into(), "-f".into(), "a b".into()], ..Default::default() }).unwrap();
        assert_eq!((r.pane_id.as_str(), r.split_from.as_str()), ("w1:p7", "w1:p1"));
        let sp = &f.requests_for("pane.split")[0].params;
        assert_eq!(sp["target_pane_id"], "w1:p1");
        assert_eq!(sp["direction"], "right");
        assert_eq!(sp["focus"], false);
        assert!((sp["ratio"].as_f64().unwrap() - 0.3).abs() < 1e-6);
        let si = &f.requests_for("pane.send_input")[0].params;
        assert_eq!(si, &json!({"pane_id":"w1:p7","text":"tail -f 'a b'","keys":["enter"]}));
    }

    #[test]
    fn split_refuses_other_workspace_and_bad_input_without_creating() {
        let f = fake();
        let c = Client::new(f.path());
        let e = split(&c, &scope(), &SplitOpts { direction: "right", pane: Some("w2:p1"), ..Default::default() }).unwrap_err();
        assert!(matches!(e, TetherError::Refused(_)), "{e}");
        assert!(e.to_string().contains("--workspace w2"));
        assert!(matches!(split(&c, &scope(), &SplitOpts { direction: "left", ..Default::default() }).unwrap_err(), TetherError::Input(_)));
        assert!(matches!(split(&c, &scope(), &SplitOpts { direction: "down", ratio: Some(1.5), ..Default::default() }).unwrap_err(), TetherError::Input(_)));
        assert!(matches!(split(&c, &scope(), &SplitOpts { direction: "down", pane: Some("w1:p99"), ..Default::default() }).unwrap_err(), TetherError::Target(_)));
        assert!(f.requests_for("pane.split").is_empty(), "nothing may be split on refusal");
        // an explicit --workspace w2 scope may split w2's pane
        let s2 = Scope { workspace_id: "w2".into(), tab_id: None, pane_id: None, explicit_other: true };
        assert!(split(&c, &s2, &SplitOpts { direction: "down", pane: Some("w2:p1"), ..Default::default() }).is_ok());
    }
}
