use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::config::{metadata_defaults_table, MetadataFieldKind, WikiConfig, ZkLspConfig};

const FORMAT_VERSION: i64 = 1;
const KIND_FIELDS: &str = "zk-lsp.config.metadata.fields";
const KIND_DEFAULTS: &str = "zk-lsp.config.metadata.defaults";
const KIND_JSON_SCHEMA: &str = "zk-lsp.config.metadata.json-schema";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataSchemaFormat {
    Json,
    Toml,
}

#[derive(Debug, Clone)]
struct FieldSpec {
    path: String,
    kind: &'static str,
    default: toml::Value,
    enum_values: Option<Vec<&'static str>>,
    required: bool,
    core: bool,
}

pub fn render_fields(
    config: &WikiConfig,
    format: MetadataSchemaFormat,
    include_sources: bool,
) -> Result<String> {
    match format {
        MetadataSchemaFormat::Json => render_json(&fields_json(config, include_sources)),
        MetadataSchemaFormat::Toml => render_toml(fields_toml(config, include_sources)),
    }
}

pub fn render_defaults(
    config: &WikiConfig,
    format: MetadataSchemaFormat,
    include_sources: bool,
) -> Result<String> {
    match format {
        MetadataSchemaFormat::Json => render_json(&defaults_json(config, include_sources)),
        MetadataSchemaFormat::Toml if include_sources => {
            render_toml(defaults_toml_envelope(config, include_sources))
        }
        MetadataSchemaFormat::Toml => render_toml(default_metadata_table(&config.zk_config)),
    }
}

pub fn render_json_schema(config: &WikiConfig, include_sources: bool) -> Result<String> {
    render_json(&json_schema_value(config, include_sources))
}

fn render_json(value: &Value) -> Result<String> {
    let mut out = serde_json::to_string_pretty(value)?;
    out.push('\n');
    Ok(out)
}

fn render_toml(table: toml::Table) -> Result<String> {
    let mut out = toml::to_string_pretty(&table)?;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn string_value(value: &str) -> toml::Value {
    toml::Value::String(value.to_string())
}

fn string_array(values: &[&str]) -> toml::Value {
    toml::Value::Array(values.iter().map(|v| string_value(v)).collect())
}

fn kind_name(kind: &MetadataFieldKind) -> &'static str {
    match kind {
        MetadataFieldKind::String => "string",
        MetadataFieldKind::Boolean => "boolean",
        MetadataFieldKind::ArrayString => "array-string",
    }
}

fn core_field_specs() -> Vec<FieldSpec> {
    vec![
        FieldSpec {
            path: "schema-version".into(),
            kind: "integer",
            default: toml::Value::Integer(1),
            enum_values: None,
            required: true,
            core: true,
        },
        FieldSpec {
            path: "aliases".into(),
            kind: "array-string",
            default: toml::Value::Array(Vec::new()),
            enum_values: None,
            required: true,
            core: true,
        },
        FieldSpec {
            path: "abstract".into(),
            kind: "string",
            default: string_value(""),
            enum_values: None,
            required: true,
            core: true,
        },
        FieldSpec {
            path: "keywords".into(),
            kind: "array-string",
            default: toml::Value::Array(Vec::new()),
            enum_values: None,
            required: true,
            core: true,
        },
        FieldSpec {
            path: "generated".into(),
            kind: "boolean",
            default: toml::Value::Boolean(true),
            enum_values: None,
            required: true,
            core: true,
        },
        FieldSpec {
            path: "checklist-status".into(),
            kind: "string",
            default: string_value("none"),
            enum_values: Some(vec!["none", "todo", "wip", "done"]),
            required: true,
            core: true,
        },
        FieldSpec {
            path: "relation".into(),
            kind: "string",
            default: string_value("active"),
            enum_values: Some(vec!["active", "archived", "legacy"]),
            required: true,
            core: true,
        },
        FieldSpec {
            path: "relation-target".into(),
            kind: "array-string",
            default: toml::Value::Array(Vec::new()),
            enum_values: None,
            required: true,
            core: true,
        },
    ]
}

fn all_field_specs(config: &ZkLspConfig) -> Vec<FieldSpec> {
    let mut fields = core_field_specs();
    fields.extend(config.metadata.fields.iter().map(|field| FieldSpec {
        path: field.path.clone(),
        kind: kind_name(&field.kind),
        default: field.default.clone(),
        enum_values: None,
        required: false,
        core: false,
    }));
    fields
}

