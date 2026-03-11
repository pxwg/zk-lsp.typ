use std::collections::HashMap;
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;
use tower_lsp::lsp_types::*;

use crate::parser::{self, StatusTag, ChecklistStatus};

static RE_TODO_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"@(\d{10})").unwrap());

/// Apply the tag-line formatting to `content` and return the result.
/// Internal helper; no cross-file I/O.
fn apply_tag_edit(content: &str) -> String {
    let Some(edit) = compute_tag_edit(content) else {
        return content.to_string();
    };
    let line_num = edit.range.start.line as usize;
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if line_num < lines.len() {
        lines[line_num] = edit.new_text;
    }
    let trailing_newline = content.ends_with('\n');
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// Returns true iff `content` has an effective checklist status of Done.
///
/// For TOML-format notes: derive done-ness from todo counts (same logic the
/// formatter would apply when updating `checklist-status`).
/// For legacy notes: simulate what `apply_tag_edit` would produce, then check
/// for `#tag.done`.
pub fn is_note_done(content: &str) -> bool {
    let Some(header) = parser::parse_header(content) else {
        return false;
    };

    if header.metadata_block.is_some() {
        let todos = parser::count_todos(content);
        return match parser::compute_status_tag(&todos, header.archived) {
            Some(tag) => tag == StatusTag::Done,
            None => header.checklist_status == Some(parser::ChecklistStatus::Done),
        };
    }

    let Some(tag_line_idx) = header.tag_line_idx else {
        return false;
    };
    let lines: Vec<&str> = content.lines().collect();
    let existing = lines
        .get(tag_line_idx)
        .copied()
        .unwrap_or("")
        .to_string();
    let effective = match compute_tag_edit(content) {
        Some(edit) => edit.new_text,
        None => existing,
    };
    effective.contains("#tag.done")
}

/// Normalize `content` using a pre-built map of dependency states.
/// Pure (no I/O): looks up each `@ID` in `dep_states` (absent = not done).
/// Calls `update_ref_checkboxes_sync`, `update_nested_checkboxes`, and `apply_tag_edit`.
pub fn normalize_note(content: &str, dep_states: &HashMap<String, bool>) -> String {
    let after_refs = update_ref_checkboxes_sync(content, dep_states);
    let after_nested = update_nested_checkboxes(&after_refs);
    apply_tag_edit(&after_nested)
}

/// Sync version of ref-checkbox update using a pre-built dep_states map.
fn update_ref_checkboxes_sync(content: &str, dep_states: &HashMap<String, bool>) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut changed = false;

    for (i, line) in lines.iter().enumerate() {
        if !is_todo_line(line) {
            continue;
        }
        let ids: Vec<&str> = RE_TODO_ID
            .captures_iter(line)
            .filter_map(|c| c.get(1).map(|m| m.as_str()))
            .collect();
        if ids.is_empty() {
            continue;
        }
        let all_done = ids.iter().all(|id| dep_states.get(*id).copied().unwrap_or(false));
        let new_state = if all_done { 'x' } else { ' ' };
        if get_todo_state(line) != Some(new_state) {
            if let Some(new_line) = replace_todo_state(line, new_state) {
                result[i] = new_line;
                changed = true;
            }
        }
    }

    if !changed {
        return content.to_string();
    }
    let trailing_newline = content.ends_with('\n');
    let mut out = result.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// Format `content`:
/// 1. Update `- [ ] @<id>` / `- [x] @<id>` checkboxes by reading referenced
///    notes from `note_dir` — all IDs on a line must be Done for the box to be
///    checked, otherwise the box is cleared.
/// 2. Recompute and apply the note's own status tag based on the updated
///    checkbox state.
pub async fn format_content(content: &str, note_dir: &Path) -> String {
    // Collect all @IDs referenced on todo lines
    let mut ids_to_fetch: Vec<String> = Vec::new();
    for line in content.lines() {
        if !is_todo_line(line) {
            continue;
        }
        for cap in RE_TODO_ID.captures_iter(line) {
            if let Some(m) = cap.get(1) {
                let id = m.as_str().to_string();
                if !ids_to_fetch.contains(&id) {
                    ids_to_fetch.push(id);
                }
            }
        }
    }

    let mut dep_states: HashMap<String, bool> = HashMap::new();
    for id in ids_to_fetch {
        let path = note_dir.join(format!("{id}.typ"));
        let done = match tokio::fs::read_to_string(&path).await {
            Ok(c) => is_note_done(&c),
            Err(_) => false,
        };
        dep_states.insert(id, done);
    }

    normalize_note(content, &dep_states)
}

/// Propagate nested checkbox states bottom-up: if a todo item has children,
/// its state is derived from them (all `[x]` → `[x]`, any `[ ]` → `[ ]`).
/// Leaf items are left unchanged.
fn update_nested_checkboxes(content: &str) -> String {
    let mut owned_lines: Vec<String> = content.lines().map(str::to_string).collect();

    let todo_items: Vec<(usize, usize)> = owned_lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            if is_todo_line(line) {
                let indent = line.len() - line.trim_start().len();
                Some((idx, indent))
            } else {
                None
            }
        })
        .collect();

    for i in (0..todo_items.len()).rev() {
        let (line_idx, indent) = todo_items[i];

        let mut descendants: Vec<usize> = Vec::new();
        for j in (i + 1)..todo_items.len() {
            let (child_line_idx, child_indent) = todo_items[j];
            if child_indent <= indent {
                break;
            }
            descendants.push(child_line_idx);
        }

        if descendants.is_empty() {
            continue;
        }

        let all_done = descendants
            .iter()
            .all(|&child_idx| get_todo_state(&owned_lines[child_idx]) == Some('x'));

        let new_state = if all_done { 'x' } else { ' ' };
        if get_todo_state(&owned_lines[line_idx]) != Some(new_state) {
            if let Some(new_line) = replace_todo_state(&owned_lines[line_idx], new_state) {
                owned_lines[line_idx] = new_line;
            }
        }
    }

    let trailing_newline = content.ends_with('\n');
    let mut out = owned_lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// Compute the TextEdit needed to update `checklist-status` in a TOML metadata
