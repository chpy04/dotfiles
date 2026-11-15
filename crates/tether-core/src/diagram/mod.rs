//! Code-linked diagrams: spec types, JSON Schema, validation, Mermaid export,
//! the on-disk store and anchor re-resolution (PRD Feature 3).

pub mod anchor;
pub mod mermaid;
pub mod schema;
pub mod spec;
pub mod store;
pub mod validate;

pub use anchor::{resolve, resolve_with_window, Resolved, DEFAULT_WINDOW};
pub use mermaid::to_mermaid;
pub use spec::*;
pub use store::{DiagramInfo, DiagramStore, StoreError};
pub use validate::{format_errors, normalize, validate, validate_diagram, ValidationError};
