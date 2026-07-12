use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::config::WikiConfig;
use crate::config::{metadata_defaults_table, MetadataFieldConfig};
use crate::metadata::{self, MetadataRecord};
use crate::parser::{NoteHeader, Relation};

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
    parsed: &MetadataRecord,
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
    parsed: &MetadataRecord,
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
    record: &MetadataRecord,
    metadata_fields: &[MetadataFieldConfig],
) -> anyhow::Result<serde_json::Value> {
    let header = parse_note_info_content(id, content)?;
    Ok(build_note_info_value(
        id,
        path,
        &header,
        record,
        metadata_fields,
    ))
}

pub fn build_note_info_json(
    id: &str,
    path: &Path,
    header: &NoteHeader,
    parsed: &MetadataRecord,
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
    let header = parse_note_info_content(id, &content)?;
    let record = metadata::read_record(config, id).await?;
    build_note_info_json(
        id,
        &path,
        &header,
        &record,
        &config.zk_config.metadata.fields,
        &content,
    )
}

#[allow(dead_code)]
pub async fn build_notes_json(config: &WikiConfig) -> anyhow::Result<String> {
    let records = build_note_values(config).await?;
    Ok(serde_json::to_string_pretty(&records)?)
}

pub async fn write_notes_json<W: std::io::Write>(
    config: &WikiConfig,
    mut writer: W,
    compact: bool,
) -> anyhow::Result<()> {
    let records = build_note_values(config).await?;
    if compact {
        serde_json::to_writer(&mut writer, &records)?;
    } else {
        serde_json::to_writer_pretty(&mut writer, &records)?;
    }
    writer.write_all(b"\n")?;
    Ok(())
}

async fn build_note_values(config: &WikiConfig) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut notes = collect_note_paths(&config.note_dir).await?;
    notes.sort_by(|(id_a, path_a), (id_b, path_b)| id_a.cmp(id_b).then_with(|| path_a.cmp(path_b)));
    let metadata_snapshot = metadata::MetadataSnapshot::load(config)
        .await
        .unwrap_or_else(metadata::MetadataSnapshot::unavailable);
    let cache_context = CacheContext::load(config).await;
    let cache_path = config.root.join(".zk-lsp/notes-v1.json");
    let cache = NotesCache::load(&cache_path, &cache_context).await;

    const READ_CONCURRENCY: usize = 32;
    let metadata_snapshot = &metadata_snapshot;
    let cached_records = &cache.records;
    let results: Vec<NoteRecordResult> = stream::iter(notes)
        .map(move |(id, path)| async move {
            build_note_record(
                &id,
                &path,
                config,
                metadata_snapshot,
                cached_records.get(&id),
            )
            .await
        })
        .buffered(READ_CONCURRENCY)
        .collect()
        .await;

    let mut records = Vec::with_capacity(results.len());
    let mut next_cache_records = HashMap::with_capacity(results.len());
    for result in results {
        if let Some(warning) = &result.warning {
            eprintln!("zk-lsp notes: warning: {warning}");
        }
        if let Some(fingerprint) = &result.fingerprint {
            next_cache_records.insert(
                result.id.clone(),
                CachedNoteRecord {
                    fingerprint: fingerprint.clone(),
                    value: result.value.clone(),
                    warning: result.warning.clone(),
                },
            );
        }
        records.push(result.value);
    }
    NotesCache::save(cache_path, cache_context, next_cache_records).await;

    Ok(records)
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

async fn build_note_record(
    id: &str,
    path: &Path,
    config: &WikiConfig,
    metadata_snapshot: &metadata::MetadataSnapshot,
    cached: Option<&CachedNoteRecord>,
) -> NoteRecordResult {
    let initial_stat = tokio::fs::metadata(path).await.ok();
    let initial_fingerprint = initial_stat
        .as_ref()
        .and_then(FileFingerprint::from_metadata);
    if let (Some(cached), Some(fingerprint)) = (cached, &initial_fingerprint) {
        if cached.fingerprint == *fingerprint {
            return NoteRecordResult {
                id: id.to_string(),
                value: cached.value.clone(),
                warning: cached.warning.clone(),
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }

    let (content, stat) = match read_note_with_stat(path).await {
        Ok(result) => result,
        Err(err) => {
            let message = format!("reading {}: {err}", path.display());
            return NoteRecordResult::warning(
                id,
                note_error_value(id, path, &message, None),
                message,
                None,
            );
        }
    };

    let record = match metadata_snapshot.record(id) {
        Ok(record) => record,
        Err(err) => {
            let message = err.to_string();
            return NoteRecordResult::warning(
                id,
                note_error_value(id, path, &message, stat.as_ref()),
                format!("{}: {message}", path.display()),
                stat.as_ref().and_then(FileFingerprint::from_metadata),
            );
        }
    };

    match build_note_info_value_from_content(
        id,
        path,
        &content,
        &record,
        &config.zk_config.metadata.fields,
    ) {
        Ok(mut value) => {
            add_file_stat(&mut value, stat.as_ref());
            NoteRecordResult::ok(
                id,
                value,
                stat.as_ref().and_then(FileFingerprint::from_metadata),
            )
        }
        Err(err) => {
            let message = err.to_string();
            NoteRecordResult::warning(
                id,
                note_error_value(id, path, &message, stat.as_ref()),
                format!("{}: {message}", path.display()),
                stat.as_ref().and_then(FileFingerprint::from_metadata),
            )
        }
    }
}

async fn read_note_with_stat(path: &Path) -> anyhow::Result<(String, Option<std::fs::Metadata>)> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;

        let mut file = std::fs::File::open(path)?;
        let stat = file.metadata().ok();
        let mut content = String::with_capacity(
            stat.as_ref()
                .and_then(|metadata| usize::try_from(metadata.len()).ok())
                .unwrap_or(0),
        );
        file.read_to_string(&mut content)?;
        Ok((content, stat))
    })
    .await
    .map_err(|err| anyhow::anyhow!("note reader task failed: {err}"))?
}