fn default_metadata_table(config: &ZkLspConfig) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert("schema-version".into(), toml::Value::Integer(1));
    table.insert("aliases".into(), toml::Value::Array(Vec::new()));
    table.insert("abstract".into(), string_value(""));
    table.insert("keywords".into(), toml::Value::Array(Vec::new()));
    table.insert("generated".into(), toml::Value::Boolean(true));
    table.insert("checklist-status".into(), string_value("none"));
    table.insert("relation".into(), string_value("active"));
    table.insert("relation-target".into(), toml::Value::Array(Vec::new()));

    for (key, value) in metadata_defaults_table(&config.metadata.fields) {
        table.insert(key, value);
    }

    table
}

fn toml_value_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(n) => Value::Number((*n).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Array(values) => Value::Array(values.iter().map(toml_value_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
                .collect(),
        ),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
    }
}

fn sources_json(config: &WikiConfig) -> Value {
    Value::Array(
        ZkLspConfig::source_infos(&config.root)
            .into_iter()
            .map(|source| {
                json!({
                    "scope": source.scope,
                    "path": source.path.to_string_lossy().to_string(),
                    "loaded": source.loaded,
                })
            })
            .collect(),
    )
}

fn sources_toml(config: &WikiConfig) -> toml::Value {
    toml::Value::Array(
        ZkLspConfig::source_infos(&config.root)
            .into_iter()
            .map(|source| {
                let mut table = toml::Table::new();
                table.insert("scope".into(), string_value(source.scope));
                table.insert(
                    "path".into(),
                    string_value(source.path.to_string_lossy().as_ref()),
                );
                table.insert("loaded".into(), toml::Value::Boolean(source.loaded));
                toml::Value::Table(table)
            })
            .collect(),
    )
}

fn field_spec_json(field: &FieldSpec) -> Value {
    let mut object = Map::new();
    object.insert("path".into(), json!(field.path));
    object.insert("kind".into(), json!(field.kind));
    object.insert("default".into(), toml_value_to_json(&field.default));
    object.insert("required".into(), json!(field.required));
    object.insert("core".into(), json!(field.core));
    if let Some(enum_values) = &field.enum_values {
        object.insert("enum".into(), json!(enum_values));
    }
    Value::Object(object)
}

fn field_spec_toml(field: &FieldSpec) -> toml::Value {
    let mut table = toml::Table::new();
    table.insert("path".into(), string_value(&field.path));
    table.insert("kind".into(), string_value(field.kind));
    table.insert("default".into(), field.default.clone());
    table.insert("required".into(), toml::Value::Boolean(field.required));
    table.insert("core".into(), toml::Value::Boolean(field.core));
    if let Some(enum_values) = &field.enum_values {
        table.insert("enum".into(), string_array(enum_values));
    }
    toml::Value::Table(table)
}

fn fields_json(config: &WikiConfig, include_sources: bool) -> Value {
    let mut object = Map::new();
    object.insert("kind".into(), json!(KIND_FIELDS));
    object.insert("formatVersion".into(), json!(FORMAT_VERSION));
    if include_sources {
        object.insert("sources".into(), sources_json(config));
    }
    object.insert(
        "fields".into(),
        Value::Array(
            all_field_specs(&config.zk_config)
                .iter()
                .map(field_spec_json)
                .collect(),
        ),
    );
    Value::Object(object)
}

fn fields_toml(config: &WikiConfig, include_sources: bool) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert("kind".into(), string_value(KIND_FIELDS));
    table.insert(
        "format-version".into(),
        toml::Value::Integer(FORMAT_VERSION),
    );
    if include_sources {
        table.insert("sources".into(), sources_toml(config));
    }
    table.insert(
        "fields".into(),
        toml::Value::Array(
            all_field_specs(&config.zk_config)
                .iter()
                .map(field_spec_toml)
                .collect(),
        ),
    );
    table
}

fn defaults_json(config: &WikiConfig, include_sources: bool) -> Value {
    let mut object = Map::new();
    object.insert("kind".into(), json!(KIND_DEFAULTS));
    object.insert("formatVersion".into(), json!(FORMAT_VERSION));
    if include_sources {
        object.insert("sources".into(), sources_json(config));
    }
    object.insert(
        "metadata".into(),
        toml_value_to_json(&toml::Value::Table(default_metadata_table(
            &config.zk_config,
        ))),
    );
    Value::Object(object)
}