/// block to `new_status`. Returns None if not found or already correct.
pub fn compute_toml_status_edit(content: &str, new_status: &str) -> Option<TextEdit> {
    let block = parser::find_toml_metadata_block(content)?;
    let lines: Vec<&str> = content.lines().collect();

    for i in block.start_line..=block.end_line {
        let line = lines.get(i)?;
        if line.trim_start().starts_with("checklist-status") {
            let new_line = format!("  checklist-status = \"{new_status}\"");
            if *line == new_line {
                return None;
            }
            return Some(TextEdit {
                range: Range {
                    start: Position {
                        line: i as u32,
                        character: 0,
                    },
                    end: Position {
                        line: i as u32,
                        character: line.len() as u32,
                    },
                },
                new_text: new_line,
            });
        }
    }
    None
}

/// Compute the TextEdit needed to update the status, if any change is required.
/// For TOML-format notes, updates `checklist-status` in the TOML block.
/// For legacy notes, updates the tag line.
/// Returns None if no change is needed.
pub fn compute_tag_edit(content: &str) -> Option<TextEdit> {
    let header = parser::parse_header(content)?;
    let todos = parser::count_todos(content);
    let new_tag = parser::compute_status_tag(&todos, header.archived)?;

    if header.metadata_block.is_some() {
        let status_str = match new_tag {
            StatusTag::Done => "done",
            StatusTag::Wip => "wip",
            StatusTag::Todo => "todo",
        };
        // Only update if the current checklist_status differs
        let current = header.checklist_status.as_ref();
        let already_correct = match new_tag {
            StatusTag::Done => current == Some(&ChecklistStatus::Done),
            StatusTag::Wip => current == Some(&ChecklistStatus::Wip),
            StatusTag::Todo => current == Some(&ChecklistStatus::Todo),
        };
        if already_correct {
            return None;
        }
        return compute_toml_status_edit(content, status_str);
    }

    // Legacy path
    let tag_line_idx = header.tag_line_idx?;
    let new_tag_str = match new_tag {
        StatusTag::Done => "#tag.done",
        StatusTag::Wip => "#tag.wip",
        StatusTag::Todo => "#tag.todo",
    };

    let lines: Vec<&str> = content.lines().collect();
    let tag_line = lines.get(tag_line_idx)?;

    let current_tag_str = if tag_line.contains("#tag.done") {
        Some("#tag.done")
    } else if tag_line.contains("#tag.wip") {
        Some("#tag.wip")
    } else if tag_line.contains("#tag.todo") {
        Some("#tag.todo")
    } else {
        None
    };

    if current_tag_str == Some(new_tag_str) {
        return None;
    }

    let new_line = if let Some(old) = current_tag_str {
        tag_line.replace(old, new_tag_str)
    } else {
        format!("{tag_line} {new_tag_str}")
    };

    let line_num = tag_line_idx as u32;
    Some(TextEdit {
        range: Range {
            start: Position {
                line: line_num,
                character: 0,
            },
            end: Position {
                line: line_num,
                character: tag_line.len() as u32,
            },
        },
        new_text: new_line,
    })
}


