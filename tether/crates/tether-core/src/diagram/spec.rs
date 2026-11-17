//! Serde types for the diagram spec (flowchart and sequence).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A link from a diagram element to source code. Paths are relative to the worktree root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeRef {
    /// File path relative to the worktree root (no leading `/`, no `..`).
    #[schemars(length(min = 1))]
    pub path: String,
    /// 1-based line number.
    #[schemars(range(min = 1, max = 10_000_000))]
    pub line: u32,
    /// Optional 1-based last line of a range; must be >= `line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 10_000_000))]
    pub end_line: Option<u32>,
    /// Optional snippet of the source line, used to re-find the line after the code shifts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Optional git revision (branch, tag or commit, e.g. `main`) that Neovim's git signs and file
    /// tree compare against when the link is opened. Omit it to open the file normally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub diff: Option<String>,
}

/// An expandable detail attached to a node, message or note (data model, edge case, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    /// Short heading, e.g. "Edge case".
    #[schemars(length(min = 1))]
    pub title: String,
    /// Body text.
    pub body: String,
}

/// A diagram spec, tagged by `"type"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Diagram {
    /// A flowchart.
    Flowchart(Flowchart),
    /// A sequence diagram.
    Sequence(Sequence),
}

impl Diagram {
    /// The diagram title.
    pub fn title(&self) -> &str {
        match self {
            Diagram::Flowchart(f) => &f.title,
            Diagram::Sequence(s) => &s.title,
        }
    }

    /// Replace the diagram title (the caller validates it is not empty).
    pub fn set_title(&mut self, title: impl Into<String>) {
        match self {
            Diagram::Flowchart(f) => f.title = title.into(),
            Diagram::Sequence(s) => s.title = title.into(),
        }
    }

    /// The `"type"` tag as a string (`flowchart` or `sequence`).
    pub fn kind(&self) -> DiagramKind {
        match self {
            Diagram::Flowchart(_) => DiagramKind::Flowchart,
            Diagram::Sequence(_) => DiagramKind::Sequence,
        }
    }
}

/// The two supported diagram types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagramKind {
    /// Flowchart.
    Flowchart,
    /// Sequence diagram.
    Sequence,
}

impl DiagramKind {
    /// Lowercase name, as used in the `"type"` field.
    pub fn as_str(self) -> &'static str {
        match self {
            DiagramKind::Flowchart => "flowchart",
            DiagramKind::Sequence => "sequence",
        }
    }
}

// ---------------------------------------------------------------- flowchart

/// Flowchart layout direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Direction {
    /// Top to bottom (default).
    #[default]
    TB,
    /// Left to right.
    LR,
}

/// Node shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Rectangle (default).
    #[default]
    Process,
    /// Diamond.
    Decision,
    /// Start/end, rounded pill.
    Terminal,
    /// Input/output parallelogram.
    Io,
    /// Predefined process, double-sided rectangle.
    Subroutine,
    /// Cylinder.
    Database,
}

/// Edge line style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EdgeStyle {
    /// Solid line (default).
    #[default]
    Solid,
    /// Dashed line.
    Dashed,
}

/// A flowchart spec (the `"type"` tag lives on [`Diagram`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Flowchart {
    /// Diagram title; also names the tab and the stored id.
    #[schemars(length(min = 1))]
    pub title: String,
    /// Layout direction, `TB` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// Nodes (at least one).
    #[schemars(length(min = 1))]
    pub nodes: Vec<Node>,
    /// Edges between node ids.
    #[serde(default)]
    pub edges: Vec<Edge>,
}

impl Flowchart {
    /// The effective direction (`TB` when unset).
    pub fn direction(&self) -> Direction {
        self.direction.unwrap_or_default()
    }
}

/// A flowchart node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Unique id within the diagram: letters, digits, `_`, `.` and `-` only.
    #[schemars(length(min = 1), regex(pattern = r"^[A-Za-z0-9_.-]+$"))]
    pub id: String,
    /// Text shown in the node.
    #[schemars(length(min = 1))]
    pub label: String,
    /// Shape, `process` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    /// Linked source code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeRef>,
    /// Expandable details.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Detail>,
}

impl Node {
    /// The effective shape (`process` when unset).
    pub fn shape(&self) -> Shape {
        self.shape.unwrap_or_default()
    }
}

/// A directed flowchart edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Optional edge label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Line style, `solid` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<EdgeStyle>,
}

impl Edge {
    /// The effective style (`solid` when unset).
    pub fn style(&self) -> EdgeStyle {
        self.style.unwrap_or_default()
    }
}

// ----------------------------------------------------------------- sequence

/// Participant kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ParticipantKind {
    /// A person.
    Actor,
    /// A service or component.
    Service,
    /// A database.
    Database,
    /// A queue or bus.
    Queue,
    /// A third-party system.
    External,
}

/// Message kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    /// Synchronous call (default).
    #[default]
    Sync,
    /// Asynchronous call.
    Async,
    /// Return, drawn dashed.
    Return,
}

/// Group kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    /// Alternatives (use `branches`).
    Alt,
    /// Optional (use `items`).
    Opt,
    /// Loop (use `items`).
    Loop,
    /// Parallel (use `branches`, or `items` for a single lane).
    Par,
}

