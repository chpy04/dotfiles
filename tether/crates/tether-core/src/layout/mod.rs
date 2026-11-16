//! Diagram layout: a pure function from a diagram spec plus UI state (which objects are
//! expanded) to **terminal-cell geometry** that a ratatui viewer draws directly (PRD F3.5 and
//! "Rendering: a viewer where every node is an object").
//!
//! ```no_run
//! use std::collections::HashSet;
//! use tether_core::diagram::{validate, Diagram};
//! use tether_core::layout::{layout, LayoutInput};
//! # fn run(json: &str) {
//! let diagram: Diagram = validate(json).unwrap();
//! let expanded = HashSet::new();
//! let l = layout(&LayoutInput::new(&diagram, &expanded)).unwrap();
//! println!("{}", tether_core::layout::ascii_debug(&l));
//! # }
//! ```
//!
//! # Coordinates
//!
//! Everything is in terminal cells with the origin at the top-left of the diagram canvas
//! (`0,0`). All rects and points are non-negative and inside `width` x `height`. The viewer
//! scrolls/clips this canvas itself.
//!
//! # Object model
//!
//! [`Layout::objects`] is in **paint order** (group frames first, then nodes, participants,
//! notes and messages, then details panels, then edge labels). Every object has a unique
//! [`ObjectId`]:
//!
//! | object | id |
//! | --- | --- |
//! | flowchart node, sequence participant, message, note | the spec id |
//! | group frame (sequence) | `group:N` (N = 1-based, document order) |
//! | details panel | `<owner id>#details` |
//! | flowchart edge ([`EdgeGeom::id`]) | `edge:I` (I = index into the spec's `edges`) |
//! | flowchart edge label | `edge:I:label` |
//!
//! # Flowchart geometry
//!
//! A node is a 1-cell bordered rounded box. Inside (left to right): 1 pad, an icon cell, a
//! space, the label (left aligned), optional code marker, optional `+N` badge, 1 pad. The exact
//! cells are given in [`Object::label_rect`] and [`Decor`]. Edge polylines are lists of
//! orthogonal corner points; consecutive points differ in exactly one coordinate. The first
//! point is the cell just outside the source's border, the last point is the cell just outside
//! the target's border and holds the arrowhead ([`EdgeGeom::arrow`]). Use [`edge_cells`] to get
//! per-cell direction masks for picking box-drawing glyphs.
//!
//! # Sequence geometry
//!
//! One column per participant ([`Lifeline`]), one row band per message (label rows above the
//! arrow row), notes spanning their `over` participants, group frames enclosing their rows.
//! A details panel sits directly under its owner's last row; every later row shifts down by
//! exactly the panel height, and lifelines are interrupted over the panel rows
//! ([`Lifeline::breaks`]).
//!
//! # Navigation
//!
//! [`Layout::nav`] carries the precomputed j/k/h/l graph, see [`FlowNav`] and [`SeqNav`].

mod ascii;
mod common;
mod dotgraph;
mod flow;
mod seq;
mod text;

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::Serialize;

use crate::diagram::{
    Diagram, DiagramKind, EdgeStyle, GroupKind, MessageKind, ParticipantKind, Shape,
};

pub use ascii::ascii_debug;
pub use text::{char_width, str_width, wrap};

/// Cell width in inches used to talk to Graphviz (10 columns per inch).
pub const CELL_W_IN: f64 = 0.1;
/// Cell height in inches used to talk to Graphviz. Twice [`CELL_W_IN`]: a terminal cell is
/// about twice as tall as it is wide.
pub const CELL_H_IN: f64 = 0.2;

// --------------------------------------------------------------------------- ids

/// Stable identifier of anything the viewer can focus, draw or hit-test.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ObjectId(pub String);