struct NoteRecordResult {
    id: String,
    value: serde_json::Value,
    warning: Option<String>,
    fingerprint: Option<FileFingerprint>,
}

impl NoteRecordResult {
    fn ok(id: &str, value: serde_json::Value, fingerprint: Option<FileFingerprint>) -> Self {
        Self {
            id: id.to_string(),
            value,
            warning: None,
            fingerprint,
        }
    }

    fn warning(
        id: &str,
        value: serde_json::Value,
        warning: String,
        fingerprint: Option<FileFingerprint>,
    ) -> Self {
        Self {
            id: id.to_string(),
            value,
            warning: Some(warning),
            fingerprint,
        }
    }
}

const NOTES_CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FileFingerprint {
    size: u64,
    modified_nanos: u64,
}

impl FileFingerprint {
    fn from_metadata(metadata: &std::fs::Metadata) -> Option<Self> {
        let nanos = metadata
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(Self {
            size: metadata.len(),
            modified_nanos: nanos.min(u128::from(u64::MAX)) as u64,
        })
    }

    async fn load(path: &Path) -> Option<Self> {
        Self::from_metadata(&tokio::fs::metadata(path).await.ok()?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CacheContext {
    metadata: Option<FileFingerprint>,
    metadata_config_hash: u64,
}

impl CacheContext {
    async fn load(config: &WikiConfig) -> Self {
        let mut hasher = DefaultHasher::new();
        for field in &config.zk_config.metadata.fields {
            field.path.hash(&mut hasher);
            format!("{:?}", field.kind).hash(&mut hasher);
            field.default.to_string().hash(&mut hasher);
        }
        Self {
            metadata: FileFingerprint::load(&metadata::metadata_path(&config.root)).await,
            metadata_config_hash: hasher.finish(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct CachedNoteRecord {
    fingerprint: FileFingerprint,
    value: serde_json::Value,
    warning: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct NotesCache {
    version: u32,
    context: CacheContext,
    records: HashMap<String, CachedNoteRecord>,
}

impl NotesCache {
    async fn load(path: &Path, context: &CacheContext) -> Self {
        let loaded = match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes).ok(),
            Err(_) => None,
        };
        match loaded {
            Some(cache) if cache.version == NOTES_CACHE_VERSION && cache.context == *context => {
                cache
            }
            _ => Self {
                version: NOTES_CACHE_VERSION,
                context: context.clone(),
                records: HashMap::new(),
            },
        }
    }

    async fn save(
        path: PathBuf,
        context: CacheContext,
        records: HashMap<String, CachedNoteRecord>,
    ) {
        let cache = Self {
            version: NOTES_CACHE_VERSION,
            context,
            records,
        };
        let Ok(bytes) = serde_json::to_vec(&cache) else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return;
        }
        let temp = path.with_extension(format!("tmp-{}", std::process::id()));
        if tokio::fs::write(&temp, bytes).await.is_ok()
            && tokio::fs::rename(&temp, &path).await.is_err()
        {
            let _ = tokio::fs::remove_file(&temp).await;
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

fn parse_note_info_content(id: &str, content: &str) -> anyhow::Result<NoteHeader> {
    let header = crate::parser::parse_header(content).ok_or_else(|| {
        anyhow::anyhow!("Failed to parse note {id}: missing or invalid title/metadata binding")
    })?;
    if header.id != id {
        anyhow::bail!("Failed to parse note {id}: title ID does not match current note");
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{build_note_info_value, build_notes_json, write_notes_json};
    use crate::config::WikiConfig;
    use crate::config::{MetadataFieldConfig, MetadataFieldKind};
    use crate::metadata::MetadataRecord;
    use crate::parser::{ChecklistStatus, NoteHeader, Relation};

    fn test_header() -> NoteHeader {
        NoteHeader {
            id: "2604070001".into(),
            title: "Test Note".into(),
            title_line_idx: 0,
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
             #let zk-metadata = zk_metadata(\"{id}\")\n\
             #show: zettel.with(metadata: zk-metadata)\n\
             \n\
             = {title} <{id}>\n"
        );
        fs::write(root.join("note").join(format!("{id}.typ")), content).unwrap();
        let metadata_path = root.join("metadata.toml");
        if !metadata_path.exists() {
            fs::write(&metadata_path, "format-version = 1\n\n").unwrap();
        }
        let user_table = if extra_toml.trim().is_empty() {
            String::new()
        } else {
            format!("\n[notes.\"{id}\".user]\n{extra_toml}\n")
        };
        let record = format!(
            "[notes.\"{id}\"]\n\
             schema-version = 1\n\
             aliases = []\n\
             abstract = \"\"\n\
             keywords = []\n\
             generated = false\n\
             checklist-status = \"none\"\n\
             relation = \"active\"\n\
             relation-target = []\n\
             {user_table}\n"
        );
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&metadata_path)
            .unwrap();
        file.write_all(record.as_bytes()).unwrap();
    }

    #[test]
    fn note_info_includes_missing_custom_metadata_defaults() {
        let parsed = MetadataRecord {
            extra: toml::toml! {
                [user]
                course = "QFT"
            },
            ..MetadataRecord::default()
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
        let parsed = MetadataRecord {
            checklist_status: ChecklistStatus::Todo,
            relation: Relation::Archived,
            extra: toml::toml! {
                [user]
                priority = "urgent"
                reviewed = true
            },
            ..MetadataRecord::default()
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
        write_note(&root, "2222222222", "Second", "priority = \"normal\"\n");
        write_note(&root, "1111111111", "First", "priority = \"urgent\"\n");

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
        let error = arr[1]["error"].as_str().unwrap();
        assert!(
            error.contains("Failed to parse note") || error.contains("Missing metadata"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn notes_json_reports_malformed_toml_as_error_record() {
        let root = make_test_root("malformed_toml");
        write_note(&root, "1111111111", "Good", "");
        fs::write(
            root.join("note/2222222222.typ"),
            "#import \"../include.typ\": *\n\
             #let zk-metadata = zk_metadata(\"2222222222\")\n\
             #show: zettel.with(metadata: zk-metadata)\n\
             \n\
             = Broken <2222222222>\n",
        )
        .unwrap();
        fs::write(
            root.join("metadata.toml"),
            "format-version = 1\n[notes.\"2222222222\"\nschema-version = 1\n",
        )
        .unwrap();

        let config = WikiConfig::from_root(root.clone());
        let out = build_notes_json(&config).await.unwrap();
        let _ = fs::remove_dir_all(&root);

        let items: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = items.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["id"], "2222222222");
        assert!(arr[1]["error"].as_str().unwrap().contains("parsing"));
    }

    #[tokio::test]
    async fn notes_json_cache_invalidates_changed_notes_and_metadata() {
        let root = make_test_root("cache_invalidation");
        write_note(&root, "1111111111", "First", "");
        let config = WikiConfig::from_root(root.clone());

        let first = build_notes_json(&config).await.unwrap();
        assert!(root.join(".zk-lsp/notes-v1.json").is_file());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&first).unwrap()[0]["title"],
            "First"
        );

        let note_path = root.join("note/1111111111.typ");
        let changed = fs::read_to_string(&note_path)
            .unwrap()
            .replace("= First <", "= A longer second title <");
        fs::write(&note_path, changed).unwrap();
        let second = build_notes_json(&config).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&second).unwrap()[0]["title"],
            "A longer second title"
        );

        let metadata_path = root.join("metadata.toml");
        let changed = fs::read_to_string(&metadata_path)
            .unwrap()
            .replace("relation = \"active\"", "relation = \"archived\"");
        fs::write(metadata_path, changed).unwrap();
        let third = build_notes_json(&config).await.unwrap();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&third).unwrap()[0]["metadata"]["relation"],
            "archived"
        );
    }

    #[tokio::test]
    async fn notes_json_writer_supports_pretty_and_compact_output() {
        let root = make_test_root("streaming_output");
        write_note(&root, "1111111111", "First", "");
        let config = WikiConfig::from_root(root.clone());

        let mut pretty = Vec::new();
        write_notes_json(&config, &mut pretty, false).await.unwrap();
        let mut compact = Vec::new();
        write_notes_json(&config, &mut compact, true).await.unwrap();
        let _ = fs::remove_dir_all(&root);

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&pretty).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&compact).unwrap()
        );
        assert!(compact.len() < pretty.len());
        assert!(pretty.ends_with(b"\n"));
        assert!(compact.ends_with(b"\n"));
    }
}
