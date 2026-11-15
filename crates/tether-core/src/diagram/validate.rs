//! Validation: JSON Schema first, then semantic checks.

use super::schema::{flowchart_schema, sequence_schema};
use super::spec::*;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;

/// A problem in a spec, located by JSON pointer so an agent can fix it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationError {
    /// JSON pointer into the spec (empty string is the document root).
    pub path: String,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl ValidationError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = if self.path.is_empty() {
            "(root)"
        } else {
            &self.path
        };
        write!(f, "{p}: {}", self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Render errors one per line, for CLI output.
pub fn format_errors(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|e| format!("- {e}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn validators() -> &'static (Option<jsonschema::Validator>, Option<jsonschema::Validator>) {
    static V: OnceLock<(Option<jsonschema::Validator>, Option<jsonschema::Validator>)> =
        OnceLock::new();
    V.get_or_init(|| {
        (
            jsonschema::validator_for(&flowchart_schema()).ok(),
            jsonschema::validator_for(&sequence_schema()).ok(),
        )
    })
}

/// Parse, schema-validate and semantically validate a spec.
///
/// On success the returned diagram is normalized (message/note ids assigned).
pub fn validate(json: &str) -> Result<Diagram, Vec<ValidationError>> {
    let value: Value = serde_json::from_str(json).map_err(|e| {
        vec![ValidationError::new(
            "",
            format!(
                "invalid JSON at line {} column {}: {e}",
                e.line(),
                e.column()
            ),
        )]
    })?;
    let kind = match value.get("type").and_then(Value::as_str) {
        Some("flowchart") => DiagramKind::Flowchart,
        Some("sequence") => DiagramKind::Sequence,
        Some(other) => {
            return Err(vec![ValidationError::new(
                "/type",
                format!("unknown diagram type \"{other}\"; expected \"flowchart\" or \"sequence\""),
            )])
        }
        None => {
            return Err(vec![ValidationError::new(
                "/type",
                "missing required \"type\"; expected \"flowchart\" or \"sequence\"",
            )])
        }
    };
    let (fc, sq) = validators();
    let validator = match kind {
        DiagramKind::Flowchart => fc,
        DiagramKind::Sequence => sq,
    };
    let Some(validator) = validator else {
        return Err(vec![ValidationError::new(
            "",
            "internal error: schema failed to compile",
        )]);
    };
    let mut schema_errors = Vec::new();
    collect_schema_errors(validator, &value, "", &mut schema_errors);
    if !schema_errors.is_empty() {
        return Err(dedup(schema_errors));
    }
    let mut diagram: Diagram = serde_json::from_value(value).map_err(|e| {
        vec![ValidationError::new(
            "",
            format!("could not read spec: {e}"),
        )]
    })?;
    normalize(&mut diagram);
    let errors = validate_diagram(&diagram);
    if errors.is_empty() {
        Ok(diagram)
    } else {
        Err(errors)
    }
}

/// Validators for the three sequence item kinds, used to explain `oneOf` failures precisely.
fn item_validators() -> &'static HashMap<&'static str, jsonschema::Validator> {
    static V: OnceLock<HashMap<&'static str, jsonschema::Validator>> = OnceLock::new();
    V.get_or_init(|| {
        let root = sequence_schema();
        let mut m = HashMap::new();
        let arms = root["$defs"]["Item"]["oneOf"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for arm in arms {
            let tag = match arm["properties"]["type"]["const"].as_str() {
                Some("message") => "message",
                Some("note") => "note",
                Some("group") => "group",
                _ => continue,
            };
            let Value::Object(mut sub) = arm else {
                continue;
            };
            sub.insert("$schema".into(), root["$schema"].clone());
            sub.insert("$defs".into(), root["$defs"].clone());
            if let Ok(v) = jsonschema::validator_for(&Value::Object(sub)) {
                m.insert(tag, v);
            }
        }
        m
    })
}

/// Collect schema errors; a failed item `oneOf` is re-validated against the item kind named by
/// its `"type"` so the agent sees the real problem and its pointer.
fn collect_schema_errors(
    validator: &jsonschema::Validator,
    instance: &Value,
    prefix: &str,
    out: &mut Vec<ValidationError>,
) {
    for e in validator.iter_errors(instance) {
        let rel = e.instance_path().to_string();
        let path = format!("{prefix}{rel}");
        let msg = e.to_string();
        if msg.contains("'oneOf' keyword") || msg.contains("'anyOf' keyword") {
            if let Some(sub) = instance.pointer(&rel) {
                match sub.get("type").and_then(Value::as_str) {
                    Some(t) => {
                        if let Some(v) = item_validators().get(t) {
                            collect_schema_errors(v, sub, &path, out);
                            continue;
                        }
                        out.push(ValidationError::new(
                            format!("{path}/type"),
                            format!("unknown item type \"{t}\"; expected \"message\", \"note\" or \"group\""),
                        ));
                        continue;
                    }
                    None if sub.is_object() => {
                        out.push(ValidationError::new(
                            path,
                            "item is missing \"type\"; expected \"message\", \"note\" or \"group\"",
                        ));
                        continue;
                    }
                    None => {}
                }
            }
        }
        out.push(ValidationError::new(path, msg));
    }
}

fn dedup(mut errors: Vec<ValidationError>) -> Vec<ValidationError> {
    let mut seen = HashSet::new();
    errors.retain(|e| seen.insert((e.path.clone(), e.message.clone())));
    errors
}

/// Assign missing message/note ids (`m1`.., `n1`..). Idempotent.
pub fn normalize(d: &mut Diagram) {
    if let Diagram::Sequence(s) = d {
        s.assign_ids();
    }
}

/// Run the semantic checks on an already-typed diagram (ids must be assigned).
pub fn validate_diagram(d: &Diagram) -> Vec<ValidationError> {
    let mut errs = Vec::new();
    match d {
        Diagram::Flowchart(f) => check_flowchart(f, &mut errs),
        Diagram::Sequence(s) => check_sequence(s, &mut errs),
    }
    errs
}

fn nonempty(errs: &mut Vec<ValidationError>, path: String, what: &str, s: &str) {
    if s.trim().is_empty() {
        errs.push(ValidationError::new(
            path,
            format!("{what} must not be empty or whitespace"),
        ));
    }
}

fn check_code(errs: &mut Vec<ValidationError>, path: &str, c: &CodeRef) {
    let p = c.path.as_str();
    if p.trim().is_empty() {
        errs.push(ValidationError::new(
            format!("{path}/path"),
            "code path must not be empty",
        ));
    } else if p.starts_with('/') || p.starts_with('\\') || p.split(['/', '\\']).any(|s| s == "..") {
        errs.push(ValidationError::new(
            format!("{path}/path"),
            format!("code path \"{p}\" must be relative to the worktree root (no leading '/' and no '..')"),
        ));
    }
    if c.line < 1 {
        errs.push(ValidationError::new(
            format!("{path}/line"),
            "line is 1-based and must be >= 1",
        ));
    }
    if let Some(end) = c.end_line {
        if end < c.line {
            errs.push(ValidationError::new(
                format!("{path}/end_line"),
                format!("end_line ({end}) must be >= line ({})", c.line),
            ));
        }
    }
    if let Some(a) = &c.anchor {
        if a.trim().is_empty() {
            errs.push(ValidationError::new(
                format!("{path}/anchor"),
                "anchor must not be empty; omit it or give a snippet of the source line",
            ));
        }
    }
}

fn check_details(errs: &mut Vec<ValidationError>, path: &str, ds: &[Detail]) {
    for (i, d) in ds.iter().enumerate() {
        nonempty(errs, format!("{path}/{i}/title"), "detail title", &d.title);
    }
}

fn dup_hint(kind: &str, id: &str, first: &str) -> String {
    format!("duplicate {kind} id \"{id}\" (first used at {first}); ids must be unique")
}

fn check_flowchart(f: &Flowchart, errs: &mut Vec<ValidationError>) {
    nonempty(errs, "/title".into(), "title", &f.title);
    let mut ids: HashMap<&str, usize> = HashMap::new();
    for (i, n) in f.nodes.iter().enumerate() {
        let base = format!("/nodes/{i}");
        nonempty(errs, format!("{base}/id"), "node id", &n.id);
        nonempty(errs, format!("{base}/label"), "node label", &n.label);
        if let Some(first) = ids.insert(n.id.as_str(), i) {
            errs.push(ValidationError::new(
                format!("{base}/id"),
                dup_hint("node", &n.id, &format!("/nodes/{first}/id")),
            ));
        }
        if let Some(c) = &n.code {
            check_code(errs, &format!("{base}/code"), c);
        }
        check_details(errs, &format!("{base}/details"), &f.nodes[i].details);
    }
    let known = || {
        let mut v: Vec<&str> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v.join(", ")
    };
    for (i, e) in f.edges.iter().enumerate() {
        for (field, id) in [("from", &e.from), ("to", &e.to)] {
            if !ids.contains_key(id.as_str()) {
                errs.push(ValidationError::new(
                    format!("/edges/{i}/{field}"),
                    format!(
                        "edge refers to unknown node \"{id}\"; known node ids: {}",
                        known()
                    ),
                ));
            }
        }
        if let Some(l) = &e.label {
            nonempty(errs, format!("/edges/{i}/label"), "edge label", l);
        }
    }
}

struct SeqCtx<'a> {
    participants: HashSet<&'a str>,
    item_ids: HashMap<&'a str, String>,
}

fn check_sequence(s: &Sequence, errs: &mut Vec<ValidationError>) {
    nonempty(errs, "/title".into(), "title", &s.title);
    let mut ctx = SeqCtx {
        participants: HashSet::new(),
        item_ids: HashMap::new(),
    };
    for (i, p) in s.participants.iter().enumerate() {
        let base = format!("/participants/{i}");
        nonempty(errs, format!("{base}/id"), "participant id", &p.id);
        nonempty(errs, format!("{base}/label"), "participant label", &p.label);
        if !ctx.participants.insert(p.id.as_str()) {
            let first = s
                .participants
                .iter()
                .position(|q| q.id == p.id)
                .unwrap_or(0);
            errs.push(ValidationError::new(
                format!("{base}/id"),
                dup_hint("participant", &p.id, &format!("/participants/{first}/id")),
            ));
        }
    }
    check_items(&s.items, "/items", &mut ctx, s, errs);
}

fn check_items<'a>(
    items: &'a [Item],
    base: &str,
    ctx: &mut SeqCtx<'a>,
    s: &'a Sequence,
    errs: &mut Vec<ValidationError>,
) {
    let known = || {
        s.participants
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    for (i, it) in items.iter().enumerate() {
        let p = format!("{base}/{i}");
        match it {
            Item::Message(m) => {
                nonempty(errs, format!("{p}/id"), "message id", &m.id);
                nonempty(errs, format!("{p}/label"), "message label", &m.label);
                if let Some(first) = ctx.item_ids.insert(m.id.as_str(), format!("{p}/id")) {
                    errs.push(ValidationError::new(
                        format!("{p}/id"),
                        dup_hint("message/note", &m.id, &first),
                    ));
                }
                for (field, id) in [("from", &m.from), ("to", &m.to)] {
                    if !ctx.participants.contains(id.as_str()) {
                        errs.push(ValidationError::new(
                            format!("{p}/{field}"),
                            format!("message refers to unknown participant \"{id}\"; known participant ids: {}", known()),
                        ));
                    }
                }
                if let Some(c) = &m.code {
                    check_code(errs, &format!("{p}/code"), c);
                }
                check_details(errs, &format!("{p}/details"), &m.details);
            }
            Item::Note(n) => {
                nonempty(errs, format!("{p}/id"), "note id", &n.id);
                nonempty(errs, format!("{p}/text"), "note text", &n.text);
                if let Some(first) = ctx.item_ids.insert(n.id.as_str(), format!("{p}/id")) {
                    errs.push(ValidationError::new(
                        format!("{p}/id"),
                        dup_hint("message/note", &n.id, &first),
                    ));
                }
                for (j, id) in n.over.iter().enumerate() {
                    if !ctx.participants.contains(id.as_str()) {
                        errs.push(ValidationError::new(
                            format!("{p}/over/{j}"),
                            format!("note refers to unknown participant \"{id}\"; known participant ids: {}", known()),
                        ));
                    }
                }
                check_details(errs, &format!("{p}/details"), &n.details);
            }
            Item::Group(g) => {
                nonempty(errs, format!("{p}/label"), "group label", &g.label);
                let has_b = !g.branches.is_empty();
                let has_i = !g.items.is_empty();
                match g.kind {
                    GroupKind::Alt if !has_b => errs.push(ValidationError::new(
                        format!("{p}/branches"),
                        "an alt group needs \"branches\" (each with optional label and items)",
                    )),
                    GroupKind::Opt | GroupKind::Loop if !has_i => errs.push(ValidationError::new(
                        format!("{p}/items"),
                        "opt and loop groups need a non-empty \"items\" array",
                    )),
                    GroupKind::Par if !has_b && !has_i => errs.push(ValidationError::new(
                        p.clone(),
                        "a par group needs \"branches\" (or \"items\" for a single lane)",
                    )),
                    _ => {}
                }
                if has_b && has_i {
                    errs.push(ValidationError::new(
                        p.clone(),
                        "a group must use either \"branches\" or \"items\", not both",
                    ));
                }
                if has_b && matches!(g.kind, GroupKind::Opt | GroupKind::Loop) {
                    errs.push(ValidationError::new(
                        format!("{p}/branches"),
                        "opt and loop groups take \"items\", not \"branches\"",
                    ));
                }
                if has_i && matches!(g.kind, GroupKind::Alt) {
                    errs.push(ValidationError::new(
                        format!("{p}/items"),
                        "an alt group takes \"branches\", not \"items\"",
                    ));
                }
                check_items(&g.items, &format!("{p}/items"), ctx, s, errs);
                for (bi, b) in g.branches.iter().enumerate() {
                    let bp = format!("{p}/branches/{bi}");
                    if let Some(l) = &b.label {
                        nonempty(errs, format!("{bp}/label"), "branch label", l);
                    }
                    check_items(&b.items, &format!("{bp}/items"), ctx, s, errs);
                }
            }
        }
    }
}
