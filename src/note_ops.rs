use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use tokio::fs;

use crate::config::{WikiConfig, ZkLspConfig};
use crate::link_gen;

/// Map of metadata key → TOML value to override when building a new note.
///
/// Keys follow the same names as the TOML metadata block:
/// `checklist-status`, `relation`, `generated`, `aliases`, `keywords`,
/// `abstract`, `relation-target`, or `user.<field>` for user-defined fields.
pub type MetaOverrides = HashMap<String, toml::Value>;

/// Core metadata keys that may be overridden via `--meta`.
const CORE_OVERRIDABLE_KEYS: &[&str] = &[
    "checklist-status",
    "relation",
    "generated",
    "aliases",
    "keywords",
    "abstract",
    "relation-target",
];

/// Parse `KEY=VALUE` CLI strings into a [`MetaOverrides`] map.
///
/// Valid keys are the core metadata fields and any `user.*` fields declared in
/// `config`.  Unknown keys or undeclared `user.*` fields are rejected with an
/// error.
///
/// The VALUE portion is interpreted as a TOML literal.  Bare words (no
/// quotes, brackets, or `true`/`false`) are treated as string values so
/// `--meta checklist-status=todo` works without quoting.
pub fn parse_meta_overrides(meta: &[String], config: &ZkLspConfig) -> Result<MetaOverrides> {
    let declared_user_keys: Vec<&str> = config
        .metadata
        .fields
        .iter()
        .map(|f| f.path.as_str())
        .collect();

    let mut map = MetaOverrides::new();
    for item in meta {
        let (key, val_str) = item.split_once('=').ok_or_else(|| {
            anyhow::anyhow!(
                "invalid --meta argument {:?}: expected KEY=VALUE format",
                item
            )
        })?;

        // Validate key
        if key.starts_with("user.") {
            if !declared_user_keys.contains(&key) {
                anyhow::bail!(
                    "--meta key {:?} is not declared in config; \
                     add [[metadata.field]] path = {:?} to zk-lsp.toml first",
                    key,
                    key
                );
            }
        } else if !CORE_OVERRIDABLE_KEYS.contains(&key) {
            anyhow::bail!(
                "--meta key {:?} is not a valid metadata field; \
                 valid core fields: {}",
                key,
                CORE_OVERRIDABLE_KEYS.join(", ")
            );
        }

        map.insert(key.to_string(), parse_toml_value_str(val_str));
    }
    Ok(map)
}

/// Convert a [`serde_json::Value`] to a [`toml::Value`].
/// `null` maps to an empty string; objects map to TOML tables.
fn json_value_to_toml(v: serde_json::Value) -> toml::Value {
    match v {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else {
                toml::Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.into_iter().map(json_value_to_toml).collect())
        }
        serde_json::Value::Object(obj) => {
            let map = obj
                .into_iter()
                .map(|(k, v)| (k, json_value_to_toml(v)))
                .collect();
            toml::Value::Table(map)
        }
    }
}

/// Parse a JSON object string into metadata overrides, an optional title, and an optional body.
///
/// The JSON object may contain:
/// - `"metadata"`: object of `KEY: VALUE` pairs (same keys as `--meta`)
/// - `"title"`: string to use as the note heading
/// - `"content"`: string to append as the note body
///
/// Key validation follows the same rules as [`parse_meta_overrides`].
pub fn parse_json_creation_input(
    json_str: &str,
    config: &ZkLspConfig,
) -> Result<(MetaOverrides, Option<String>, Option<String>)> {
    let obj: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(json_str).context("--json: expected a JSON object")?;

    let title = obj
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let body = obj
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut overrides = MetaOverrides::new();
    if let Some(meta_val) = obj.get("metadata") {
        let meta_obj = meta_val
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("--json: \"metadata\" must be a JSON object"))?;

        let declared_user_keys: Vec<&str> = config
            .metadata
            .fields
            .iter()
            .map(|f| f.path.as_str())
            .collect();

        for (key, val) in meta_obj {
            if key.starts_with("user.") {
                if !declared_user_keys.contains(&key.as_str()) {
                    anyhow::bail!(
                        "--json metadata key {:?} is not declared in config; \
                         add [[metadata.field]] path = {:?} to zk-lsp.toml first",
                        key,
                        key
                    );
                }
            } else if !CORE_OVERRIDABLE_KEYS.contains(&key.as_str()) {
                anyhow::bail!(
                    "--json metadata key {:?} is not a valid metadata field; \
                     valid core fields: {}",
                    key,
                    CORE_OVERRIDABLE_KEYS.join(", ")
                );
            }
            overrides.insert(key.clone(), json_value_to_toml(val.clone()));
        }
    }

    Ok((overrides, title, body))
}