/// A sequence diagram spec (the `"type"` tag lives on [`Diagram`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    /// Diagram title.
    #[schemars(length(min = 1))]
    pub title: String,
    /// Participants, drawn left to right in this order (at least one).
    #[schemars(length(min = 1))]
    pub participants: Vec<Participant>,
    /// Ordered messages, notes and groups (at least one).
    #[schemars(length(min = 1))]
    pub items: Vec<Item>,
}

/// A sequence participant (lifeline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    /// Unique id within the diagram, shared with message and note ids: letters, digits, `_`, `.`
    /// and `-` only.
    #[schemars(length(min = 1), regex(pattern = r"^[A-Za-z0-9_.-]+$"))]
    pub id: String,
    /// Display name.
    #[schemars(length(min = 1))]
    pub label: String,
    /// Kind, controls the icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ParticipantKind>,
}

/// One entry of a sequence: a message, a note or a group, tagged by `"type"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Item {
    /// A message between two participants.
    Message(Message),
    /// A note over one or more participants.
    Note(Note),
    /// A group (alt/opt/loop/par) holding nested items.
    Group(Group),
}

/// A message arrow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// Stable id; assigned as `m1`, `m2`, ... at load time when omitted. Shares one namespace
    /// with participant and note ids; letters, digits, `_`, `.` and `-` only.
    #[serde(default)]
    #[schemars(regex(pattern = r"^[A-Za-z0-9_.-]+$"))]
    pub id: String,
    /// Sender participant id.
    pub from: String,
    /// Receiver participant id.
    pub to: String,
    /// Arrow label.
    #[schemars(length(min = 1))]
    pub label: String,
    /// Arrow kind, `sync` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MessageKind>,
    /// Linked source code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeRef>,
    /// Expandable details.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Detail>,
}

impl Message {
    /// The effective kind (`sync` when unset).
    pub fn kind(&self) -> MessageKind {
        self.kind.unwrap_or_default()
    }
}

/// A note over one or more participants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// Stable id; assigned as `n1`, `n2`, ... at load time when omitted. Shares one namespace
    /// with participant and message ids; letters, digits, `_`, `.` and `-` only.
    #[serde(default)]
    #[schemars(regex(pattern = r"^[A-Za-z0-9_.-]+$"))]
    pub id: String,
    /// Participant ids the note spans (at least one).
    #[schemars(length(min = 1))]
    pub over: Vec<String>,
    /// Note text.
    #[schemars(length(min = 1))]
    pub text: String,
    /// Expandable details.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Detail>,
}

/// A group of items: `alt`/`par` use `branches`, `opt`/`loop` use `items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// Group kind.
    pub kind: GroupKind,
    /// Group label, e.g. the condition.
    #[schemars(length(min = 1))]
    pub label: String,
    /// Branches (alt, par). Mutually exclusive with `items`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<Branch>,
    /// Nested items (opt, loop, single-lane par). Mutually exclusive with `branches`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
}

/// One branch of an `alt` or `par` group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    /// Branch label (the `else` condition); optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Items in this branch (at least one).
    #[schemars(length(min = 1))]
    pub items: Vec<Item>,
}

impl Sequence {
    /// Visit every item depth-first in document order, including items nested in groups.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Item)) {
        fn go<'a>(items: &'a [Item], f: &mut impl FnMut(&'a Item)) {
            for it in items {
                f(it);
                if let Item::Group(g) = it {
                    go(&g.items, f);
                    for b in &g.branches {
                        go(&b.items, f);
                    }
                }
            }
        }
        go(&self.items, f);
    }

    /// Assign deterministic ids (`m1`.., `n1`..) to messages and notes lacking one,
    /// in depth-first document order, skipping ids already in use (including participant ids).
    pub fn assign_ids(&mut self) {
        let mut used: std::collections::HashSet<String> =
            self.participants.iter().map(|p| p.id.clone()).collect();
        self.walk(&mut |it| match it {
            Item::Message(m) if !m.id.is_empty() => {
                used.insert(m.id.clone());
            }
            Item::Note(n) if !n.id.is_empty() => {
                used.insert(n.id.clone());
            }
            _ => {}
        });
        fn go(
            items: &mut [Item],
            used: &mut std::collections::HashSet<String>,
            mc: &mut u32,
            nc: &mut u32,
        ) {
            fn fresh(used: &mut std::collections::HashSet<String>, c: &mut u32, p: &str) -> String {
                loop {
                    *c += 1;
                    let id = format!("{p}{c}");
                    if used.insert(id.clone()) {
                        return id;
                    }
                }
            }
            for it in items {
                match it {
                    Item::Message(m) if m.id.is_empty() => m.id = fresh(used, mc, "m"),
                    Item::Note(n) if n.id.is_empty() => n.id = fresh(used, nc, "n"),
                    Item::Group(g) => {
                        go(&mut g.items, used, mc, nc);
                        for b in &mut g.branches {
                            go(&mut b.items, used, mc, nc);
                        }
                    }
                    _ => {}
                }
            }
        }
        go(&mut self.items, &mut used, &mut 0, &mut 0);
    }
}