impl ObjectId {
    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ObjectId {
    fn from(s: &str) -> Self {
        ObjectId(s.to_string())
    }
}

impl From<String> for ObjectId {
    fn from(s: String) -> Self {
        ObjectId(s)
    }
}

impl std::borrow::Borrow<str> for ObjectId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ObjectId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Id of the details panel attached to `owner`.
pub fn details_id(owner: &str) -> ObjectId {
    ObjectId(format!("{owner}#details"))
}

// --------------------------------------------------------------------------- geometry

/// A cell coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Point {
    /// Column.
    pub x: u16,
    /// Row.
    pub y: u16,
}

/// A rectangle of cells. `w` and `h` count cells; the rect covers `x..x+w` by `y..y+h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Rect {
    /// Left column.
    pub x: u16,
    /// Top row.
    pub y: u16,
    /// Width in cells.
    pub w: u16,
    /// Height in cells.
    pub h: u16,
}

impl Rect {
    /// Exclusive right edge.
    pub fn right(&self) -> u16 {
        self.x + self.w
    }

    /// Exclusive bottom edge.
    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }

    /// True when the two rects share at least one cell.
    pub fn intersects(&self, o: &Rect) -> bool {
        self.w > 0
            && self.h > 0
            && o.w > 0
            && o.h > 0
            && self.x < o.right()
            && o.x < self.right()
            && self.y < o.bottom()
            && o.y < self.bottom()
    }

    /// True when the cell is inside the rect.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// True when `o` lies completely inside `self`.
    pub fn contains_rect(&self, o: &Rect) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }

    /// The centre cell (rounded towards the top-left).
    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w.saturating_sub(1) / 2,
            y: self.y + self.h.saturating_sub(1) / 2,
        }
    }
}

/// Direction of an arrowhead (the direction the line is travelling when it ends).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Dir {
    /// Towards smaller `y`.
    Up,
    /// Towards larger `y`.
    Down,
    /// Towards smaller `x`.
    Left,
    /// Towards larger `x`.
    Right,
}

// --------------------------------------------------------------------------- output types

/// What an [`Object`] is, with the kind-specific data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ObjectKind {
    /// A flowchart node.
    Node {
        /// Shape (the viewer picks the icon from it).
        shape: Shape,
    },
    /// A sequence participant box.
    Participant {
        /// Kind, if the spec gave one.
        kind: Option<ParticipantKind>,
    },
    /// A sequence message. `rect` covers the label rows and the arrow row.
    Message {
        /// Arrow kind.
        kind: MessageKind,
        /// Sender participant.
        from: ObjectId,
        /// Receiver participant.
        to: ObjectId,
    },
    /// A sequence note.
    Note {
        /// Participants it spans.
        over: Vec<ObjectId>,
    },
    /// A group frame (alt/opt/loop/par). The header text sits on the top border row.
    Group {
        /// Group kind.
        kind: GroupKind,
        /// Nesting depth, 0 for top level.
        depth: u16,
        /// Branch separators/headers, top to bottom (empty for opt/loop).
        branches: Vec<BranchRow>,
    },
    /// An expanded details panel; see [`Object::sections`] and [`Object::owner`].
    Details,
    /// The label of a flowchart edge; [`Object::owner`] is the edge id.
    EdgeLabel,
}

/// One branch header/separator row inside a group frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchRow {
    /// Branch label (the `else` condition).
    pub label: Option<String>,
    /// Absolute row of the header.
    pub y: u16,
    /// True for a dashed separator across the frame (every branch but a first one), false
    /// for a plain label row under the top border.
    pub separator: bool,
}

/// One title/body section of a details panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    /// Title (drawn bold). `CODE` for the linked code ref section.
    pub title: String,
    /// Wrapped body lines.
    pub body: Vec<String>,
    /// Absolute row of the title; body lines follow on the next rows.
    pub y: u16,
}

/// Positions of the small decorations inside an object (top-left cell of each).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Decor {
    /// Shape/kind icon cell (nodes and participants).
    pub icon: Option<Point>,
    /// Two-cell code marker (`has_code`); the viewer draws any glyph that fits.
    pub code: Option<Point>,
    /// Start of the `+N` badge text (`details_count > 0`); width is `badge_width`.
    pub badge: Option<Point>,
}

