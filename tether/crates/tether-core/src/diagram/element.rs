//! Lookup of the "things you can point at" in a diagram (nodes, participants, messages, notes)
//! by id, with the label and code link the viewer, `tether send --diagram` and the search need.

use super::spec::{CodeRef, Diagram, Item};

/// What kind of element an id refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    /// Flowchart node.
    Node,
    /// Sequence participant.
    Participant,
    /// Sequence message.
    Message,
    /// Sequence note.
    Note,
}

impl ElementKind {
    /// Word used in comment payloads and messages (`node`, `participant`, `message`, `note`).
    pub fn word(self) -> &'static str {
        match self {
            ElementKind::Node => "node",
            ElementKind::Participant => "participant",
            ElementKind::Message => "message",
            ElementKind::Note => "note",
        }
    }
}

/// One focusable diagram element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Spec id (message and note ids are the normalized `m1`/`n1` ones).
    pub id: String,
    /// Kind.
    pub kind: ElementKind,
    /// Node label, participant label, message label or note text.
    pub label: String,
    /// Linked code, if any.
    pub code: Option<CodeRef>,
    /// Number of `details` entries.
    pub details: usize,
}

/// All elements of a diagram in document order (participants first for sequences).
pub fn elements(d: &Diagram) -> Vec<Element> {
    let mut out = Vec::new();
    match d {
        Diagram::Flowchart(f) => {
            for n in &f.nodes {
                out.push(Element {
                    id: n.id.clone(),
                    kind: ElementKind::Node,
                    label: n.label.clone(),
                    code: n.code.clone(),
                    details: n.details.len(),
                });
            }
        }
        Diagram::Sequence(s) => {
            let mut s = s.clone();
            s.assign_ids();
            for p in &s.participants {
                out.push(Element {
                    id: p.id.clone(),
                    kind: ElementKind::Participant,
                    label: p.label.clone(),
                    code: None,
                    details: 0,
                });
            }
            s.walk(&mut |it| match it {
                Item::Message(m) => out.push(Element {
                    id: m.id.clone(),
                    kind: ElementKind::Message,
                    label: m.label.clone(),
                    code: m.code.clone(),
                    details: m.details.len(),
                }),
                Item::Note(n) => out.push(Element {
                    id: n.id.clone(),
                    kind: ElementKind::Note,
                    label: n.text.clone(),
                    code: None,
                    details: n.details.len(),
                }),
                Item::Group(_) => {}
            });
        }
    }
    out
}

/// Find one element by id.
pub fn find_element(d: &Diagram, id: &str) -> Option<Element> {
    elements(d).into_iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::validate;

    #[test]
    fn finds_nodes_and_sequence_items_with_assigned_ids() {
        let f = validate(
            r#"{"type":"flowchart","title":"T","nodes":[{"id":"a","label":"A","code":{"path":"x.rs","line":3}}]}"#,
        )
        .unwrap();
        let e = find_element(&f, "a").unwrap();
        assert_eq!((e.kind, e.label.as_str(), e.code.unwrap().line), (ElementKind::Node, "A", 3));
        assert!(find_element(&f, "zz").is_none());

        let s = validate(
            r#"{"type":"sequence","title":"S","participants":[{"id":"p","label":"P"},{"id":"q","label":"Q"}],
               "items":[{"type":"message","from":"p","to":"q","label":"hi"},{"type":"note","over":["p"],"text":"n"}]}"#,
        )
        .unwrap();
        assert_eq!(find_element(&s, "m1").unwrap().kind, ElementKind::Message);
        assert_eq!(find_element(&s, "n1").unwrap().label, "n");
        assert_eq!(find_element(&s, "q").unwrap().kind.word(), "participant");
        assert_eq!(elements(&s).len(), 4);
    }
}
