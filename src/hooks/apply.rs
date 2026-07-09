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

pub fn apply_hook_text_edits(result: &HookResult, content: &str) -> anyhow::Result<String> {
    validate_hook_result(result, content)?;
    if result.edits.is_empty() {
        return Ok(content.to_string());
    }
    let edits_as_tuples: Vec<(usize, usize, String)> = result
        .edits
        .iter()
        .map(|e| (e.start_byte, e.end_byte, e.text.clone()))
        .collect();
    formatting::apply_byte_edits(content, &edits_as_tuples)
}