/// Parse a string as a TOML value literal.
/// Falls back to `String` if the input is not valid TOML syntax.
fn parse_toml_value_str(s: &str) -> toml::Value {
    let probe = format!("_x_ = {s}");
    if let Ok(mut table) = probe.parse::<toml::Table>() {
        if let Some(v) = table.remove("_x_") {
            return v;
        }
    }
    toml::Value::String(s.to_string())
}

/// Render a TOML default value as an inline TOML string.
fn toml_default_inline(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{}\"", escaped)
        }
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) if arr.is_empty() => "[]".to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    toml::Value::String(s) => format!("\"{}\"", s),
                    other => other.to_string(),
                })
                .collect();
            format!("[{}]", items.join(", "))
        }
        _ => "\"\"".to_string(),
    }
}

/// Build the `#let zk-metadata = toml(bytes(…))` block.
///
/// `overrides` is an optional map of key → TOML value that replaces the
/// defaults for the named field.  Keys follow the TOML metadata names
/// (`checklist-status`, `relation`, `user.priority`, …).  Fields not present
/// in `overrides` keep their normal defaults.
pub fn build_metadata_block(config: &ZkLspConfig, overrides: &MetaOverrides) -> String {
    // --- per-type helpers ---

    let ov_str = |key: &str, default: &str| -> String {
        match overrides.get(key).and_then(|v| v.as_str()) {
            Some(s) => {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }
            None => format!("\"{}\"", default),
        }
    };

    let ov_bool = |key: &str, default: bool| -> String {
        overrides
            .get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
            .to_string()
    };

    let ov_arr = |key: &str| -> String {
        match overrides.get(key).and_then(|v| v.as_array()) {
            Some(arr) => {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| match v {
                        toml::Value::String(s) => format!("\"{}\"", s),
                        other => other.to_string(),
                    })
                    .collect();
                format!("[{}]", items.join(", "))
            }
            None => "[]".to_string(),
        }
    };

    let mut lines: Vec<String> = vec![
        "#let zk-metadata = toml(bytes(".to_string(),
        "  ```toml".to_string(),
        "  schema-version = 1".to_string(),
        format!("  aliases = {}", ov_arr("aliases")),
        format!("  abstract = {}", ov_str("abstract", "")),
        format!("  keywords = {}", ov_arr("keywords")),
        format!("  generated = {}", ov_bool("generated", true)),
        format!(
            "  checklist-status = {}",
            ov_str("checklist-status", "none")
        ),
        format!("  relation = {}", ov_str("relation", "active")),
        format!("  relation-target = {}", ov_arr("relation-target")),
    ];

    // Collect user.* fields: config defaults, overridden where provided.
    let user_fields: Vec<(String, String)> = config
        .metadata
        .fields
        .iter()
        .filter_map(|f| {
            f.path.strip_prefix("user.").map(|sub_key| {
                let val = overrides
                    .get(&f.path)
                    .map(|v| toml_default_inline(v))
                    .unwrap_or_else(|| toml_default_inline(&f.default));
                (sub_key.to_string(), val)
            })
        })
        .collect();

    if !user_fields.is_empty() {
        lines.push(String::new()); // blank line before [user] section
        lines.push("  [user]".to_string());
        for (key, val) in &user_fields {
            lines.push(format!("  {key} = {val}"));
        }
    }

    lines.push("  ```.text,".to_string());
    lines.push("))".to_string());
    lines.join("\n")
}