/// Anything the viewer draws and focuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Object {
    /// Unique id.
    pub id: ObjectId,
    /// Kind and kind-specific data.
    pub kind: ObjectKind,
    /// Bounding rect in cells.
    pub rect: Rect,
    /// Where the label lines are drawn (left aligned, one line per row). `None` for panels
    /// and edge-less objects without text.
    pub label_rect: Option<Rect>,
    /// Label already wrapped to lines.
    pub label: Vec<String>,
    /// Number of spec `details` (for the `+N` badge). 0 on panels and labels.
    pub details_count: usize,
    /// True when the spec element has a code link.
    pub has_code: bool,
    /// True when this element's details are currently expanded.
    pub expanded: bool,
    /// For details panels the owning object id, for edge labels the edge id.
    pub owner: Option<ObjectId>,
    /// Sections of a details panel (empty otherwise).
    pub sections: Vec<Section>,
    /// Icon/code/badge cells.
    pub decor: Decor,
    /// Innermost enclosing group frame (sequence only).
    pub group: Option<ObjectId>,
}

/// Geometry of one flowchart edge or one sequence message arrow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EdgeGeom {
    /// `edge:I` for flowcharts; the message id for sequences.
    pub id: ObjectId,
    /// Source node or participant.
    pub from: ObjectId,
    /// Target node or participant.
    pub to: ObjectId,
    /// Orthogonal polyline in cells; consecutive points share a row or a column.
    pub points: Vec<Point>,
    /// Arrowhead at the last point.
    pub arrow: Dir,
    /// Solid or dashed.
    pub style: EdgeStyle,
    /// Message kind for sequence arrows.
    pub msg_kind: Option<MessageKind>,
    /// True when the edge points against the flow (flowchart back edge).
    pub back: bool,
    /// Id of the label object, if the edge has a label.
    pub label: Option<ObjectId>,
    /// Rect of the label, if any.
    pub label_rect: Option<Rect>,
}

/// A participant lifeline (sequence only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lifeline {
    /// Participant id.
    pub participant: ObjectId,
    /// Column.
    pub x: u16,
    /// First row of the line (the row under the participant box).
    pub y0: u16,
    /// Last row of the line (inclusive).
    pub y1: u16,
    /// Inclusive row ranges where the line is interrupted (under a details panel or a note).
    pub breaks: Vec<(u16, u16)>,
}

/// Navigation graph for flowcharts. Every node id has an entry in each map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FlowNav {
    /// Outgoing neighbours, forward edges first in left-to-right (top-to-bottom for `LR`)
    /// order, then back-edge targets. `j` follows `children[id][0]` (or the remembered branch).
    pub children: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Incoming neighbours, same ordering rule (forward first, by position; back edges last).
    pub parents: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Other nodes of the same rank, ordered by position across the flow (left to right in
    /// `TB`). `h`/`l` step through the position-sorted [`FlowNav::ranks`] row.
    pub siblings: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Other children of any shared parent, ordered by position (the branch alternatives).
    pub branch_siblings: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Rank (0-based layer) of every node.
    pub rank: BTreeMap<ObjectId, usize>,
    /// Nodes per rank, ordered by position.
    pub ranks: Vec<Vec<ObjectId>>,
    /// Nodes without incoming forward edges, in reading order. Initial focus candidates.
    pub roots: Vec<ObjectId>,
    /// All nodes in reading order (rank, then position). For search and fallback movement.
    pub order: Vec<ObjectId>,
}

impl FlowNav {
    /// The neighbour of `id` in its rank row, `dir` = -1 (left/up) or +1 (right/down).
    pub fn sibling(&self, id: &str, dir: i32) -> Option<&ObjectId> {
        let r = *self.rank.get(id)?;
        let row = self.ranks.get(r)?;
        let i = row.iter().position(|o| o.as_str() == id)? as i32 + dir;
        if i < 0 {
            return None;
        }
        row.get(i as usize)
    }
}

