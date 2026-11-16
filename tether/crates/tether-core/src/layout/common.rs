//! Pieces shared by the flowchart and sequence layouts: badge/code adornments and details
//! panels.

use crate::diagram::{CodeRef, Detail};

use super::text::{max_width, wrap};
use super::{details_id, IRect, ObjectId, RawDecor, RawKind, RawObject, RawSection};

/// Cells taken by the `+N` badge text.
pub(crate) fn badge_len(n: usize) -> i32 {
    1 + n.to_string().len() as i32
}

/// Cells reserved to the right of a label: ` +N` for details and ` <glyph>` for a code link.
pub(crate) fn adorn_width(details: usize, has_code: bool) -> i32 {
    (if details > 0 {
        1 + badge_len(details)
    } else {
        0
    }) + if has_code { 2 } else { 0 }
}

/// `path:line[-end]`.
pub(crate) fn code_text(c: &CodeRef) -> String {
    match c.end_line {
        Some(e) => format!("{}:{}-{}", c.path, c.line, e),
        None => format!("{}:{}", c.path, c.line),
    }
}

/// Size and content of a details panel.
#[derive(Debug, Clone)]
pub(crate) struct PanelSpec {
    pub w: i32,
    pub h: i32,
    pub sections: Vec<(String, Vec<String>)>,
}

/// Build a panel for `details` (plus a `CODE` section when `code` is given). `None` when there
/// are no details: an element with only a code link has nothing to expand.
pub(crate) fn build_panel(
    details: &[Detail],
    code: Option<&CodeRef>,
    max_w: u16,
) -> Option<PanelSpec> {
    if details.is_empty() {
        return None;
    }
    let inner_max = (max_w as usize).saturating_sub(4).max(8);
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for d in details {
        sections.push((d.title.clone(), wrap(&d.body, inner_max)));
    }
    if let Some(c) = code {
        sections.push(("CODE".to_string(), wrap(&code_text(c), inner_max)));
    }
    let mut inner = 0usize;
    let mut rows = 0i32;
    for (t, body) in &sections {
        inner = inner.max(super::text::str_width(t)).max(max_width(body));
        rows += 1 + body.len() as i32;
    }
    Some(PanelSpec {
        w: (inner as i32 + 4).max(14),
        h: rows + 2,
        sections,
    })
}

/// Turn a panel spec into an object at `(x, y)`.
pub(crate) fn panel_object(
    owner: &str,
    spec: &PanelSpec,
    x: i32,
    y: i32,
    group: Option<ObjectId>,
) -> RawObject {
    let mut o = RawObject::new(
        details_id(owner),
        RawKind::Details,
        IRect::new(x, y, spec.w, spec.h),
    );
    o.owner = Some(ObjectId::from(owner));
    o.expanded = true;
    o.group = group;
    let mut row = y + 1;
    for (t, body) in &spec.sections {
        o.sections.push(RawSection {
            title: t.clone(),
            body: body.clone(),
            y: row,
        });
        row += 1 + body.len() as i32;
    }
    o
}

/// Decoration cells of a box-shaped object occupying `rect`, with the first label row at `y`.
/// Icon at `x+2`; badge and code glyph packed against the right padding cell.
pub(crate) fn box_decor(
    rect: IRect,
    y: i32,
    details: usize,
    has_code: bool,
    icon: bool,
) -> RawDecor {
    let mut d = RawDecor::default();
    if icon {
        d.icon = Some((rect.x + 2, y));
    }
    let mut end = rect.right() - 2;
    if details > 0 {
        let bl = badge_len(details);
        d.badge = Some((end - bl, y));
        end = end - bl - 1;
    }
    if has_code {
        d.code = Some((end - 1, y));
    }
    d
}
