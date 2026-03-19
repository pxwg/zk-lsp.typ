use std::path::Path;

use crate::parser::{NoteHeader, ParsedToml, Relation};

fn toml_value_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(n) => serde_json::Value::Number((*n).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(toml_value_to_json).collect())
        }
        toml::Value::Table(t) => {
            let map: serde_json::Map<String, serde_json::Value> = t
                .iter()
                .map(|(k, v)| (k.clone(), toml_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
    }
}

pub fn build_note_info_value(
    id: &str,
    path: &Path,
    header: &NoteHeader,
    parsed: &ParsedToml,
) -> serde_json::Value {
    use serde_json::{json, Map, Value};

    let checklist_status_str = parsed.checklist_status.as_str();
    let relation_str = match parsed.relation {
        Relation::Active => "active",
        Relation::Archived => "archived",
        Relation::Legacy => "legacy",
    };

    let mut metadata: Map<String, Value> = Map::new();
    metadata.insert("schema-version".into(), json!(parsed.schema_version));
    metadata.insert("aliases".into(), json!(parsed.aliases));
    metadata.insert(
        "abstract".into(),
        json!(parsed.abstract_text.as_deref().unwrap_or("")),
    );
    metadata.insert("keywords".into(), json!(parsed.keywords));
    metadata.insert("generated".into(), json!(parsed.generated));
    metadata.insert("checklist-status".into(), json!(checklist_status_str));
    metadata.insert("relation".into(), json!(relation_str));
    metadata.insert("relation-target".into(), json!(parsed.relation_target));

    for (k, v) in &parsed.extra {
        metadata.insert(k.clone(), toml_value_to_json(v));
    }

    json!({
        "id": id,
        "path": path.to_string_lossy().as_ref(),
        "title": header.title,
        "metadata": Value::Object(metadata),
    })
}

pub fn build_note_info_json(
    id: &str,
    path: &Path,
    header: &NoteHeader,
    parsed: &ParsedToml,
) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&build_note_info_value(
        id, path, header, parsed,
    ))?)
}