fn defaults_toml_envelope(config: &WikiConfig, include_sources: bool) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert("kind".into(), string_value(KIND_DEFAULTS));
    table.insert(
        "format-version".into(),
        toml::Value::Integer(FORMAT_VERSION),
    );
    if include_sources {
        table.insert("sources".into(), sources_toml(config));
    }
    table.insert(
        "metadata".into(),
        toml::Value::Table(default_metadata_table(&config.zk_config)),
    );
    table
}

fn json_schema_for_field(field: &FieldSpec) -> Value {
    let mut object = Map::new();
    match field.kind {
        "integer" => {
            object.insert("type".into(), json!("integer"));
        }
        "boolean" => {
            object.insert("type".into(), json!("boolean"));
        }
        "array-string" => {
            object.insert("type".into(), json!("array"));
            object.insert("items".into(), json!({ "type": "string" }));
        }
        _ => {
            object.insert("type".into(), json!("string"));
        }
    }
    if let Some(enum_values) = &field.enum_values {
        object.insert("enum".into(), json!(enum_values));
    }
    object.insert("default".into(), toml_value_to_json(&field.default));
    Value::Object(object)
}

fn json_schema_value(config: &WikiConfig, include_sources: bool) -> Value {
    let fields = all_field_specs(&config.zk_config);
    let mut properties = Map::new();
    let mut user_properties = Map::new();
    let mut required = Vec::new();

    for field in &fields {
        if field.required && field.core && !field.path.starts_with("user.") {
            required.push(Value::String(field.path.clone()));
        }

        if let Some(user_key) = field.path.strip_prefix("user.") {
            user_properties.insert(user_key.to_string(), json_schema_for_field(field));
        } else {
            properties.insert(field.path.clone(), json_schema_for_field(field));
        }
    }

    if !user_properties.is_empty() {
        properties.insert(
            "user".into(),
            json!({
                "type": "object",
                "properties": Value::Object(user_properties),
                "additionalProperties": true,
            }),
        );
    }

    let mut object = Map::new();
    object.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    object.insert("title".into(), json!("zk-lsp metadata"));
    object.insert("type".into(), json!("object"));
    object.insert("required".into(), Value::Array(required));
    object.insert("properties".into(), Value::Object(properties));
    object.insert("additionalProperties".into(), json!(true));

    let mut extension = Map::new();
    extension.insert("kind".into(), json!(KIND_JSON_SCHEMA));
    extension.insert("formatVersion".into(), json!(FORMAT_VERSION));
    if include_sources {
        extension.insert("sources".into(), sources_json(config));
    }
    object.insert("x-zk-lsp".into(), Value::Object(extension));

    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::config::{MetadataConfig, MetadataFieldConfig};

    fn wiki_config() -> WikiConfig {
        let zk_config = ZkLspConfig {
            metadata: MetadataConfig {
                fields: vec![
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
                ],
            },
            ..ZkLspConfig::default()
        };
        WikiConfig {
            root: PathBuf::from("/tmp/wiki"),
            note_dir: PathBuf::from("/tmp/wiki/note"),
            link_file: PathBuf::from("/tmp/wiki/link.typ"),
            zk_config,
        }
    }

    #[test]
    fn fields_include_core_and_custom_metadata() {
        let config = wiki_config();
        let fields = all_field_specs(&config.zk_config);
        let paths: Vec<&str> = fields.iter().map(|field| field.path.as_str()).collect();
        assert!(paths.contains(&"schema-version"));
        assert!(paths.contains(&"relation-target"));
        assert!(paths.contains(&"user.priority"));
        assert!(paths.contains(&"user.reviewed"));
    }

    #[test]
    fn defaults_include_custom_metadata_tree() {
        let config = wiki_config();
        let defaults = toml_value_to_json(&toml::Value::Table(default_metadata_table(
            &config.zk_config,
        )));
        assert_eq!(
            defaults.get("user"),
            Some(&json!({
                "priority": "normal",
                "reviewed": false
            }))
        );
    }

    #[test]
    fn json_schema_nests_custom_fields_under_user() {
        let config = wiki_config();
        let schema = json_schema_value(&config, false);
        assert_eq!(
            schema
                .get("properties")
                .and_then(|p| p.get("user"))
                .and_then(|u| u.get("properties"))
                .and_then(|p| p.get("priority"))
                .and_then(|p| p.get("type")),
            Some(&json!("string"))
        );
        assert_eq!(
            schema
                .get("properties")
                .and_then(|p| p.get("relation"))
                .and_then(|p| p.get("enum")),
            Some(&json!(["active", "archived", "legacy"]))
        );
    }
}
