use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs;

use crate::config::{metadata_defaults_table, MetadataFieldConfig, MetadataFieldKind, ZkLspConfig};
use crate::parser::{ChecklistStatus, Relation};

pub const FORMAT_VERSION: i64 = 1;
pub const SCHEMA_VERSION: i64 = 1;

pub const CORE_FIELDS: &[&str] = &[
    "schema-version",
    "aliases",
    "abstract",
    "keywords",
    "generated",
    "checklist-status",
    "relation",
    "relation-target",
];

#[derive(Debug, Clone)]
pub struct MetadataRecord {
    pub schema_version: u32,
    pub aliases: Vec<String>,
    pub abstract_text: Option<String>,
    pub keywords: Vec<String>,
    pub generated: bool,
    pub checklist_status: ChecklistStatus,
    pub relation: Relation,
    pub relation_target: Vec<String>,
    pub extra: toml::Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataTextRange {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataIdKind {
    RecordKey,
    RelationTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataIdPosition {
    pub kind: MetadataIdKind,
    pub owner_id: String,
    pub target_id: String,
    pub range: MetadataTextRange,
}

impl Default for MetadataRecord {
    fn default() -> Self {
        Self {
            schema_version: 1,
            aliases: Vec::new(),
            abstract_text: None,
            keywords: Vec::new(),
            generated: true,
            checklist_status: ChecklistStatus::None,
            relation: Relation::Active,
            relation_target: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetadataSnapshot {
    records: HashMap<String, MetadataRecord>,
    record_errors: HashMap<String, String>,
    global_error: Option<String>,
}

impl MetadataSnapshot {
    pub async fn load(config: &crate::config::WikiConfig) -> Result<Self> {
        let root = read_index_table(config).await?;
        Self::from_index_table(&root, &config.zk_config.metadata.fields)
    }

    pub fn unavailable(error: impl ToString) -> Self {
        Self {
            records: HashMap::new(),
            record_errors: HashMap::new(),
            global_error: Some(error.to_string()),
        }
    }

    pub fn record(&self, id: &str) -> Result<&MetadataRecord> {
        if let Some(error) = &self.global_error {
            anyhow::bail!("{error}");
        }
        if let Some(record) = self.records.get(id) {
            return Ok(record);
        }
        if let Some(error) = self.record_errors.get(id) {
            anyhow::bail!("{error}");
        }
        anyhow::bail!("Missing metadata for current note")
    }

    pub fn into_records(self) -> HashMap<String, MetadataRecord> {
        self.records
    }

    fn from_index_table(
        root: &toml::Table,
        metadata_fields: &[MetadataFieldConfig],
    ) -> Result<Self> {
        let notes = root
            .get("notes")
            .and_then(|v| v.as_table())
            .ok_or_else(|| anyhow::anyhow!("Missing notes table"))?;

        let mut records = HashMap::new();
        let mut record_errors = HashMap::new();
        for (id, value) in notes {
            if !is_note_id(id) {
                continue;
            }
            let Some(table) = value.as_table() else {
                record_errors.insert(id.clone(), "Invalid metadata for current note".to_string());
                continue;
            };
            match parse_record_for_read(table, metadata_fields) {
                Ok(record) => {
                    records.insert(id.clone(), record);
                }
                Err(err) => {
                    record_errors.insert(id.clone(), err.to_string());
                }
            }
        }

        Ok(Self {
            records,
            record_errors,
            global_error: None,
        })
    }
}

pub fn current_record_id(content: &str, line_idx: usize) -> Option<String> {
    let mut current = None;
    for (idx, line) in content.lines().enumerate() {
        if idx > line_idx {
            break;
        }
        if let Some((id, _, _, _)) = parse_notes_header(line) {
            current = Some(id);
        } else if is_any_table_header(line) && !is_notes_subtable_header(line) {
            current = None;
        }
    }
    current
}

pub fn find_record_key_range(content: &str, id: &str) -> Option<MetadataTextRange> {
    for (line_idx, line) in content.lines().enumerate() {
        let Some((header_id, start_col, end_col, is_record_table)) = parse_notes_header(line)
        else {
            continue;
        };
        if is_record_table && header_id == id {
            return Some(MetadataTextRange {
                line: line_idx,
                start_col,
                end_col,
            });
        }
    }
    None
}

pub fn id_at_position(content: &str, line_idx: usize, col: usize) -> Option<MetadataIdPosition> {
    all_id_positions(content).into_iter().find(|pos| {
        pos.range.line == line_idx && col >= pos.range.start_col && col <= pos.range.end_col
    })
}

pub fn all_id_positions(content: &str) -> Vec<MetadataIdPosition> {
    let mut positions = Vec::new();
    let mut current_owner: Option<String> = None;
    let mut in_relation_target = false;

    for (line_idx, line) in content.lines().enumerate() {
        if let Some((id, start_col, end_col, is_record_table)) = parse_notes_header(line) {
            current_owner = Some(id.clone());
            in_relation_target = false;
            if is_record_table {
                positions.push(MetadataIdPosition {
                    kind: MetadataIdKind::RecordKey,
                    owner_id: id.clone(),
                    target_id: id,
                    range: MetadataTextRange {
                        line: line_idx,
                        start_col,
                        end_col,
                    },
                });
            }
            continue;
        }
        if is_any_table_header(line) && !is_notes_subtable_header(line) {
            current_owner = None;
            in_relation_target = false;
            continue;
        }

        let Some(owner_id) = current_owner.clone() else {
            continue;
        };

        let trimmed = line.trim_start();
        let starts_relation_target = trimmed.starts_with("relation-target")
            && trimmed
                .chars()
                .nth("relation-target".len())
                .map(|c| c.is_whitespace() || c == '=')
                .unwrap_or(true);
        if starts_relation_target {
            in_relation_target = true;
        }

        if in_relation_target {
            for (target_id, start_col, end_col) in quoted_note_ids(line) {
                positions.push(MetadataIdPosition {
                    kind: MetadataIdKind::RelationTarget,
                    owner_id: owner_id.clone(),
                    target_id,
                    range: MetadataTextRange {
                        line: line_idx,
                        start_col,
                        end_col,
                    },
                });
            }
            if line.contains(']') {
                in_relation_target = false;
            }
        }
    }

    positions
}

pub fn find_field_line_range(content: &str, id: &str, field: &str) -> Option<MetadataTextRange> {
    let mut current_owner: Option<String> = None;
    let mut in_user_table = false;
    for (line_idx, line) in content.lines().enumerate() {
        if let Some((header_id, _, _, is_record_table)) = parse_notes_header(line) {
            current_owner = Some(header_id);
            in_user_table = !is_record_table && is_notes_user_subtable_header(line);
            continue;
        }
        if is_any_table_header(line) && !is_notes_subtable_header(line) {
            current_owner = None;
            in_user_table = false;
            continue;
        }
        if current_owner.as_deref() != Some(id) {
            continue;
        }
        let trimmed = line.trim_start();
        let Some((candidate, _)) = trimmed.split_once('=') else {
            continue;
        };
        let candidate = candidate.trim();
        let field_name = if in_user_table {
            field.strip_prefix("user.").unwrap_or(field)
        } else {
            field
        };
        if candidate == field_name {
            return Some(MetadataTextRange {
                line: line_idx,
                start_col: 0,
                end_col: line.len(),
            });
        }
    }
    None
}

pub fn present_record_fields(content: &str, id: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current_owner: Option<String> = None;
    let mut in_user_table = false;
    for line in content.lines() {
        if let Some((header_id, _, _, is_record_table)) = parse_notes_header(line) {
            current_owner = Some(header_id);
            in_user_table = !is_record_table && is_notes_user_subtable_header(line);
            continue;
        }
        if is_any_table_header(line) && !is_notes_subtable_header(line) {
            current_owner = None;
            in_user_table = false;
            continue;
        }
        if current_owner.as_deref() != Some(id) {
            continue;
        }
        let trimmed = line.trim_start();
        let Some((candidate, _)) = trimmed.split_once('=') else {
            continue;
        };
        let field = candidate.trim();
        let field = if in_user_table {
            format!("user.{field}")
        } else {
            field.to_string()
        };
        if !field.is_empty() && !fields.iter().any(|f| f == &field) {
            fields.push(field);
        }
    }
    fields
}

pub fn metadata_path(root: &Path) -> PathBuf {
    root.join("metadata.toml")
}

pub fn is_note_id(id: &str) -> bool {
    id.len() == 10 && id.chars().all(|c| c.is_ascii_digit())
}

pub fn default_record_table(config: &ZkLspConfig) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert(
        "schema-version".into(),
        toml::Value::Integer(SCHEMA_VERSION),
    );
    table.insert("aliases".into(), toml::Value::Array(Vec::new()));
    table.insert("abstract".into(), toml::Value::String(String::new()));
    table.insert("keywords".into(), toml::Value::Array(Vec::new()));
    table.insert("generated".into(), toml::Value::Boolean(true));
    table.insert(
        "checklist-status".into(),
        toml::Value::String("none".into()),
    );
    table.insert("relation".into(), toml::Value::String("active".into()));
    table.insert("relation-target".into(), toml::Value::Array(Vec::new()));

    for (key, value) in metadata_defaults_table(&config.metadata.fields) {
        table.insert(key, value);
    }

    table
}

pub fn complete_record_table(table: &mut toml::Table, config: &ZkLspConfig) {
    let defaults = default_record_table(config);
    merge_missing_values(table, &defaults);
}

fn merge_missing_values(target: &mut toml::Table, defaults: &toml::Table) {
    for (key, default_value) in defaults {
        match (target.get_mut(key), default_value) {
            (Some(toml::Value::Table(target_table)), toml::Value::Table(default_table)) => {
                merge_missing_values(target_table, default_table);
            }
            (Some(_), _) => {}
            (None, _) => {
                target.insert(key.clone(), default_value.clone());
            }
        }
    }
}

pub fn parse_record_table(table: &toml::Table) -> Option<MetadataRecord> {
    let schema_version = table
        .get("schema-version")
        .and_then(|v| v.as_integer())
        .map(|n| n as u32)
        .unwrap_or(1);

    let generated = table
        .get("generated")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let aliases = string_array(table.get("aliases"));
    let keywords = string_array(table.get("keywords"));

    let abstract_text = table
        .get("abstract")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let checklist_status = table
        .get("checklist-status")
        .and_then(|v| v.as_str())
        .and_then(ChecklistStatus::from_str)
        .unwrap_or(ChecklistStatus::None);

    let relation = table
        .get("relation")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "archived" => Relation::Archived,
            "legacy" => Relation::Legacy,
            _ => Relation::Active,
        })
        .unwrap_or(Relation::Active);

    let relation_target = string_array(table.get("relation-target"));

    let extra = table
        .iter()
        .filter(|(k, _)| !CORE_FIELDS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    Some(MetadataRecord {
        schema_version,
        aliases,
        abstract_text,
        keywords,
        generated,
        checklist_status,
        relation,
        relation_target,
        extra,
    })
}

fn string_array(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_notes_header(line: &str) -> Option<(String, usize, usize, bool)> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let after_prefix = trimmed.strip_prefix("[notes.")?;
    let first = after_prefix.as_bytes().first().copied()?;
    let (id, after_id_start, key_offset) = if first == b'"' || first == b'\'' {
        let quote = first as char;
        let after_quote = &after_prefix[1..];
        let end_rel = after_quote.find(quote)?;
        (&after_quote[..end_rel], 1 + end_rel + 1, 1)
    } else {
        let end_rel = after_prefix
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .unwrap_or(after_prefix.len());
        (&after_prefix[..end_rel], end_rel, 0)
    };
    if !is_note_id(id) {
        return None;
    }
    let after_id = &after_prefix[after_id_start..];
    let is_record_table = after_id.trim_start().starts_with(']');
    let is_notes_subtable = after_id.trim_start().starts_with('.');
    if !is_record_table && !is_notes_subtable {
        return None;
    }
    let start_col = indent + "[notes.".len() + key_offset;
    let end_col = start_col + id.len();
    Some((id.to_string(), start_col, end_col, is_record_table))
}

fn is_any_table_header(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

fn is_notes_subtable_header(line: &str) -> bool {
    parse_notes_header(line)
        .map(|(_, _, _, is_record_table)| !is_record_table)
        .unwrap_or(false)
}

fn is_notes_user_subtable_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with(".user]")
}

fn quoted_note_ids(line: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let end = start + 10;
        if end < bytes.len()
            && bytes[end] == quote
            && bytes[start..end].iter().all(|b| b.is_ascii_digit())
        {
            out.push((line[start..end].to_string(), start, end));
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

pub async fn read_record_table(
    config: &crate::config::WikiConfig,
    id: &str,
) -> Result<toml::Table> {
    let root = read_index_table(config).await?;
    let notes = root
        .get("notes")
        .and_then(|v| v.as_table())
        .ok_or_else(|| anyhow::anyhow!("Missing notes table"))?;
    notes
        .get(id)
        .and_then(|v| v.as_table())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Missing metadata for current note"))
}

pub async fn read_record_table_or_default(
    config: &crate::config::WikiConfig,
    id: &str,
) -> Result<toml::Table> {
    match read_record_table(config, id).await {
        Ok(table) => Ok(table),
        Err(err) if is_missing_record_error(&err) || is_missing_metadata_file_error(&err) => {
            Ok(default_record_table(&config.zk_config))
        }
        Err(err) => Err(err),
    }
}

pub async fn read_record(config: &crate::config::WikiConfig, id: &str) -> Result<MetadataRecord> {
    let table = read_record_table(config, id).await?;
    parse_record_for_read(&table, &config.zk_config.metadata.fields)
}

pub async fn read_records(
    config: &crate::config::WikiConfig,
) -> Result<HashMap<String, MetadataRecord>> {
    Ok(MetadataSnapshot::load(config).await?.into_records())
}

fn parse_record_for_read(
    table: &toml::Table,
    metadata_fields: &[MetadataFieldConfig],
) -> Result<MetadataRecord> {
    validate_record_table_for_read(table, metadata_fields)?;
    parse_record_table(table).ok_or_else(|| anyhow::anyhow!("Invalid metadata for current note"))
}

pub async fn ensure_record(
    config: &crate::config::WikiConfig,
    id: &str,
    overrides: &HashMap<String, toml::Value>,
) -> Result<()> {
    let mut table = read_record_table_or_default(config, id).await?;
    complete_record_table(&mut table, &config.zk_config);
    apply_patch_to_table(&mut table, overrides, &config.zk_config.metadata.fields)?;
    put_record_table(config, id, &table).await
}

pub async fn patch_record(
    config: &crate::config::WikiConfig,
    id: &str,
    patch: &HashMap<String, toml::Value>,
) -> Result<()> {
    let mut table = read_record_table_or_default(config, id).await?;
    complete_record_table(&mut table, &config.zk_config);
    apply_patch_to_table(&mut table, patch, &config.zk_config.metadata.fields)?;
    put_record_table(config, id, &table).await
}

pub async fn put_record_table(
    config: &crate::config::WikiConfig,
    id: &str,
    table: &toml::Table,
) -> Result<()> {
    anyhow::ensure!(is_note_id(id), "Invalid note ID");
    let mut root = match read_index_table(config).await {
        Ok(root) => root,
        Err(err) if is_missing_metadata_file_error(&err) => empty_index_table(),
        Err(err) => return Err(err),
    };
    root.insert(
        "format-version".into(),
        toml::Value::Integer(FORMAT_VERSION),
    );
    let notes = root
        .entry("notes")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let toml::Value::Table(notes_table) = notes else {
        anyhow::bail!("Missing notes table");
    };
    notes_table.insert(
        id.to_string(),
        toml::Value::Table(canonical_record_table(table)),
    );
    write_index_table(&metadata_path(&config.root), &root).await
}

fn empty_index_table() -> toml::Table {
    let mut table = toml::Table::new();
    table.insert(
        "format-version".into(),
        toml::Value::Integer(FORMAT_VERSION),
    );
    table.insert("notes".into(), toml::Value::Table(toml::Table::new()));
    table
}

fn is_missing_record_error(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string() == "Missing metadata for current note")
}

fn is_missing_metadata_file_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    })
}

pub async fn delete_record(config: &crate::config::WikiConfig, id: &str) -> Result<()> {
    let mut root = read_index_table(config).await?;
    if let Some(notes) = root.get_mut("notes").and_then(|v| v.as_table_mut()) {
        notes.remove(id);
    }
    write_index_table(&metadata_path(&config.root), &root).await
}

pub fn apply_patch_to_table(
    table: &mut toml::Table,
    patch: &HashMap<String, toml::Value>,
    metadata_fields: &[MetadataFieldConfig],
) -> Result<()> {
    for (key, value) in patch {
        validate_patch_entry(key, value, metadata_fields)?;
        if let Some(user_key) = key.strip_prefix("user.") {
            let user = table
                .entry("user")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let toml::Value::Table(user_table) = user else {
                anyhow::bail!("Invalid metadata for current note");
            };
            user_table.insert(user_key.to_string(), value.clone());
        } else {
            table.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn validate_patch_entry(
    key: &str,
    value: &toml::Value,
    metadata_fields: &[MetadataFieldConfig],
) -> Result<()> {
    if CORE_FIELDS.contains(&key) {
        validate_core_value(key, value)?;
        return Ok(());
    }
    if key.starts_with("user.") {
        if let Some(field) = metadata_fields
            .iter()
            .find(|field| field.path.as_str() == key)
        {
            anyhow::ensure!(
                value_matches_kind(value, &field.kind),
                "invalid metadata value for '{key}'"
            );
            return Ok(());
        }
    }
    anyhow::bail!("unknown metadata field '{key}'")
}

fn validate_core_value(key: &str, value: &toml::Value) -> Result<()> {
    match key {
        "schema-version" => anyhow::ensure!(
            value.as_integer() == Some(SCHEMA_VERSION),
            "invalid metadata value for '{key}'"
        ),
        "aliases" | "keywords" | "relation-target" => anyhow::ensure!(
            value
                .as_array()
                .map(|items| items.iter().all(|item| item.as_str().is_some()))
                .unwrap_or(false),
            "invalid metadata value for '{key}'"
        ),
        "abstract" => anyhow::ensure!(
            value.as_str().is_some(),
            "invalid metadata value for '{key}'"
        ),
        "generated" => anyhow::ensure!(
            value.as_bool().is_some(),
            "invalid metadata value for '{key}'"
        ),
        "checklist-status" => anyhow::ensure!(
            value.as_str().and_then(ChecklistStatus::from_str).is_some(),
            "invalid metadata value for '{key}'"
        ),
        "relation" => anyhow::ensure!(
            matches!(
                value.as_str(),
                Some("active") | Some("archived") | Some("legacy")
            ),
            "invalid metadata value for '{key}'"
        ),
        _ => anyhow::bail!("unknown metadata field '{key}'"),
    }
    Ok(())
}

fn validate_record_table_for_read(
    table: &toml::Table,
    metadata_fields: &[MetadataFieldConfig],
) -> Result<()> {
    for field in CORE_FIELDS {
        if !table.contains_key(*field) {
            anyhow::bail!("Metadata record is incomplete");
        }
    }
    match table.get("schema-version").and_then(|v| v.as_integer()) {
        Some(SCHEMA_VERSION) => {}
        _ => anyhow::bail!("Invalid metadata for current note"),
    }
    ensure_array_string(table, "aliases")?;
    ensure_array_string(table, "keywords")?;
    ensure_array_string(table, "relation-target")?;
    ensure_string(table, "abstract")?;
    ensure_bool(table, "generated")?;
    ensure_enum(table, "checklist-status", &["none", "todo", "wip", "done"])?;
    ensure_enum(table, "relation", &["active", "archived", "legacy"])?;

    let user = table.get("user");
    let user_table = user.and_then(|value| value.as_table());
    if user.is_some() && user_table.is_none() {
        anyhow::bail!("Invalid metadata for current note");
    }
    for field in metadata_fields {
        let Some(user_key) = field.path.strip_prefix("user.") else {
            continue;
        };
        let Some(value) = user_table.and_then(|table| table.get(user_key)) else {
            anyhow::bail!("Metadata record is incomplete");
        };
        if !value_matches_kind(value, &field.kind) {
            anyhow::bail!("Invalid metadata for current note");
        }
    }
    Ok(())
}

fn ensure_array_string(table: &toml::Table, field: &str) -> Result<()> {
    let valid = table
        .get(field)
        .and_then(|value| value.as_array())
        .map(|items| items.iter().all(|item| item.as_str().is_some()))
        .unwrap_or(false);
    anyhow::ensure!(valid, "Invalid metadata for current note");
    Ok(())
}

fn ensure_string(table: &toml::Table, field: &str) -> Result<()> {
    anyhow::ensure!(
        table.get(field).and_then(|value| value.as_str()).is_some(),
        "Invalid metadata for current note"
    );
    Ok(())
}

fn ensure_bool(table: &toml::Table, field: &str) -> Result<()> {
    anyhow::ensure!(
        table.get(field).and_then(|value| value.as_bool()).is_some(),
        "Invalid metadata for current note"
    );
    Ok(())
}

fn ensure_enum(table: &toml::Table, field: &str, values: &[&str]) -> Result<()> {
    let valid = table
        .get(field)
        .and_then(|value| value.as_str())
        .map(|value| values.contains(&value))
        .unwrap_or(false);
    anyhow::ensure!(valid, "Invalid metadata for current note");
    Ok(())
}

fn value_matches_kind(value: &toml::Value, kind: &MetadataFieldKind) -> bool {
    match kind {
        MetadataFieldKind::String => value.as_str().is_some(),
        MetadataFieldKind::Boolean => value.as_bool().is_some(),
        MetadataFieldKind::ArrayString => value
            .as_array()
            .map(|items| items.iter().all(|item| item.as_str().is_some()))
            .unwrap_or(false),
    }
}

fn canonical_record_table(table: &toml::Table) -> toml::Table {
    let mut out = toml::Table::new();
    for key in CORE_FIELDS {
        if let Some(value) = table.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    for (key, value) in table {
        if !CORE_FIELDS.contains(&key.as_str()) && key != "user" {
            out.insert(key.clone(), value.clone());
        }
    }
    if let Some(user) = table.get("user") {
        out.insert("user".into(), user.clone());
    }
    out
}

async fn read_index_table(config: &crate::config::WikiConfig) -> Result<toml::Table> {
    let path = metadata_path(&config.root);
    let raw = fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let table = raw
        .parse::<toml::Table>()
        .with_context(|| format!("parsing {}", path.display()))?;
    match table.get("format-version").and_then(|v| v.as_integer()) {
        None => anyhow::bail!("Missing format-version"),
        Some(version) if version != FORMAT_VERSION => {
            anyhow::bail!("Unsupported metadata format-version");
        }
        Some(_) => {}
    }
    Ok(table)
}

async fn write_index_table(path: &Path, root: &toml::Table) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let content = render_index(root);
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, content)
        .await
        .with_context(|| format!("writing tmp file {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .await
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn render_index(root: &toml::Table) -> String {
    let mut out = String::new();
    out.push_str("format-version = 1\n\n");

    for (key, value) in root {
        if key == "format-version" || key == "notes" {
            continue;
        }
        out.push_str(&format!("{} = {}\n", render_key(key), render_value(value)));
    }
    if root
        .keys()
        .any(|key| key != "format-version" && key != "notes")
    {
        out.push('\n');
    }

    if let Some(notes) = root.get("notes").and_then(|v| v.as_table()) {
        let mut scalar_keys: Vec<&String> = notes
            .iter()
            .filter_map(|(key, value)| (!value.is_table()).then_some(key))
            .collect();
        scalar_keys.sort();
        if !scalar_keys.is_empty() {
            out.push_str("[notes]\n");
            for key in scalar_keys {
                if let Some(value) = notes.get(key) {
                    out.push_str(&format!("{} = {}\n", render_key(key), render_value(value)));
                }
            }
            out.push('\n');
        }

        let mut ids: Vec<&String> = notes
            .iter()
            .filter_map(|(id, value)| value.is_table().then_some(id))
            .collect();
        ids.sort();
        for id in ids {
            let Some(record) = notes.get(id).and_then(|v| v.as_table()) else {
                continue;
            };
            out.push_str(&format!("[notes.{}]\n", render_note_key(id)));
            for key in CORE_FIELDS {
                if let Some(value) = record.get(*key) {
                    out.push_str(&format!("{} = {}\n", render_key(key), render_value(value)));
                }
            }
            for (key, value) in record {
                if !CORE_FIELDS.contains(&key.as_str()) && key != "user" {
                    out.push_str(&format!("{} = {}\n", render_key(key), render_value(value)));
                }
            }
            if let Some(user) = record.get("user").and_then(|v| v.as_table()) {
                if !user.is_empty() {
                    out.push('\n');
                    out.push_str(&format!("[notes.{}.user]\n", render_note_key(id)));
                    for (key, value) in user {
                        out.push_str(&format!("{} = {}\n", render_key(key), render_value(value)));
                    }
                }
            }
            out.push('\n');
        }
    }

    out
}

fn render_note_key(key: &str) -> String {
    format!("{key:?}")
}

fn render_key(key: &str) -> String {
    if key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        key.to_string()
    } else {
        format!("{key:?}")
    }
}

fn render_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => format!("{s:?}"),
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(n) => n.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(values) => {
            let items = values
                .iter()
                .map(render_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{items}]")
        }
        toml::Value::Datetime(dt) => dt.to_string(),
        toml::Value::Table(table) => {
            let items = table
                .iter()
                .map(|(k, v)| format!("{k} = {}", render_value(v)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {items} }}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WikiConfig;
    use crate::config::{MetadataConfig, MetadataFieldKind};

    #[test]
    fn render_index_quotes_note_record_keys() {
        let mut record = default_record_table(&ZkLspConfig::default());
        record.insert(
            "user".into(),
            toml::Value::Table(
                [("project".to_string(), toml::Value::String("zk".into()))]
                    .into_iter()
                    .collect(),
            ),
        );
        let mut notes = toml::Table::new();
        notes.insert("2607081902".into(), toml::Value::Table(record));
        let root = [
            (
                "format-version".to_string(),
                toml::Value::Integer(FORMAT_VERSION),
            ),
            ("notes".to_string(), toml::Value::Table(notes)),
        ]
        .into_iter()
        .collect();

        let rendered = render_index(&root);

        assert!(rendered.contains("[notes.\"2607081902\"]"));
        assert!(rendered.contains("[notes.\"2607081902\".user]"));
        assert!(!rendered.contains("[notes.2607081902]"));
    }

    #[test]
    fn find_record_key_range_accepts_quoted_and_bare_note_keys() {
        let quoted = "format-version = 1\n\n[notes.\"2607081902\"]\nrelation-target = []\n";
        let bare = "format-version = 1\n\n[notes.2607081902]\nrelation-target = []\n";

        assert_eq!(
            find_record_key_range(quoted, "2607081902"),
            Some(MetadataTextRange {
                line: 2,
                start_col: 8,
                end_col: 18
            })
        );
        assert_eq!(
            find_record_key_range(bare, "2607081902"),
            Some(MetadataTextRange {
                line: 2,
                start_col: 7,
                end_col: 17
            })
        );
    }

    #[test]
    fn complete_record_table_merges_core_and_user_defaults_without_overwriting() {
        let config = ZkLspConfig {
            metadata: MetadataConfig {
                fields: vec![MetadataFieldConfig {
                    path: "user.project".into(),
                    kind: MetadataFieldKind::String,
                    default: toml::Value::String("default-project".into()),
                }],
            },
            ..ZkLspConfig::default()
        };
        let mut table = toml::Table::new();
        table.insert("relation".into(), toml::Value::String("archived".into()));
        table.insert(
            "user".into(),
            toml::Value::Table(
                [(
                    "project".to_string(),
                    toml::Value::String("manual-project".into()),
                )]
                .into_iter()
                .collect(),
            ),
        );

        complete_record_table(&mut table, &config);

        assert_eq!(
            table.get("relation").and_then(|v| v.as_str()),
            Some("archived")
        );
        assert_eq!(
            table
                .get("user")
                .and_then(|v| v.as_table())
                .and_then(|t| t.get("project"))
                .and_then(|v| v.as_str()),
            Some("manual-project")
        );
        assert!(table.contains_key("schema-version"));
        assert!(table.contains_key("checklist-status"));
    }

    #[test]
    fn apply_patch_to_table_validates_core_and_user_values() {
        let fields = vec![MetadataFieldConfig {
            path: "user.reviewed".into(),
            kind: MetadataFieldKind::Boolean,
            default: toml::Value::Boolean(false),
        }];
        let mut table = default_record_table(&ZkLspConfig {
            metadata: MetadataConfig {
                fields: fields.clone(),
            },
            ..ZkLspConfig::default()
        });

        let invalid_status = HashMap::from([(
            "checklist-status".to_string(),
            toml::Value::String("blocked".into()),
        )]);
        assert!(apply_patch_to_table(&mut table, &invalid_status, &fields).is_err());

        let invalid_targets = HashMap::from([(
            "relation-target".to_string(),
            toml::Value::Array(vec![toml::Value::Integer(1)]),
        )]);
        assert!(apply_patch_to_table(&mut table, &invalid_targets, &fields).is_err());

        let invalid_user = HashMap::from([(
            "user.reviewed".to_string(),
            toml::Value::String("yes".into()),
        )]);
        assert!(apply_patch_to_table(&mut table, &invalid_user, &fields).is_err());

        let valid_user = HashMap::from([("user.reviewed".to_string(), toml::Value::Boolean(true))]);
        apply_patch_to_table(&mut table, &valid_user, &fields).unwrap();
        assert_eq!(
            table
                .get("user")
                .and_then(|value| value.as_table())
                .and_then(|user| user.get("reviewed"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn metadata_snapshot_keeps_valid_records_and_per_record_errors() {
        let valid = default_record_table(&ZkLspConfig::default());
        let mut invalid = default_record_table(&ZkLspConfig::default());
        invalid.insert(
            "checklist-status".into(),
            toml::Value::String("blocked".into()),
        );
        let mut notes = toml::Table::new();
        notes.insert("2607081902".into(), toml::Value::Table(valid));
        notes.insert("2607081903".into(), toml::Value::Table(invalid));
        let root = toml::Table::from_iter([
            (
                "format-version".to_string(),
                toml::Value::Integer(FORMAT_VERSION),
            ),
            ("notes".to_string(), toml::Value::Table(notes)),
        ]);

        let snapshot = MetadataSnapshot::from_index_table(&root, &[]).unwrap();

        assert!(snapshot.record("2607081902").is_ok());
        assert_eq!(
            snapshot.record("2607081903").unwrap_err().to_string(),
            "Invalid metadata for current note"
        );
        assert_eq!(
            snapshot.record("2607089999").unwrap_err().to_string(),
            "Missing metadata for current note"
        );
    }

    #[tokio::test]
    async fn write_paths_do_not_replace_structurally_invalid_metadata_index() {
        let root = std::env::temp_dir().join(format!("zk-lsp-metadata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = metadata_path(&root);
        let original = "notes = {}\n";
        std::fs::write(&path, original).unwrap();
        let config = WikiConfig::from_root(root.clone());

        let err = put_record_table(
            &config,
            "2607081902",
            &default_record_table(&config.zk_config),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("Missing format-version"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn read_record_table_or_default_only_defaults_missing_file_or_record() {
        let root =
            std::env::temp_dir().join(format!("zk-lsp-metadata-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config = WikiConfig::from_root(root.clone());

        let missing_file = read_record_table_or_default(&config, "2607081902")
            .await
            .unwrap();
        assert_eq!(
            missing_file
                .get("checklist-status")
                .and_then(|value| value.as_str()),
            Some("none")
        );

        std::fs::write(metadata_path(&root), "format-version = 1\n\n[notes]\n").unwrap();
        let missing_record = read_record_table_or_default(&config, "2607081902")
            .await
            .unwrap();
        assert_eq!(
            missing_record
                .get("checklist-status")
                .and_then(|value| value.as_str()),
            Some("none")
        );

        std::fs::write(metadata_path(&root), "notes = {}\n").unwrap();
        let err = read_record_table_or_default(&config, "2607081902")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing format-version"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
