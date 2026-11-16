//! JSON Schemas for the two diagram types, generated from the serde types.
//!
//! The published copies live in `schema/*.schema.json` at the repo root and are embedded here.
//! A test fails if they drift from the types; regenerate with
//! `UPDATE_SCHEMAS=1 cargo test -p tether-core schema_files`.

use super::spec::{Flowchart, Sequence};
use serde_json::{json, Value};

/// Embedded copy of `schema/flowchart.schema.json`.
pub const FLOWCHART_SCHEMA: &str = include_str!("../../../../schema/flowchart.schema.json");
/// Embedded copy of `schema/sequence.schema.json`.
pub const SEQUENCE_SCHEMA: &str = include_str!("../../../../schema/sequence.schema.json");

/// Collapse `oneOf` lists of string consts (schemars' output for documented enums) into
/// `enum`, which gives validators a much clearer error ("x is not one of [...]").
fn collapse_enums(v: &mut Value) {
    match v {
        Value::Object(map) => {
            let all_const = map
                .get("oneOf")
                .and_then(Value::as_array)
                .filter(|a| !a.is_empty())
                .is_some_and(|a| {
                    a.iter()
                        .all(|e| e.get("const").is_some_and(Value::is_string))
                });
            if all_const {
                if let Some(Value::Array(arms)) = map.remove("oneOf") {
                    let names: Vec<Value> = arms.iter().map(|a| a["const"].clone()).collect();
                    let doc: Vec<String> = arms
                        .iter()
                        .filter_map(|a| {
                            Some(format!(
                                "{}: {}",
                                a["const"].as_str()?,
                                a["description"].as_str()?
                            ))
                        })
                        .collect();
                    map.insert("type".into(), json!("string"));
                    map.insert("enum".into(), Value::Array(names));
                    if !doc.is_empty() {
                        let head = map
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        map.insert(
                            "description".into(),
                            json!(format!("{head} ({})", doc.join("; ")).trim().to_string()),
                        );
                    }
                }
            }
            // Optional fields are omitted, never null: drop the schemars `null` alternatives.
            if let Some(Value::Array(types)) = map.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let only = types.remove(0);
                    map.insert("type".into(), only);
                }
            }
            let nullable_any = map
                .get("anyOf")
                .and_then(Value::as_array)
                .is_some_and(|a| a.len() == 2 && a.iter().any(|e| e == &json!({ "type": "null" })));
            if nullable_any {
                if let Some(Value::Array(arms)) = map.remove("anyOf") {
                    if let Some(Value::Object(inner)) =
                        arms.into_iter().find(|e| e != &json!({ "type": "null" }))
                    {
                        map.extend(inner);
                    }
                }
            }
            for x in map.values_mut() {
                collapse_enums(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(collapse_enums),
        _ => {}
    }
}

fn with_type_tag(mut schema: Value, tag: &str, id: &str) -> Value {
    collapse_enums(&mut schema);
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("$id".into(), json!(id));
        if let Some(props) = obj.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert("type".into(), json!({ "const": tag }));
        }
        match obj.get_mut("required").and_then(Value::as_array_mut) {
            Some(req) => req.insert(0, json!("type")),
            None => {
                obj.insert("required".into(), json!(["type"]));
            }
        }
    }
    schema
}

/// Generate the flowchart schema from the Rust types.
pub fn generate_flowchart_schema() -> Value {
    let v = serde_json::to_value(schemars::schema_for!(Flowchart)).unwrap_or(Value::Null);
    with_type_tag(
        v,
        "flowchart",
        "https://tether.dev/schema/flowchart.schema.json",
    )
}

/// Generate the sequence schema from the Rust types.
pub fn generate_sequence_schema() -> Value {
    let v = serde_json::to_value(schemars::schema_for!(Sequence)).unwrap_or(Value::Null);
    with_type_tag(
        v,
        "sequence",
        "https://tether.dev/schema/sequence.schema.json",
    )
}

/// Pretty-printed schema text exactly as written to `schema/`.
pub fn render(schema: &Value) -> String {
    let mut s = serde_json::to_string_pretty(schema).unwrap_or_default();
    s.push('\n');
    s
}

/// Parsed embedded flowchart schema.
pub fn flowchart_schema() -> Value {
    serde_json::from_str(FLOWCHART_SCHEMA).unwrap_or(Value::Null)
}

/// Parsed embedded sequence schema.
pub fn sequence_schema() -> Value {
    serde_json::from_str(SEQUENCE_SCHEMA).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn schema_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema")
    }

    #[test]
    fn schema_files_match_generated() {
        for (name, generated) in [
            ("flowchart.schema.json", generate_flowchart_schema()),
            ("sequence.schema.json", generate_sequence_schema()),
        ] {
            let path = schema_dir().join(name);
            let text = render(&generated);
            if std::env::var_os("UPDATE_SCHEMAS").is_some() {
                std::fs::write(&path, &text).expect("write schema");
            }
            let on_disk = std::fs::read_to_string(&path).expect("read schema");
            assert_eq!(
                on_disk, text,
                "{name} drifted from the Rust types; run UPDATE_SCHEMAS=1 cargo test -p tether-core schema_files"
            );
        }
    }

    #[test]
    fn embedded_schemas_are_draft_2020_12_and_tagged() {
        for (s, tag) in [
            (flowchart_schema(), "flowchart"),
            (sequence_schema(), "sequence"),
        ] {
            assert_eq!(s["$schema"], "https://json-schema.org/draft/2020-12/schema");
            assert_eq!(s["properties"]["type"]["const"], tag);
            assert_eq!(s["required"][0], "type");
        }
    }
}