fn build_note_content(
    id: &str,
    config: &WikiConfig,
    overrides: &MetaOverrides,
    title: Option<&str>,
    body: Option<&str>,
) -> String {
    let metadata_block = build_metadata_block(&config.zk_config, overrides);
    let heading = match title {
        Some(t) if !t.is_empty() => format!("= {t} <{id}>"),
        _ => format!("=  <{id}>"),
    };
    if let Some(tmpl) = &config.zk_config.new_note_template {
        return tmpl
            .replace("{{id}}", id)
            .replace("{{metadata}}", &metadata_block)
            .replace("{{title}}", title.unwrap_or(""))
            .replace("{{content}}", body.unwrap_or(""));
    }
    let body_section = match body {
        Some(b) if !b.is_empty() => format!("\n{b}"),
        _ => String::new(),
    };
    format!(
        "#import \"../include.typ\": *\n\
         {metadata_block}\n\
         #show: zettel.with(metadata: zk-metadata)\n\
         \n\
         {heading}\n{body_section}"
    )
}

/// Validate that a note ID is exactly 10 ASCII decimal digits.
pub fn validate_note_id(id: &str) -> Result<()> {
    if id.len() == 10 && id.chars().all(|c| c.is_ascii_digit()) {
        Ok(())
    } else {
        anyhow::bail!(
            "invalid note ID {:?}: must be exactly 10 decimal digits (YYMMDDHHMM)",
            id
        )
    }
}

/// Create a new note, optionally using a caller-supplied ID, metadata overrides,
/// a custom title, and a body to append after the heading.
/// Returns the path to the new file.
pub async fn create_note(
    config: &WikiConfig,
    custom_id: Option<String>,
    overrides: &MetaOverrides,
    title: Option<String>,
    body: Option<String>,
) -> Result<PathBuf> {
    let id = match custom_id {
        Some(id) => {
            validate_note_id(&id)?;
            id
        }
        None => Local::now().format("%y%m%d%H%M").to_string(),
    };
    fs::create_dir_all(&config.note_dir).await?;

    let path = config.note_dir.join(format!("{id}.typ"));
    if !path.exists() {
        let content = build_note_content(&id, config, overrides, title.as_deref(), body.as_deref());
        fs::write(&path, &content)
            .await
            .with_context(|| format!("writing note {}", path.display()))?;
    }

    link_gen::add_entry(&id, config).await?;
    Ok(path)
}