/// Navigation graph for sequence diagrams.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SeqNav {
    /// Participants left to right (`h`/`l`).
    pub participants: Vec<ObjectId>,
    /// Messages top to bottom (`j`/`k` when notes are skipped).
    pub messages: Vec<ObjectId>,
    /// Messages and notes top to bottom, the focusable rows in document order.
    pub items: Vec<ObjectId>,
    /// Group frames in document order.
    pub groups: Vec<ObjectId>,
}

/// Navigation data, by diagram type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Nav {
    /// Flowchart navigation.
    Flow(FlowNav),
    /// Sequence navigation.
    Seq(SeqNav),
}

/// The result of [`layout`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Layout {
    /// Diagram type.
    pub kind: DiagramKind,
    /// Canvas width in cells.
    pub width: u16,
    /// Canvas height in cells.
    pub height: u16,
    /// Objects in paint order.
    pub objects: Vec<Object>,
    /// Flowchart edges or sequence message arrows (spec order).
    pub edges: Vec<EdgeGeom>,
    /// Sequence lifelines, left to right (empty for flowcharts).
    pub lifelines: Vec<Lifeline>,
    /// Navigation graph.
    pub nav: Nav,
    #[serde(skip)]
    index: BTreeMap<ObjectId, usize>,
}

impl Layout {
    /// Look an object up by id.
    pub fn object(&self, id: &str) -> Option<&Object> {
        self.index.get(id).map(|&i| &self.objects[i])
    }

    /// Look an edge up by id.
    pub fn edge(&self, id: &str) -> Option<&EdgeGeom> {
        self.edges.iter().find(|e| e.id.as_str() == id)
    }

    /// The details panel attached to `owner`, if it is expanded.
    pub fn details_of(&self, owner: &str) -> Option<&Object> {
        self.object(details_id(owner).as_str())
    }

    /// Flowchart navigation, if this is a flowchart.
    pub fn flow_nav(&self) -> Option<&FlowNav> {
        match &self.nav {
            Nav::Flow(n) => Some(n),
            Nav::Seq(_) => None,
        }
    }

    /// Sequence navigation, if this is a sequence diagram.
    pub fn seq_nav(&self) -> Option<&SeqNav> {
        match &self.nav {
            Nav::Seq(n) => Some(n),
            Nav::Flow(_) => None,
        }
    }

    pub(crate) fn build_index(&mut self) {
        self.index = self
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.id.clone(), i))
            .collect();
    }
}

// --------------------------------------------------------------------------- edge cell masks

/// Direction bit: the line leaves the cell upwards.
pub const UP: u8 = 1;
/// Direction bit: the line leaves the cell downwards.
pub const DOWN: u8 = 2;
/// Direction bit: the line leaves the cell to the left.
pub const LEFT: u8 = 4;
/// Direction bit: the line leaves the cell to the right.
pub const RIGHT: u8 = 8;

/// For every cell crossed by an edge polyline, the OR of the direction bits of all edges in
/// that cell (`UP | DOWN` is a vertical line, `DOWN | RIGHT` a corner, all four a crossing).
/// Arrowhead cells carry the bit pointing back along the edge only.
pub fn edge_cells(layout: &Layout) -> BTreeMap<(u16, u16), u8> {
    let mut m: BTreeMap<(u16, u16), u8> = BTreeMap::new();
    for e in &layout.edges {
        for (p, bits) in polyline_cells(&e.points) {
            *m.entry((p.x, p.y)).or_insert(0) |= bits;
        }
    }
    m
}

