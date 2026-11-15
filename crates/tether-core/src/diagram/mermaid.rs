//! Mermaid export.

use super::spec::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Export a diagram as Mermaid source (flowchart or sequenceDiagram).
pub fn to_mermaid(d: &Diagram) -> String {
    match d {
        Diagram::Flowchart(f) => flowchart(f),
        Diagram::Sequence(s) => sequence(s),
    }
}

/// Escape text for use inside a Mermaid quoted label or message.
///
/// Uses Mermaid entity codes (`#quot;`, `#35;`, ...) and turns newlines into `<br/>`.
pub fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '#' => out.push_str("#35;"),
            '"' => out.push_str("#quot;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            ';' => out.push_str("#59;"),
            '\r' => {}
            '\n' => out.push_str("<br/>"),
            c => out.push(c),
        }
    }
    out
}

const RESERVED: &[&str] = &[
    "end",
    "graph",
    "flowchart",
    "subgraph",
    "style",
    "class",
    "classdef",
    "click",
    "default",
    "linkstyle",
    "direction",
    "participant",
    "actor",
    "note",
    "loop",
    "alt",
    "else",
    "opt",
    "par",
    "and",
    "over",
    "as",
    "autonumber",
    "title",
    "activate",
    "deactivate",
];

/// Map spec ids to safe Mermaid identifiers (`[A-Za-z0-9_]`, not reserved, unique).
fn safe_ids<'a>(ids: impl Iterator<Item = &'a str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    for id in ids {
        if map.contains_key(id) {
            continue;
        }
        let mut s: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
            s.insert(0, 'n');
        }
        if RESERVED.contains(&s.to_ascii_lowercase().as_str()) {
            s.push('_');
        }
        let base = s.clone();
        let mut k = 1;
        while !taken.insert(s.clone()) {
            k += 1;
            s = format!("{base}_{k}");
        }
        map.insert(id.to_string(), s);
    }
    map
}

fn flowchart(f: &Flowchart) -> String {
    let ids = safe_ids(f.nodes.iter().map(|n| n.id.as_str()));
    let id = |raw: &str| ids.get(raw).cloned().unwrap_or_else(|| raw.to_string());
    let mut out = String::new();
    let dir = match f.direction() {
        Direction::TB => "TD",
        Direction::LR => "LR",
    };
    let _ = writeln!(out, "flowchart {dir}");
    for n in &f.nodes {
        let l = escape_label(&n.label);
        let nid = id(&n.id);
        let line = match n.shape() {
            Shape::Process => format!("{nid}[\"{l}\"]"),
            Shape::Decision => format!("{nid}{{\"{l}\"}}"),
            Shape::Terminal => format!("{nid}([\"{l}\"])"),
            Shape::Io => format!("{nid}[/\"{l}\"/]"),
            Shape::Subroutine => format!("{nid}[[\"{l}\"]]"),
            Shape::Database => format!("{nid}[(\"{l}\")]"),
        };
        let _ = writeln!(out, "    {line}");
    }
    for e in &f.edges {
        let arrow = match e.style() {
            EdgeStyle::Solid => "-->",
            EdgeStyle::Dashed => "-.->",
        };
        let label = match e.label.as_deref().filter(|l| !l.is_empty()) {
            Some(l) => format!("|\"{}\"|", escape_label(l)),
            None => String::new(),
        };
        let _ = writeln!(out, "    {} {arrow}{label} {}", id(&e.from), id(&e.to));
    }
    out
}

fn sequence(s: &Sequence) -> String {
    let ids = safe_ids(s.participants.iter().map(|p| p.id.as_str()));
    let id = |raw: &str| ids.get(raw).cloned().unwrap_or_else(|| raw.to_string());
    let mut out = String::from("sequenceDiagram\n");
    for p in &s.participants {
        let pid = id(&p.id);
        let l = escape_label(&p.label);
        let line = match p.kind {
            Some(ParticipantKind::Actor) => format!("actor {pid} as {l}"),
            Some(ParticipantKind::Database) => {
                format!("participant {pid}@{{ \"type\": \"database\" }} as {l}")
            }
            Some(ParticipantKind::Queue) => {
                format!("participant {pid}@{{ \"type\": \"queue\" }} as {l}")
            }
            _ => format!("participant {pid} as {l}"),
        };
        let _ = writeln!(out, "    {line}");
    }
    items(&s.items, 1, &id, &mut out);
    out
}

fn items(list: &[Item], depth: usize, id: &dyn Fn(&str) -> String, out: &mut String) {
    let pad = "    ".repeat(depth);
    for it in list {
        match it {
            Item::Message(m) => {
                let arrow = match m.kind() {
                    MessageKind::Sync => "->>",
                    MessageKind::Async => "-)",
                    MessageKind::Return => "-->>",
                };
                let _ = writeln!(
                    out,
                    "{pad}{}{arrow}{}: {}",
                    id(&m.from),
                    id(&m.to),
                    escape_label(&m.label)
                );
            }
            Item::Note(n) => {
                let over: Vec<String> = n.over.iter().map(|p| id(p)).collect();
                let _ = writeln!(
                    out,
                    "{pad}Note over {}: {}",
                    over.join(","),
                    escape_label(&n.text)
                );
            }
            Item::Group(g) => {
                let (kw, sep) = match g.kind {
                    GroupKind::Alt => ("alt", "else"),
                    GroupKind::Opt => ("opt", "else"),
                    GroupKind::Loop => ("loop", "else"),
                    GroupKind::Par => ("par", "and"),
                };
                // Mermaid has no frame title separate from the first branch's condition, so
                // a first-branch label is appended to the group label ("label: branch").
                let first = g
                    .branches
                    .first()
                    .and_then(|b| b.label.as_deref())
                    .filter(|l| !l.is_empty());
                let header = match first {
                    Some(l) => format!("{}: {}", g.label, l),
                    None => g.label.clone(),
                };
                let _ = writeln!(out, "{pad}{kw} {}", escape_label(&header));
                if g.branches.is_empty() {
                    items(&g.items, depth + 1, id, out);
                }
                for (i, b) in g.branches.iter().enumerate() {
                    if i > 0 {
                        let _ = match b.label.as_deref().filter(|l| !l.is_empty()) {
                            Some(l) => writeln!(out, "{pad}{sep} {}", escape_label(l)),
                            None => writeln!(out, "{pad}{sep}"),
                        };
                    }
                    items(&b.items, depth + 1, id, out);
                }
                let _ = writeln!(out, "{pad}end");
            }
        }
    }
}