/// Delete a note and remove its entry from link.typ.
pub async fn delete_note(id: &str, config: &WikiConfig) -> Result<()> {
    let path = config.note_dir.join(format!("{id}.typ"));
    if path.exists() {
        fs::remove_file(&path)
            .await
            .with_context(|| format!("deleting note {}", path.display()))?;
    }
    link_gen::remove_entry(id, config).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_note_id_valid() {
        assert!(validate_note_id("2602110128").is_ok());
        assert!(validate_note_id("0000000000").is_ok());
        assert!(validate_note_id("9999999999").is_ok());
    }

    #[test]
    fn test_validate_note_id_too_short() {
        assert!(validate_note_id("260211012").is_err());
        assert!(validate_note_id("").is_err());
    }

    #[test]
    fn test_validate_note_id_too_long() {
        assert!(validate_note_id("26021101280").is_err());
    }

    #[test]
    fn test_validate_note_id_non_numeric() {
        assert!(validate_note_id("260211012x").is_err());
        assert!(validate_note_id("YYMMDDHHMM").is_err());
        assert!(validate_note_id("2602110128 ").is_err());
    }
    use crate::config::{
        MetadataConfig, MetadataFieldConfig, MetadataFieldKind, WikiConfig, ZkLspConfig,
    };
    use crate::parser;

    fn make_test_wiki(suffix: &str) -> (WikiConfig, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("zk_note_ops_test_{suffix}"));
        let config = WikiConfig::from_root(root.clone());
        (config, root)
    }

    #[tokio::test]
    async fn test_create_note_custom_id_produces_correct_filename() {
        let (config, _root) = make_test_wiki("custom_id_filename");
        let custom_id = "2602110128".to_string();
        let path = create_note(
            &config,
            Some(custom_id.clone()),
            &MetaOverrides::new(),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            path.file_name().unwrap(),
            std::ffi::OsStr::new(&format!("{custom_id}.typ"))
        );
        assert!(path.exists(), "created note file must exist on disk");
    }

    #[tokio::test]
    async fn test_create_note_custom_id_content_contains_id() {
        let (config, _root) = make_test_wiki("custom_id_content");
        let custom_id = "2602110129".to_string();
        let path = create_note(
            &config,
            Some(custom_id.clone()),
            &MetaOverrides::new(),
            None,
            None,
        )
        .await
        .unwrap();
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains(&custom_id),
            "note content must reference the custom ID"
        );
    }

    #[tokio::test]
    async fn test_create_note_invalid_id_is_rejected() {
        let (config, _root) = make_test_wiki("invalid_id");
        let err = create_note(
            &config,
            Some("badid".to_string()),
            &MetaOverrides::new(),
            None,
            None,
        )
        .await
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid note ID"),
            "error message should mention invalid note ID, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_create_note_no_id_generates_file() {
        let (config, _root) = make_test_wiki("auto_id");
        let path = create_note(&config, None, &MetaOverrides::new(), None, None)
            .await
            .unwrap();
        assert!(path.exists(), "auto-generated note file must exist on disk");
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name.len() == 14 && name.ends_with(".typ"),
            "filename should be 10-digit ID + .typ, got: {name}"
        );
    }

    fn config_with_fields(fields: Vec<MetadataFieldConfig>) -> ZkLspConfig {
        ZkLspConfig {
            new_note_template: None,
            metadata: MetadataConfig { fields },
            hooks: Vec::new(),
            reconcile_rules: Vec::new(),
            disable_default_hooks: false,
            disable_default_reconcile_rules: false,
        }
    }

    #[test]
    fn test_build_metadata_block_no_custom_fields() {
        let cfg = config_with_fields(vec![]);
        let block = build_metadata_block(&cfg, &MetaOverrides::new());
        assert!(block.contains("schema-version = 1"));
        assert!(block.contains("checklist-status = \"none\""));
        assert!(block.contains("relation = \"active\""));
        assert!(
            !block.contains("[user]"),
            "no [user] section when no custom fields"
        );
        // Should be parseable TOML
        let inner = extract_toml_from_block(&block).expect("should extract TOML");
        let parsed = parser::parse_toml_metadata(&inner).expect("should parse");
        assert_eq!(parsed.extra.len(), 0);
    }

    #[test]
    fn test_build_metadata_block_with_user_fields() {
        let cfg = config_with_fields(vec![
            MetadataFieldConfig {
                path: "user.course".into(),
                kind: MetadataFieldKind::String,
                default: toml::Value::String("".into()),
            },
            MetadataFieldConfig {
                path: "user.priority".into(),
                kind: MetadataFieldKind::String,
                default: toml::Value::String("normal".into()),
            },
            MetadataFieldConfig {
                path: "user.tags".into(),
                kind: MetadataFieldKind::ArrayString,
                default: toml::Value::Array(vec![]),
            },
        ]);
        let block = build_metadata_block(&cfg, &MetaOverrides::new());
        assert!(block.contains("[user]"));
        assert!(block.contains("course = \"\""));
        assert!(block.contains("priority = \"normal\""));
        assert!(block.contains("tags = []"));
        // Parse and verify extra fields are preserved
        let inner = extract_toml_from_block(&block).expect("should extract TOML");
        let parsed = parser::parse_toml_metadata(&inner).expect("should parse");
        assert!(
            parsed.extra.contains_key("user"),
            "user table should be in extra"
        );
    }

    fn empty_zk_config() -> ZkLspConfig {
        config_with_fields(vec![])
    }

    #[test]
    fn test_parse_meta_overrides_valid() {
        let meta = vec![
            "checklist-status=todo".to_string(),
            "relation=archived".to_string(),
        ];
        let overrides = parse_meta_overrides(&meta, &empty_zk_config()).unwrap();
        assert_eq!(
            overrides["checklist-status"].as_str(),
            Some("todo"),
            "bare word should be treated as string"
        );
        assert_eq!(overrides["relation"].as_str(), Some("archived"));
    }

    #[test]
    fn test_parse_meta_overrides_toml_array() {
        let meta = vec!["keywords=[\"a\", \"b\"]".to_string()];
        let overrides = parse_meta_overrides(&meta, &empty_zk_config()).unwrap();
        let arr = overrides["keywords"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str(), Some("a"));
        assert_eq!(arr[1].as_str(), Some("b"));
    }

    #[test]
    fn test_parse_meta_overrides_missing_eq() {
        assert!(parse_meta_overrides(&["noequalssign".to_string()], &empty_zk_config()).is_err());
    }

    #[test]
    fn test_parse_meta_overrides_unknown_core_key_rejected() {
        let err = parse_meta_overrides(&["bogus=foo".to_string()], &empty_zk_config())
            .unwrap_err()
            .to_string();
        assert!(err.contains("bogus"), "error should mention the bad key");
    }

    #[test]
    fn test_parse_meta_overrides_undeclared_user_key_rejected() {
        let cfg = empty_zk_config(); // no user fields declared
        let err = parse_meta_overrides(&["user.priority=high".to_string()], &cfg)
            .unwrap_err()
            .to_string();
        assert!(err.contains("user.priority"));
    }

    #[test]
    fn test_parse_meta_overrides_declared_user_key_accepted() {
        let cfg = config_with_fields(vec![MetadataFieldConfig {
            path: "user.priority".into(),
            kind: MetadataFieldKind::String,
            default: toml::Value::String("normal".into()),
        }]);
        let overrides = parse_meta_overrides(&["user.priority=high".to_string()], &cfg).unwrap();
        assert_eq!(overrides["user.priority"].as_str(), Some("high"));
    }

    #[test]
    fn test_build_metadata_block_with_overrides() {
        let cfg = config_with_fields(vec![]);
        let mut overrides = MetaOverrides::new();
        overrides.insert(
            "checklist-status".to_string(),
            toml::Value::String("todo".to_string()),
        );
        overrides.insert(
            "relation".to_string(),
            toml::Value::String("archived".to_string()),
        );
        let block = build_metadata_block(&cfg, &overrides);
        assert!(
            block.contains("checklist-status = \"todo\""),
            "override should replace default"
        );
        assert!(block.contains("relation = \"archived\""));
        // Non-overridden fields keep defaults
        assert!(block.contains("abstract = \"\""));
    }

    #[test]
    fn test_build_metadata_block_user_field_override() {
        let cfg = config_with_fields(vec![MetadataFieldConfig {
            path: "user.priority".into(),
            kind: MetadataFieldKind::String,
            default: toml::Value::String("normal".into()),
        }]);
        let mut overrides = MetaOverrides::new();
        overrides.insert(
            "user.priority".to_string(),
            toml::Value::String("high".to_string()),
        );
        let block = build_metadata_block(&cfg, &overrides);
        assert!(
            block.contains("priority = \"high\""),
            "config default should be overridden"
        );
    }

    /// Extract the TOML content from between ```toml and ``` fences.
    fn extract_toml_from_block(block: &str) -> Option<String> {
        let lines: Vec<&str> = block.lines().collect();
        let fence_start = lines.iter().position(|l| l.trim() == "```toml")?;
        let mut toml_lines = Vec::new();
        for line in &lines[fence_start + 1..] {
            if line.trim().starts_with("```") {
                break;
            }
            toml_lines.push(*line);
        }
        Some(toml_lines.join("\n"))
    }
}
