use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::config::MetadataFieldConfig;
use crate::index::NoteIndex;
use crate::{metadata, parser};

pub fn get_metadata_completions(
    content: &str,
    position: Position,
    index: &Arc<NoteIndex>,
    metadata_fields: &[MetadataFieldConfig],
) -> Vec<CompletionItem> {
    let line_num = position.line as usize;
    let Some(current_record_id) = metadata::current_record_id(content, line_num) else {
        return Vec::new();
    };

    let lines: Vec<&str> = content.lines().collect();
    let current_line = lines.get(line_num).copied().unwrap_or("");
    let trimmed = current_line.trim_start();

    if trimmed.starts_with("checklist-status") && trimmed.contains('"') {
        return parser::ChecklistStatus::ALL
            .map(parser::ChecklistStatus::as_str)
            .iter()
            .map(|val| enum_completion(val))
            .collect();
    }

    if trimmed.starts_with("relation")
        && !trimmed.starts_with("relation-target")
        && trimmed.contains('"')
    {
        return ["active", "archived", "legacy"]
            .iter()
            .map(|val| enum_completion(val))
            .collect();
    }

    if is_relation_target_context(content, position) {
        let col = position.character as usize;
        let prefix = &current_line[..col.min(current_line.len())];
        let after_bracket = prefix.rfind('[').map_or(prefix, |p| &prefix[p + 1..]);
        let after_delim = after_bracket
            .rfind(',')
            .map_or(after_bracket, |p| &after_bracket[p + 1..]);
        let inside_string = after_delim.chars().filter(|&c| c == '"').count() % 2 == 1;

        return index
            .notes
            .iter()
            .filter(|entry| entry.key().as_str() != current_record_id)
            .map(|entry| {
                let info = entry.value();
                let insert_text = if inside_string {
                    info.id.clone()
                } else {
                    format!("\"{}\"", info.id)
                };
                CompletionItem {
                    label: info.id.clone(),
                    insert_text: Some(insert_text),
                    detail: Some(info.title.clone()),
                    filter_text: Some(format!("{} {}", info.id, info.title)),
                    kind: Some(CompletionItemKind::REFERENCE),
                    ..Default::default()
                }
            })
            .collect();
    }

    if trimmed.is_empty() {
        return missing_field_completions(content, &current_record_id, metadata_fields);
    }

    Vec::new()
}

fn enum_completion(value: &str) -> CompletionItem {
    CompletionItem {
        label: value.to_string(),
        kind: Some(CompletionItemKind::ENUM_MEMBER),
        ..Default::default()
    }
}

fn is_relation_target_context(content: &str, position: Position) -> bool {
    let mut in_relation_target = false;
    for (idx, line) in content.lines().enumerate() {
        if idx > position.line as usize {
            break;
        }
        if line.trim_start().starts_with('[') {
            in_relation_target = false;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("relation-target") {
            in_relation_target = true;
        }
        if idx == position.line as usize {
            return in_relation_target;
        }
        if in_relation_target && line.contains(']') {
            in_relation_target = false;
        }
    }
    false
}

fn missing_field_completions(
    content: &str,
    id: &str,
    metadata_fields: &[MetadataFieldConfig],
) -> Vec<CompletionItem> {
    let present = metadata::present_record_fields(content, id);
    let mut fields: Vec<String> = metadata::CORE_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    fields.extend(metadata_fields.iter().map(|field| field.path.clone()));

    fields
        .into_iter()
        .filter(|field| !present.iter().any(|present| present == field))
        .map(|field| CompletionItem {
            label: field.clone(),
            insert_text: Some(format!("{field} = ")),
            kind: Some(CompletionItemKind::FIELD),
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MetadataFieldConfig, MetadataFieldKind, WikiConfig};
    use std::path::PathBuf;

    fn empty_index() -> Arc<NoteIndex> {
        Arc::new(NoteIndex::new(Arc::new(tokio::sync::RwLock::new(
            WikiConfig::from_root(PathBuf::from("/tmp")),
        ))))
    }

    fn index_with_note(id: &str, title: &str) -> Arc<NoteIndex> {
        use crate::index::NoteInfo;
        let idx = NoteIndex::new(Arc::new(tokio::sync::RwLock::new(WikiConfig::from_root(
            PathBuf::from("/tmp"),
        ))));
        idx.notes.insert(
            id.to_string(),
            NoteInfo {
                id: id.to_string(),
                title: title.to_string(),
                archived: false,
                legacy: false,
                alt_id: None,
                evo_id: None,
                relation_target: vec![],
                aliases: vec![],
                keywords: vec![],
                abstract_text: None,
                checklist_status: None,
                path: PathBuf::from(format!("/tmp/{id}.typ")),
            },
        );
        Arc::new(idx)
    }

    const METADATA_CONTENT: &str = concat!(
        "format-version = 1\n\n",
        "[notes.\"2603110000\"]\n",
        "schema-version = 1\n",
        "checklist-status = \"none\"\n",
        "relation = \"active\"\n",
        "relation-target = []\n",
    );

    fn pos(line: u32) -> Position {
        Position { line, character: 0 }
    }

    #[test]
    fn completion_checklist_status_in_metadata_record() {
        let items = get_metadata_completions(METADATA_CONTENT, pos(4), &empty_index(), &[]);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"none"));
        assert!(labels.contains(&"todo"));
        assert!(labels.contains(&"wip"));
        assert!(labels.contains(&"done"));
    }

    #[test]
    fn completion_relation_target_inserts_quoted_id() {
        let pos_inside = Position {
            line: 6,
            character: 20,
        };
        let index = index_with_note("2603110001", "Some Note");
        let items = get_metadata_completions(METADATA_CONTENT, pos_inside, &index, &[]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "2603110001");
        assert_eq!(items[0].insert_text.as_deref(), Some("\"2603110001\""));
    }

    #[test]
    fn completion_missing_fields_excludes_title() {
        let content = "format-version = 1\n\n[notes.\"2603110000\"]\nschema-version = 1\n\n";
        let items = get_metadata_completions(content, pos(4), &empty_index(), &[]);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"checklist-status"));
        assert!(labels.contains(&"relation"));
        assert!(!labels.contains(&"schema-version"));
        assert!(!labels.contains(&"title"));
    }

    #[test]
    fn completion_missing_fields_recognizes_user_subtable_fields() {
        let content = concat!(
            "format-version = 1\n\n",
            "[notes.\"2603110000\"]\n",
            "schema-version = 1\n\n",
            "[notes.\"2603110000\".user]\n",
            "project = \"zk\"\n\n",
        );
        let fields = vec![
            MetadataFieldConfig {
                path: "user.project".into(),
                kind: MetadataFieldKind::String,
                default: toml::Value::String(String::new()),
            },
            MetadataFieldConfig {
                path: "user.reviewed".into(),
                kind: MetadataFieldKind::Boolean,
                default: toml::Value::Boolean(false),
            },
        ];
        let items = get_metadata_completions(
            content,
            Position {
                line: 7,
                character: 0,
            },
            &empty_index(),
            &fields,
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(!labels.contains(&"user.project"));
        assert!(labels.contains(&"user.reviewed"));
    }
}