/// Cells of one polyline with direction bits. Used by [`edge_cells`] and the metrics.
pub fn polyline_cells(points: &[Point]) -> Vec<(Point, u8)> {
    let mut out: Vec<(Point, u8)> = Vec::new();
    let mut add = |p: Point, b: u8| {
        if let Some(e) = out.iter_mut().rev().find(|(q, _)| *q == p) {
            e.1 |= b;
        } else {
            out.push((p, b));
        }
    };
    if points.len() == 1 {
        add(points[0], 0);
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (
            (b.x as i32 - a.x as i32).signum(),
            (b.y as i32 - a.y as i32).signum(),
        );
        let (fwd, back) = match (dx, dy) {
            (1, 0) => (RIGHT, LEFT),
            (-1, 0) => (LEFT, RIGHT),
            (0, 1) => (DOWN, UP),
            (0, -1) => (UP, DOWN),
            _ => (0, 0),
        };
        let mut cur = a;
        add(cur, fwd);
        while cur != b {
            cur = Point {
                x: (cur.x as i32 + dx) as u16,
                y: (cur.y as i32 + dy) as u16,
            };
            add(cur, if cur == b { back } else { back | fwd });
        }
    }
    out
}

// --------------------------------------------------------------------------- input / errors

/// Which flowchart placement engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    /// Graphviz `dot -Tplain`; a missing or failing `dot` is a [`LayoutError`].
    #[default]
    Dot,
    /// Built-in naive layered placement (longest-path ranks, barycentre ordering).
    Naive,
    /// Try `dot`, fall back to the naive engine when it is missing or fails.
    Auto,
}

/// Layout tuning.
#[derive(Debug, Clone)]
pub struct LayoutOptions {
    /// Max node label width in cells before wrapping (default 28).
    pub max_label_width: u16,
    /// Max details panel width including border and padding (default 40).
    pub max_panel_width: u16,
    /// Max flowchart edge label width (default 20).
    pub max_edge_label_width: u16,
    /// Max sequence message label width before wrapping (default 40).
    pub max_message_label_width: u16,
    /// Placement engine for flowcharts.
    pub engine: Engine,
    /// The Graphviz executable (default `dot`, or `$TETHER_DOT` when set).
    pub dot_path: String,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions {
            max_label_width: 28,
            max_panel_width: 40,
            max_edge_label_width: 20,
            max_message_label_width: 40,
            engine: Engine::Dot,
            dot_path: std::env::var("TETHER_DOT").unwrap_or_else(|_| "dot".to_string()),
        }
    }
}

/// Input of [`layout`].
#[derive(Debug, Clone)]
pub struct LayoutInput<'a> {
    /// The (validated, normalized) diagram.
    pub diagram: &'a Diagram,
    /// Objects whose details are expanded (node, message or note ids).
    pub expanded: &'a HashSet<ObjectId>,
    /// Tuning.
    pub opts: LayoutOptions,
}

impl<'a> LayoutInput<'a> {
    /// Default options.
    pub fn new(diagram: &'a Diagram, expanded: &'a HashSet<ObjectId>) -> Self {
        LayoutInput {
            diagram,
            expanded,
            opts: LayoutOptions::default(),
        }
    }
}

/// Why layout failed.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    /// The Graphviz executable could not be started.
    #[error(
        "graphviz `{0}` not found or not executable ({1}); install graphviz or set TETHER_DOT"
    )]
    DotMissing(String, String),
    /// `dot` ran but failed.
    #[error("graphviz `dot` failed ({status}): {stderr}")]
    DotFailed {
        /// Exit status text.
        status: String,
        /// Captured stderr.
        stderr: String,
    },
    /// `dot -Tplain` output could not be understood.
    #[error("could not parse graphviz plain output: {0}")]
    DotParse(String),
    /// The spec references an id that does not exist.
    #[error("unknown id `{0}` in the diagram")]
    UnknownId(String),
    /// The spec reuses an id.
    #[error("duplicate id `{0}` in the diagram")]
    DuplicateId(String),
    /// The diagram does not fit in a `u16` cell canvas.
    #[error("diagram too large for a u16 cell canvas ({0})")]
    TooLarge(String),
}

/// Compute the layout of a diagram.
pub fn layout(input: &LayoutInput<'_>) -> Result<Layout, LayoutError> {
    match input.diagram {
        Diagram::Flowchart(f) => flow::layout_flowchart(f, input.expanded, &input.opts),
        Diagram::Sequence(s) => seq::layout_sequence(s, input.expanded, &input.opts),
    }
}

