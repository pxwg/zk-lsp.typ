use std::path::PathBuf;

use crate::config::MetadataFieldConfig;
use crate::config::WikiConfig;
use crate::hooks::apply::apply_hook_text_edits;
use crate::hooks::lua::{build_hook_note_input_with_metadata, HookRunner};
use crate::metadata;
use crate::parser;

/// Default hooks embedded at compile time.
const DEFAULT_CHECKLIST_HOOK: &str = include_str!("../../examples/hooks/checklist.lua");
const DEFAULT_RELATION_HOOK: &str = include_str!("../../examples/hooks/relation_status.lua");

#[allow(dead_code)]
/// Apply a list of byte-range edits to `content`.
///
/// Each edit is `(start_byte, end_byte, replacement_text)`.
/// Edits must be non-overlapping. They are applied from last to first so byte
/// offsets remain valid throughout.
///
/// Returns `Err` if any edit is invalid (out of bounds, inverted range, or overlap).
pub fn apply_byte_edits(content: &str, edits: &[(usize, usize, String)]) -> anyhow::Result<String> {
    let len = content.len();
    for (start, end, _) in edits {
        anyhow::ensure!(start <= end, "edit has start ({start}) > end ({end})");
        anyhow::ensure!(*end <= len, "edit end ({end}) out of bounds (len={len})");
    }
    // Sort by start ascending, check no overlaps
    let mut sorted: Vec<&(usize, usize, String)> = edits.iter().collect();
    sorted.sort_by_key(|(s, _, _)| *s);
    for w in sorted.windows(2) {
        anyhow::ensure!(
            w[0].1 <= w[1].0,
            "edits overlap: [{}, {}) and [{}, {})",
            w[0].0,
            w[0].1,
            w[1].0,
            w[1].1
        );
    }
    // Apply in reverse order so earlier byte offsets remain valid
    let mut result = content.to_string();
    for (start, end, text) in sorted.iter().rev() {
        result.replace_range(start..end, text);
    }
    Ok(result)
}

/// Format `content` by running hooks in sequence:
/// 1. Built-in default hooks (checklist.lua + relation_status.lua, embedded at compile time),
///    unless `config.zk_config.disable_default_hooks` is true.
/// 2. User-configured file hooks from `config.zk_config.hooks`, loaded at runtime.
///
/// Cross-file ref-checkbox sync (`@ID` items) is intentionally NOT performed here;
/// that is the exclusive responsibility of the `reconcile` command.
///
/// On any hook error the step is skipped and a warning is emitted; the original
/// content (or the output of the previous step) is passed through unchanged.
pub async fn format_content(content: &str, config: &WikiConfig) -> anyhow::Result<String> {
    let zk = &config.zk_config;
    let header = parser::parse_header(content)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid zk-metadata binding"))?;
    let mut metadata_table = metadata::read_record_table_or_default(config, &header.id).await?;
    metadata::complete_record_table(&mut metadata_table, zk);
    let mut current = content.to_string();
    if !zk.disable_default_hooks {
        current = run_default_hooks_central(&current, &mut metadata_table, &zk.metadata.fields);
    }
    current = run_hooks_central(
        &current,
        &mut metadata_table,
        &zk.hooks,
        &zk.metadata.fields,
    );
    metadata::put_record_table(config, &header.id, &metadata_table).await?;
    Ok(current)
}

fn run_default_hooks_central(
    content: &str,
    metadata_table: &mut toml::Table,
    metadata_fields: &[MetadataFieldConfig],
) -> String {
    let hooks: &[(&str, &str)] = &[
        ("checklist", DEFAULT_CHECKLIST_HOOK),
        ("relation_status", DEFAULT_RELATION_HOOK),
    ];
    let mut current = content.to_string();
    for (name, src) in hooks {
        let runner = match HookRunner::load_str(src) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("default hook '{name}' load error: {e}");
                continue;
            }
        };
        let input =
            build_hook_note_input_with_metadata(&current, metadata_table.clone(), metadata_fields);
        let result = match runner.run(&input) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("default hook '{name}' run error: {e}");
                continue;
            }
        };
        if let Err(e) =
            metadata::apply_patch_to_table(metadata_table, &result.metadata, metadata_fields)
        {
            tracing::warn!("default hook '{name}' metadata patch error: {e}");
        }
        match apply_hook_text_edits(&result, &current) {
            Ok(out) => current = out,
            Err(e) => tracing::warn!("default hook '{name}' apply error: {e}"),
        }
    }
    current
}

fn run_hooks_central(
    content: &str,
    metadata_table: &mut toml::Table,
    hook_paths: &[PathBuf],
    metadata_fields: &[MetadataFieldConfig],
) -> String {
    let mut current = content.to_string();
    for path in hook_paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let runner = match HookRunner::load_file(path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("hook '{name}' load error: {e}");
                continue;
            }
        };
        let input =
            build_hook_note_input_with_metadata(&current, metadata_table.clone(), metadata_fields);
        let result = match runner.run(&input) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("hook '{name}' run error: {e}");
                continue;
            }
        };
        if let Err(e) =
            metadata::apply_patch_to_table(metadata_table, &result.metadata, metadata_fields)
        {
            tracing::warn!("hook '{name}' metadata patch error: {e}");
        }
        match apply_hook_text_edits(&result, &current) {
            Ok(out) => current = out,
            Err(e) => tracing::warn!("hook '{name}' apply error: {e}"),
        }
    }
    current
}
