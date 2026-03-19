use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use tokio::fs;

use crate::config::{WikiConfig, ZkLspConfig};
use crate::link_gen;

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

/// Build the `#let zk-metadata = toml(bytes(…))` block, including any
/// user-defined metadata fields from config.
pub fn build_metadata_block(config: &ZkLspConfig) -> String {
    let mut lines: Vec<String> = vec![
        "#let zk-metadata = toml(bytes(".to_string(),
        "  ```toml".to_string(),
        "  schema-version = 1".to_string(),
        "  aliases = []".to_string(),
        "  abstract = \"\"".to_string(),
        "  keywords = []".to_string(),
        "  generated = true".to_string(),
        "  checklist-status = \"none\"".to_string(),
        "  relation = \"active\"".to_string(),
        "  relation-target = []".to_string(),
    ];

    // Collect user.* fields
    let user_fields: Vec<(&str, String)> = config
        .metadata
        .fields
        .iter()
        .filter_map(|f| {
            f.path
                .strip_prefix("user.")
                .map(|key| (key, toml_default_inline(&f.default)))
        })
        .collect();

    if !user_fields.is_empty() {
        lines.push(String::new()); // blank line before [user] section
        lines.push("  [user]".to_string());
        for (key, val) in user_fields {
            lines.push(format!("  {key} = {val}"));
        }
    }

    lines.push("  ```.text,".to_string());
    lines.push("))".to_string());
    lines.join("\n")
}

fn build_note_content(id: &str, config: &WikiConfig) -> String {
    let metadata_block = build_metadata_block(&config.zk_config);
    if let Some(tmpl) = &config.zk_config.new_note_template {
        return tmpl
            .replace("{{id}}", id)
            .replace("{{metadata}}", &metadata_block);
    }
    format!(
        "#import \"../include.typ\": *\n\
         {metadata_block}\n\
         #show: zettel.with(metadata: zk-metadata)\n\
         \n\
         =  <{id}>\n"
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

/// Create a new note, optionally using a caller-supplied ID.
/// Returns the path to the new file.
pub async fn create_note(config: &WikiConfig, custom_id: Option<String>) -> Result<PathBuf> {
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
        let content = build_note_content(&id, config);
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
    use crate::config::{MetadataConfig, MetadataFieldConfig, MetadataFieldKind, ZkLspConfig};
    use crate::parser;

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
        let block = build_metadata_block(&cfg);
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
        let block = build_metadata_block(&cfg);
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