// --------------------------------------------------------------------------- internal raw types

/// Signed rect used while building (shifted to non-negative at the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        IRect { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn intersects(&self, o: &IRect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    fn shift(&self, dx: i32, dy: i32) -> Result<Rect, LayoutError> {
        Ok(Rect {
            x: cv(self.x + dx)?,
            y: cv(self.y + dy)?,
            w: cv(self.w)?,
            h: cv(self.h)?,
        })
    }
}

fn cv(v: i32) -> Result<u16, LayoutError> {
    u16::try_from(v).map_err(|_| LayoutError::TooLarge(format!("coordinate {v}")))
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawDecor {
    pub icon: Option<(i32, i32)>,
    pub code: Option<(i32, i32)>,
    pub badge: Option<(i32, i32)>,
}

pub(crate) struct RawSection {
    pub title: String,
    pub body: Vec<String>,
    pub y: i32,
}

pub(crate) struct RawBranch {
    pub label: Option<String>,
    pub y: i32,
    pub separator: bool,
}

pub(crate) enum RawKind {
    Node(Shape),
    Participant(Option<ParticipantKind>),
    Message(MessageKind, ObjectId, ObjectId),
    Note(Vec<ObjectId>),
    Group(GroupKind, u16, Vec<RawBranch>),
    Details,
    EdgeLabel,
}

pub(crate) struct RawObject {
    pub id: ObjectId,
    pub kind: RawKind,
    pub rect: IRect,
    pub label_rect: Option<IRect>,
    pub label: Vec<String>,
    pub details_count: usize,
    pub has_code: bool,
    pub expanded: bool,
    pub owner: Option<ObjectId>,
    pub sections: Vec<RawSection>,
    pub decor: RawDecor,
    pub group: Option<ObjectId>,
}

impl RawObject {
    pub fn new(id: ObjectId, kind: RawKind, rect: IRect) -> Self {
        RawObject {
            id,
            kind,
            rect,
            label_rect: None,
            label: Vec::new(),
            details_count: 0,
            has_code: false,
            expanded: false,
            owner: None,
            sections: Vec::new(),
            decor: RawDecor::default(),
            group: None,
        }
    }
}

pub(crate) struct RawEdge {
    pub id: ObjectId,
    pub from: ObjectId,
    pub to: ObjectId,
    pub points: Vec<(i32, i32)>,
    pub arrow: Dir,
    pub style: EdgeStyle,
    pub msg_kind: Option<MessageKind>,
    pub back: bool,
    pub label: Option<ObjectId>,
    pub label_rect: Option<IRect>,
}

pub(crate) struct RawLifeline {
    pub participant: ObjectId,
    pub x: i32,
    pub y0: i32,
    pub y1: i32,
    pub breaks: Vec<(i32, i32)>,
}

/// Shift everything so the minimum coordinate is 0, size the canvas and convert to the public
/// types.
pub(crate) fn finish(
    kind: DiagramKind,
    objects: Vec<RawObject>,
    edges: Vec<RawEdge>,
    lifelines: Vec<RawLifeline>,
    nav: Nav,
    min_size: (i32, i32),
) -> Result<Layout, LayoutError> {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    let grow_rect = |r: &IRect, mn: &mut (i32, i32), mx: &mut (i32, i32)| {
        mn.0 = mn.0.min(r.x);
        mn.1 = mn.1.min(r.y);
        mx.0 = mx.0.max(r.right());
        mx.1 = mx.1.max(r.bottom());
    };
    let mut mn = (min_x, min_y);
    let mut mx = (max_x, max_y);
    for o in &objects {
        grow_rect(&o.rect, &mut mn, &mut mx);
    }
    for e in &edges {
        for &(x, y) in &e.points {
            grow_rect(&IRect::new(x, y, 1, 1), &mut mn, &mut mx);
        }
        if let Some(r) = &e.label_rect {
            grow_rect(r, &mut mn, &mut mx);
        }
    }
    for l in &lifelines {
        grow_rect(&IRect::new(l.x, l.y0, 1, l.y1 - l.y0 + 1), &mut mn, &mut mx);
    }
    if mn.0 == i32::MAX {
        mn = (0, 0);
    }
    min_x = mn.0;
    min_y = mn.1;
    max_x = max_x.max(mx.0);
    max_y = max_y.max(mx.1);
    let (dx, dy) = (-min_x, -min_y);
    let width = (max_x + dx).max(min_size.0);
    let height = (max_y + dy).max(min_size.1);

    let pt = |p: &(i32, i32)| -> Result<Point, LayoutError> {
        Ok(Point {
            x: cv(p.0 + dx)?,
            y: cv(p.1 + dy)?,
        })
    };
    let sh = |r: &IRect| r.shift(dx, dy);
    let shp = |p: &(i32, i32)| -> Result<Point, LayoutError> { pt(p) };

    let mut out_objects = Vec::with_capacity(objects.len());
    for o in objects {
        let kind = match o.kind {
            RawKind::Node(shape) => ObjectKind::Node { shape },
            RawKind::Participant(k) => ObjectKind::Participant { kind: k },
            RawKind::Message(k, f, t) => ObjectKind::Message {
                kind: k,
                from: f,
                to: t,
            },
            RawKind::Note(over) => ObjectKind::Note { over },
            RawKind::Group(k, depth, br) => ObjectKind::Group {
                kind: k,
                depth,
                branches: br
                    .into_iter()
                    .map(|b| {
                        Ok(BranchRow {
                            label: b.label,
                            y: cv(b.y + dy)?,
                            separator: b.separator,
                        })
                    })
                    .collect::<Result<_, LayoutError>>()?,
            },
            RawKind::Details => ObjectKind::Details,
            RawKind::EdgeLabel => ObjectKind::EdgeLabel,
        };
        out_objects.push(Object {
            id: o.id,
            kind,
            rect: sh(&o.rect)?,
            label_rect: o.label_rect.as_ref().map(sh).transpose()?,
            label: o.label,
            details_count: o.details_count,
            has_code: o.has_code,
            expanded: o.expanded,
            owner: o.owner,
            sections: o
                .sections
                .into_iter()
                .map(|s| {
                    Ok(Section {
                        title: s.title,
                        body: s.body,
                        y: cv(s.y + dy)?,
                    })
                })
                .collect::<Result<_, LayoutError>>()?,
            decor: Decor {
                icon: o.decor.icon.as_ref().map(shp).transpose()?,
                code: o.decor.code.as_ref().map(shp).transpose()?,
                badge: o.decor.badge.as_ref().map(shp).transpose()?,
            },
            group: o.group,
        });
    }
    let mut out_edges = Vec::with_capacity(edges.len());
    for e in edges {
        out_edges.push(EdgeGeom {
            id: e.id,
            from: e.from,
            to: e.to,
            points: e.points.iter().map(pt).collect::<Result<_, _>>()?,
            arrow: e.arrow,
            style: e.style,
            msg_kind: e.msg_kind,
            back: e.back,
            label: e.label,
            label_rect: e.label_rect.as_ref().map(sh).transpose()?,
        });
    }
    let mut out_lifelines = Vec::new();
    for l in lifelines {
        out_lifelines.push(Lifeline {
            participant: l.participant,
            x: cv(l.x + dx)?,
            y0: cv(l.y0 + dy)?,
            y1: cv(l.y1 + dy)?,
            breaks: l
                .breaks
                .iter()
                .map(|&(a, b)| Ok((cv(a + dy)?, cv(b + dy)?)))
                .collect::<Result<_, LayoutError>>()?,
        });
    }
    let mut layout = Layout {
        kind,
        width: cv(width)?,
        height: cv(height)?,
        objects: out_objects,
        edges: out_edges,
        lifelines: out_lifelines,
        nav,
        index: BTreeMap::new(),
    };
    layout.build_index();
    Ok(layout)
}
