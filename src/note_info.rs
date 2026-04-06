use std::path::Path;

use crate::config::{metadata_defaults_table, MetadataFieldConfig};
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

fn merge_missing_table_defaults(target: &mut toml::Table, defaults: &toml::Table) {
    for (key, default_value) in defaults {
        match target.get_mut(key) {
            Some(toml::Value::Table(target_table)) => {
                if let toml::Value::Table(default_table) = default_value {
                    merge_missing_table_defaults(target_table, default_table);
                }
            }
            Some(_) => {}
            None => {
                target.insert(key.clone(), default_value.clone());
            }
        }
    }
}

fn merged_extra_metadata(
    parsed: &ParsedToml,
    metadata_fields: &[MetadataFieldConfig],
) -> toml::Table {
    let mut merged = parsed.extra.clone();
    let defaults = metadata_defaults_table(metadata_fields);
    merge_missing_table_defaults(&mut merged, &defaults);
    merged
}

pub fn build_note_info_value(
    id: &str,
    path: &Path,
    header: &NoteHeader,
    parsed: &ParsedToml,
    metadata_fields: &[MetadataFieldConfig],
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

    for (k, v) in &merged_extra_metadata(parsed, metadata_fields) {
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
    metadata_fields: &[MetadataFieldConfig],
    content: &str,
) -> anyhow::Result<String> {
    let mut val = build_note_info_value(id, path, header, parsed, metadata_fields);
    val.as_object_mut().unwrap().insert(
        "content".into(),
        serde_json::Value::String(content.to_owned()),
    );
    Ok(serde_json::to_string_pretty(&val)?)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::build_note_info_value;
    use crate::config::{MetadataFieldConfig, MetadataFieldKind};
    use crate::parser::{ChecklistStatus, NoteHeader, ParsedToml, Relation};

    fn test_header() -> NoteHeader {
        NoteHeader {
            id: "2604070001".into(),
            title: "Test Note".into(),
            archived: false,
            legacy: false,
            alt_id: None,
            evo_id: None,
            relation_target: Vec::new(),
            aliases: Vec::new(),
            abstract_text: None,
            keywords: Vec::new(),
            tag_line_idx: None,
            title_line_idx: 0,
            metadata_block: None,
            checklist_status: Some(ChecklistStatus::None),
        }
    }

    #[test]
    fn note_info_includes_missing_custom_metadata_defaults() {
        let parsed = ParsedToml {
            extra: toml::toml! {
                [user]
                course = "QFT"
            },
            ..ParsedToml::default()
        };
        let metadata_fields = vec![
            MetadataFieldConfig {
                path: "user.course".into(),
                kind: MetadataFieldKind::String,
                default: toml::Value::String(String::new()),
            },
            MetadataFieldConfig {
                path: "user.priority".into(),
                kind: MetadataFieldKind::String,
                default: toml::Value::String("normal".into()),
            },
            MetadataFieldConfig {
                path: "user.tags".into(),
                kind: MetadataFieldKind::ArrayString,
                default: toml::Value::Array(Vec::new()),
            },
            MetadataFieldConfig {
                path: "user.reviewed".into(),
                kind: MetadataFieldKind::Boolean,
                default: toml::Value::Boolean(false),
            },
        ];

        let value = build_note_info_value(
            "2604070001",
            Path::new("/tmp/2604070001.typ"),
            &test_header(),
            &parsed,
            &metadata_fields,
        );

        assert_eq!(
            value.get("metadata").and_then(|m| m.get("user")),
            Some(&json!({
                "course": "QFT",
                "priority": "normal",
                "tags": [],
                "reviewed": false
            }))
        );
    }

    #[test]
    fn note_info_preserves_explicit_custom_metadata_values() {
        let parsed = ParsedToml {
            checklist_status: ChecklistStatus::Todo,
            relation: Relation::Archived,
            extra: toml::toml! {
                [user]
                priority = "urgent"
                reviewed = true
            },
            ..ParsedToml::default()
        };
        let metadata_fields = vec![
            MetadataFieldConfig {
                path: "user.priority".into(),
                kind: MetadataFieldKind::String,
                default: toml::Value::String("normal".into()),
            },
            MetadataFieldConfig {
                path: "user.reviewed".into(),
                kind: MetadataFieldKind::Boolean,
                default: toml::Value::Boolean(false),
            },
        ];

        let value = build_note_info_value(
            "2604070001",
            Path::new("/tmp/2604070001.typ"),
            &test_header(),
            &parsed,
            &metadata_fields,
        );

        assert_eq!(
            value.get("metadata").and_then(|m| m.get("user")),
            Some(&json!({
                "priority": "urgent",
                "reviewed": true
            }))
        );
    }
}
