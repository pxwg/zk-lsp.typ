use super::types::HookResult;
use crate::handlers::formatting;

/// Validate that all edits in `result` are within `content` and non-overlapping.
pub fn validate_hook_result(result: &HookResult, content: &str) -> anyhow::Result<()> {
    let len = content.len();
    for edit in &result.edits {
        anyhow::ensure!(
            edit.start_byte <= edit.end_byte,
            "edit has start_byte ({}) > end_byte ({})",
            edit.start_byte,
            edit.end_byte
        );
        anyhow::ensure!(
            edit.end_byte <= len,
            "edit end_byte ({}) out of bounds (content len={})",
            edit.end_byte,
            len
        );
    }
    // Check overlaps
    let mut sorted: Vec<&super::types::HookTextEdit> = result.edits.iter().collect();
    sorted.sort_by_key(|e| e.start_byte);
    for w in sorted.windows(2) {
        anyhow::ensure!(
            w[0].end_byte <= w[1].start_byte,
            "edits overlap: [{}, {}) and [{}, {})",
            w[0].start_byte,
            w[0].end_byte,
            w[1].start_byte,
            w[1].end_byte
        );
    }
    Ok(())
}

/// Apply a `HookResult` to `content`.
///
/// Pipeline:
/// 1. If `result.metadata` is non-empty: apply metadata patch.
/// 2. If `result.edits` is non-empty: apply byte edits.
///
/// The Rust normalizer (`normalize_note`) is intentionally NOT called here —
/// the Lua hook is fully responsible for formatting.
/// For global cross-file state reconciliation, use the `reconcile` command.
pub fn apply_hook_result(result: &HookResult, content: &str) -> anyhow::Result<String> {
    validate_hook_result(result, content)?;

    // Step 1: byte edits — applied first so their byte offsets (relative to original
    // content) are correct before the metadata patch may shift bytes in the TOML block.
    let after_edits = if !result.edits.is_empty() {
        let edits_as_tuples: Vec<(usize, usize, String)> = result
            .edits
            .iter()
            .map(|e| (e.start_byte, e.end_byte, e.text.clone()))
            .collect();
        formatting::apply_byte_edits(content, &edits_as_tuples)?
    } else {
        content.to_string()
    };

    // Step 2: metadata patch — uses pattern matching within the TOML block, so it is
    // unaffected by body edits applied above (which are outside the TOML block).
    let result_content = if !result.metadata.is_empty() {
        formatting::apply_metadata_patch(&after_edits, &result.metadata)?
    } else {
        after_edits
    };

    Ok(result_content)
}
