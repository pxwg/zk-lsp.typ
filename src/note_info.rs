use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::config::WikiConfig;
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

pub fn build_note_info_value_from_content(
    id: &str,
    path: &Path,
    content: &str,
    metadata_fields: &[MetadataFieldConfig],
) -> anyhow::Result<serde_json::Value> {
    let (header, parsed_toml) = parse_note_info_content(id, content)?;
    Ok(build_note_info_value(
        id,
        path,
        &header,
        &parsed_toml,
        metadata_fields,
    ))
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

pub async fn build_single_note_info_json(id: &str, config: &WikiConfig) -> anyhow::Result<String> {
    let path = config.note_dir.join(format!("{id}.typ"));
    if !path.exists() {
        anyhow::bail!("Note {id} not found at {}", path.display());
    }
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|err| anyhow::anyhow!("reading {}: {err}", path.display()))?;
    let (header, parsed_toml) = parse_note_info_content(id, &content)?;
    build_note_info_json(
        id,
        &path,
        &header,
        &parsed_toml,
        &config.zk_config.metadata.fields,
        &content,
    )
}

pub async fn build_notes_json(config: &WikiConfig) -> anyhow::Result<String> {
    let mut notes = collect_note_paths(&config.note_dir).await?;
    notes.sort_by(|(id_a, path_a), (id_b, path_b)| id_a.cmp(id_b).then_with(|| path_a.cmp(path_b)));

    let mut records = Vec::with_capacity(notes.len());
    for (id, path) in notes {
        records.push(build_note_record(&id, &path, config).await);
    }

    Ok(serde_json::to_string_pretty(&records)?)
}

async fn collect_note_paths(note_dir: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let mut notes = Vec::new();
    let mut entries = tokio::fs::read_dir(note_dir)
        .await
        .map_err(|err| anyhow::anyhow!("reading note dir {}: {err}", note_dir.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("typ") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.len() == 10 && stem.chars().all(|c| c.is_ascii_digit()) {
            notes.push((stem.to_string(), path));
        }
    }
    Ok(notes)
}

async fn build_note_record(id: &str, path: &Path, config: &WikiConfig) -> serde_json::Value {
    let stat = tokio::fs::metadata(path).await.ok();
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(err) => {
            let message = format!("reading {}: {err}", path.display());
            eprintln!("zk-lsp notes: warning: {message}");
            return note_error_value(id, path, &message, stat.as_ref());
        }
    };

    match build_note_info_value_from_content(id, path, &content, &config.zk_config.metadata.fields)
    {
        Ok(mut value) => {
            add_file_stat(&mut value, stat.as_ref());
            value
        }
        Err(err) => {
            let message = err.to_string();
            eprintln!("zk-lsp notes: warning: {}: {message}", path.display());
            note_error_value(id, path, &message, stat.as_ref())
        }
    }
}

fn note_error_value(
    id: &str,
    path: &Path,
    message: &str,
    stat: Option<&std::fs::Metadata>,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": id,
        "path": path.to_string_lossy().as_ref(),
        "title": "",
        "metadata": {},
        "error": message,
    });
    add_file_stat(&mut value, stat);
    value
}

fn add_file_stat(value: &mut serde_json::Value, stat: Option<&std::fs::Metadata>) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(stat) = stat else {
        return;
    };
    obj.insert("size".into(), serde_json::json!(stat.len()));
    if let Ok(modified) = stat.modified() {
        if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
            obj.insert("mtime".into(), serde_json::json!(duration.as_secs()));
        }
    }
}

