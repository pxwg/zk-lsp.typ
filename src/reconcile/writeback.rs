use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;
use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::handlers::formatting::compute_toml_status_edit;
use crate::parser::{self, ChecklistStatus, StatusTag};

static RE_TODO_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"@(\d{10})").unwrap());

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

pub fn is_note_done_with_deps(content: &str, deps: &HashMap<String, bool>) -> bool {
    let Some(header) = parser::parse_header(content) else {
        return false;
    };
    if header.archived {
        return true;
    }
    let items = parser::parse_checklist_items(content);
    if items.is_empty() {
        return header.checklist_status == Some(ChecklistStatus::Done);
    }
    parser::compute_note_done_from_items(&items, &|id| deps.get(id).copied().unwrap_or(false))
}

#[allow(dead_code)]
pub fn is_note_done(content: &str) -> bool {
    let Some(header) = parser::parse_header(content) else {
        return false;
    };
    if header.archived {
        return true;
    }
    match &header.checklist_status {
        Some(ChecklistStatus::Done) => return true,
        Some(ChecklistStatus::Todo) | Some(ChecklistStatus::Wip) => return false,
        _ => {}
    }
    is_note_done_with_deps(content, &HashMap::new())
}

#[allow(dead_code)]
pub fn normalize_note(content: &str, dep_states: &HashMap<String, bool>) -> String {
    let after_refs = update_ref_checkboxes_sync(content, dep_states);
    let after_nested = update_nested_checkboxes(&after_refs);
    apply_tag_edit(&after_nested)
}

pub fn normalize_note_from_checked(
    content: &str,
    checked_by_line: &HashMap<usize, bool>,
) -> String {
    let after_refs = update_ref_checkboxes_by_line(content, checked_by_line);
    let after_nested = update_nested_checkboxes(&after_refs);
    apply_tag_edit(&after_nested)
}

fn update_ref_checkboxes_sync(content: &str, dep_states: &HashMap<String, bool>) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut changed = false;
    let mut in_fence = false;

    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || !is_todo_line(line) {
            continue;
        }
        let ids: Vec<&str> = RE_TODO_ID
            .captures_iter(line)
            .filter_map(|c| c.get(1).map(|m| m.as_str()))
            .collect();
        if ids.is_empty() {
            continue;
        }
        let all_done = ids
            .iter()
            .all(|id| dep_states.get(*id).copied().unwrap_or(false));
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

fn update_ref_checkboxes_by_line(content: &str, checked_by_line: &HashMap<usize, bool>) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut changed = false;
    let mut in_fence = false;

    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || !is_todo_line(line) || !RE_TODO_ID.is_match(line) {
            continue;
        }
        if let Some(&checked) = checked_by_line.get(&i) {
            let new_state = if checked { 'x' } else { ' ' };
            if get_todo_state(line) != Some(new_state) {
                if let Some(new_line) = replace_todo_state(line, new_state) {
                    result[i] = new_line;
                    changed = true;
                }
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

fn update_nested_checkboxes(content: &str) -> String {
    let mut owned_lines: Vec<String> = content.lines().map(str::to_string).collect();

    let mut todo_items: Vec<(usize, usize)> = Vec::new();
    let mut in_fence = false;
    for (idx, line) in owned_lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if is_todo_line(line) {
            let indent = line.len() - trimmed.len();
            todo_items.push((idx, indent));
        }
    }

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

        let has_ref = RE_TODO_ID.is_match(&owned_lines[line_idx]);
        let ref_satisfied = !has_ref || get_todo_state(&owned_lines[line_idx]) == Some('x');
        let new_state = if all_done && ref_satisfied { 'x' } else { ' ' };
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
    fn fenced_checkboxes_are_not_modified() {
        let input = "- [ ] real item\n```\n- [ ] fake in fence\n```\n";
        let dep_states = HashMap::new();
        let after_refs = update_ref_checkboxes_sync(input, &dep_states);
        assert_eq!(after_refs, input);
        let after_nested = update_nested_checkboxes(input);
        assert_eq!(after_nested, input);
    }

    #[test]
    fn parent_ref_not_overridden_by_done_children() {
        let input = "- [ ] @1234567890 task\n  - [x] child\n";
        let dep_states = HashMap::new();
        let after_refs = update_ref_checkboxes_sync(input, &dep_states);
        let out = update_nested_checkboxes(&after_refs);
        assert!(out.starts_with("- [ ]"));
    }

    #[test]
    fn parent_ref_and_children_both_done_promotes_parent() {
        let input = "- [ ] @1234567890 task\n  - [ ] child\n";
        let dep_states = HashMap::from([("1234567890".to_string(), true)]);
        let after_refs = update_ref_checkboxes_sync(input, &dep_states);
        let out = update_nested_checkboxes(&after_refs);
        assert!(out.starts_with("- [ ]"));
    }

    #[test]
    fn effective_status_with_no_todos_uses_checklist_status() {
        let empty = parser::TodoStatus {
            completed: 0,
            incomplete: 0,
        };
        assert_eq!(parser::compute_status_tag(&empty, false), None);
        assert!(parser::ChecklistStatus::Done == parser::ChecklistStatus::Done);
        assert!(parser::ChecklistStatus::None != parser::ChecklistStatus::Done);
    }
}