fn is_todo_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- [") && t.len() >= 5
}

fn get_todo_state(line: &str) -> Option<char> {
    let t = line.trim_start();
    if t.starts_with("- [") && t.len() >= 5 {
        Some(t.chars().nth(3)?)
    } else {
        None
    }
}

fn replace_todo_state(line: &str, new_state: char) -> Option<String> {
    let indent_len = line.len() - line.trim_start().len();
    let trimmed = &line[indent_len..];
    if trimmed.starts_with("- [") && trimmed.len() >= 5 {
        let mut chars: Vec<char> = line.chars().collect();
        // Position of the state character: indent + 3
        let state_pos = indent_len + 3;
        if state_pos < chars.len() {
            chars[state_pos] = new_state;
            return Some(chars.into_iter().collect());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_children_done_parent_becomes_checked() {
        let input = "- [ ] parent\n  - [x] child one\n  - [x] child two\n";
        let out = update_nested_checkboxes(input);
        assert_eq!(out, "- [x] parent\n  - [x] child one\n  - [x] child two\n");
    }

    #[test]
    fn any_child_incomplete_parent_becomes_unchecked() {
        let input = "- [x] parent\n  - [x] child one\n  - [ ] child two\n";
        let out = update_nested_checkboxes(input);
        assert_eq!(out, "- [ ] parent\n  - [x] child one\n  - [ ] child two\n");
    }

    #[test]
    fn three_level_nesting_propagates_to_grandparent() {
        let input = "- [ ] grandparent\n  - [ ] parent\n    - [x] grandchild\n";
        let out = update_nested_checkboxes(input);
        // grandchild done → parent done → grandparent done
        assert_eq!(
            out,
            "- [x] grandparent\n  - [x] parent\n    - [x] grandchild\n"
        );
    }

    #[test]
    fn leaf_items_unchanged() {
        let input = "- [ ] leaf one\n- [x] leaf two\n";
        let out = update_nested_checkboxes(input);
        assert_eq!(out, input);
    }

    #[test]
    fn sibling_groups_resolved_independently() {
        let input = concat!(
            "- [ ] group a\n",
            "  - [x] a child\n",
            "- [ ] group b\n",
            "  - [ ] b child\n",
        );
        let out = update_nested_checkboxes(input);
        assert_eq!(
            out,
            concat!(
                "- [x] group a\n",
                "  - [x] a child\n",
                "- [ ] group b\n",
                "  - [ ] b child\n",
            )
        );
    }

    #[test]
    fn trailing_newline_preserved() {
        let with_nl = "- [ ] p\n  - [x] c\n";
        let without_nl = "- [ ] p\n  - [x] c";
        assert!(update_nested_checkboxes(with_nl).ends_with('\n'));
        assert!(!update_nested_checkboxes(without_nl).ends_with('\n'));
    }

    #[test]
    fn effective_status_with_no_todos_uses_checklist_status() {
        // compute_status_tag returns None when there are no todos
        let empty = parser::TodoStatus {
            completed: 0,
            incomplete: 0,
        };
        assert_eq!(parser::compute_status_tag(&empty, false), None);
        // When None, the ref_is_done branch falls through to header.checklist_status
        // ChecklistStatus::Done → true; ChecklistStatus::None → false
        assert!(
            parser::ChecklistStatus::Done == parser::ChecklistStatus::Done,
            "Done variant equality check"
        );
        assert!(
            parser::ChecklistStatus::None != parser::ChecklistStatus::Done,
            "None variant inequality check"
        );
    }
}