fn parse_note_info_content(id: &str, content: &str) -> anyhow::Result<(NoteHeader, ParsedToml)> {
    let Some(block) = crate::parser::find_toml_metadata_block(content) else {
        anyhow::bail!("Failed to parse note {id} (may be legacy format; run zk-lsp migrate first)");
    };
    let parsed_toml = crate::parser::parse_toml_metadata(&block.toml_content).ok_or_else(|| {
        let detail = match block.toml_content.parse::<toml::Value>() {
            Ok(_) => "expected TOML metadata table".to_string(),
            Err(err) => err.to_string(),
        };
        anyhow::anyhow!("Failed to parse TOML metadata for note {id}: {detail}")
    })?;
    let header = crate::parser::parse_header(content).ok_or_else(|| {
        anyhow::anyhow!(
            "Failed to parse note {id} (may be legacy format; run zk-lsp migrate first)"
        )
    })?;
    Ok((header, parsed_toml))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{build_note_info_value, build_notes_json};
    use crate::config::WikiConfig;
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

    fn make_test_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zk_lsp_note_info_{name}_{}_{}",
            std::process::id(),
            nonce
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("note")).unwrap();
        root
    }

    fn write_note(root: &Path, id: &str, title: &str, extra_toml: &str) {
        let content = format!(
            "#import \"../include.typ\": *\n\
             #let zk-metadata = toml(bytes(\n\
               ```toml\n\
               schema-version = 1\n\
               aliases = []\n\
               abstract = \"\"\n\
               keywords = []\n\
               generated = false\n\
               checklist-status = \"none\"\n\
               relation = \"active\"\n\
               relation-target = []\n\
             {extra_toml}\
               ```.text,\n\
             ))\n\
             #show: zettel.with(metadata: zk-metadata)\n\
             \n\
             = {title} <{id}>\n"
        );
        fs::write(root.join("note").join(format!("{id}.typ")), content).unwrap();
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

    #[tokio::test]
    async fn notes_json_outputs_sorted_canonical_records_with_stat_fields() {
        let root = make_test_root("sorted_records");
        fs::write(
            root.join("zk-lsp.toml"),
            r#"
[metadata]
version = 1

[[metadata.field]]
path = "user.priority"
kind = "string"
default = "normal"
"#,
        )
        .unwrap();
        write_note(&root, "2222222222", "Second", "");
        write_note(
            &root,
            "1111111111",
            "First",
            "  [user]\n  priority = \"urgent\"\n",
        );

        let config = WikiConfig::from_root(root.clone());
        let out = build_notes_json(&config).await.unwrap();
        let _ = fs::remove_dir_all(&root);

        let items: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = items.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "1111111111");
        assert_eq!(arr[0]["title"], "First");
        assert_eq!(arr[0]["metadata"]["user"]["priority"], "urgent");
        assert!(arr[0]["mtime"].as_u64().is_some());
        assert!(arr[0]["size"].as_u64().is_some());
        assert_eq!(arr[1]["id"], "2222222222");
        assert_eq!(arr[1]["metadata"]["user"]["priority"], "normal");
    }

    #[tokio::test]
    async fn notes_json_keeps_bad_notes_as_error_records() {
        let root = make_test_root("bad_records");
        write_note(&root, "1111111111", "Good", "");
        fs::write(root.join("note/2222222222.typ"), "= Broken <2222222222>\n").unwrap();

        let config = WikiConfig::from_root(root.clone());
        let out = build_notes_json(&config).await.unwrap();
        let _ = fs::remove_dir_all(&root);

        let items: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = items.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "1111111111");
        assert_eq!(arr[1]["id"], "2222222222");
        assert_eq!(arr[1]["title"], "");
        assert_eq!(arr[1]["metadata"], json!({}));
        assert!(arr[1]["error"]
            .as_str()
            .unwrap()
            .contains("Failed to parse note"));
    }

    #[tokio::test]
    async fn notes_json_reports_malformed_toml_as_error_record() {
        let root = make_test_root("malformed_toml");
        write_note(&root, "1111111111", "Good", "");
        fs::write(
            root.join("note/2222222222.typ"),
            "#let zk-metadata = toml(bytes(\n\
               ```toml\n\
               schema-version = [\n\
               ```.text,\n\
             ))\n\
             #show: zettel.with(metadata: zk-metadata)\n\
             \n\
             = Broken <2222222222>\n",
        )
        .unwrap();

        let config = WikiConfig::from_root(root.clone());
        let out = build_notes_json(&config).await.unwrap();
        let _ = fs::remove_dir_all(&root);

        let items: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = items.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["id"], "2222222222");
        assert!(arr[1]["error"]
            .as_str()
            .unwrap()
            .contains("Failed to parse TOML metadata"));
    }
}
