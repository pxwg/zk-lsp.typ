use tower_lsp::lsp_types::*;

use super::diagnostics::DiagnosticData;
use crate::{metadata, parser};

pub fn get_code_actions(uri: &Url, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    for diag in diagnostics {
        if diag.source.as_deref() != Some("zk-lsp") {
            continue;
        }
        let data: DiagnosticData = match diag
            .data
            .as_ref()
            .and_then(|d| serde_json::from_value(d.clone()).ok())
        {
            Some(d) => d,
            None => continue,
        };

        let Some(new_ids) = data.new_ids.clone() else {
            continue;
        };
        if new_ids.is_empty() {
            continue;
        }

        let old_text = format!("@{}", data.old_id);
        for new_id in &new_ids {
            let new_text = format!("@{new_id}");
            actions.push(make_replace_action(
                uri,
                diag,
                format!("Fix: Replace {old_text} with {new_text}"),
                new_text.clone(),
            ));
            let append_text = format!("{old_text} {new_text}");
            actions.push(make_replace_action(
                uri,
                diag,
                format!("Fix: Keep {old_text} and append {new_text}"),
                append_text,
            ));
        }

        if new_ids.len() > 1 {
            let all_text = new_ids
                .iter()
                .map(|id| format!("@{id}"))
                .collect::<Vec<_>>()
                .join(" ");
            actions.push(make_replace_action(
                uri,
                diag,
                format!("Fix: Replace {old_text} with all relation-target IDs"),
                all_text,
            ));
        }
    }

    actions
}

pub fn get_metadata_actions(uri: &Url, content: &str, range: Range) -> Vec<CodeActionOrCommand> {
    let Some(record_id) = metadata::current_record_id(content, range.start.line as usize) else {
        return Vec::new();
    };
    if range.end.line as usize
        > metadata_record_end_line(content, &record_id).unwrap_or(range.start.line as usize)
    {
        return Vec::new();
    }

    let parsed = content.parse::<toml::Table>().ok();
    let record = parsed
        .as_ref()
        .and_then(|root| root.get("notes"))
        .and_then(|notes| notes.as_table())
        .and_then(|notes| notes.get(&record_id))
        .and_then(|record| record.as_table());

    let mut actions = Vec::new();

    let current_status = record
        .and_then(|record| record.get("checklist-status"))
        .and_then(|value| value.as_str())
        .and_then(parser::ChecklistStatus::from_str)
        .unwrap_or(parser::ChecklistStatus::None)
        .as_str();
    if let Some(status_line) =
        metadata::find_field_line_range(content, &record_id, "checklist-status")
    {
        for new_status in parser::ChecklistStatus::ALL.map(parser::ChecklistStatus::as_str) {
            if new_status == current_status {
                continue;
            }
            actions.push(make_metadata_line_action(
                uri,
                format!("ZK: Set checklist-status to {new_status}"),
                status_line,
                format!("checklist-status = \"{new_status}\""),
            ));
        }
    }

    let current_relation = record
        .and_then(|record| record.get("relation"))
        .and_then(|value| value.as_str())
        .unwrap_or("active");
    if let Some(relation_line) = metadata::find_field_line_range(content, &record_id, "relation") {
        if current_relation == "active" {
            for new_relation in ["archived", "legacy"] {
                actions.push(make_metadata_relation_action(
                    uri,
                    content,
                    &record_id,
                    relation_line,
                    new_relation,
                    format!(
                        "ZK: Mark as {}",
                        if new_relation == "archived" {
                            "archived"
                        } else {
                            "legacy"
                        }
                    ),
                ));
            }
        } else {
            actions.push(make_metadata_line_action(
                uri,
                "ZK: Mark as active".to_string(),
                relation_line,
                "relation = \"active\"".to_string(),
            ));
            let other = if current_relation == "archived" {
                "legacy"
            } else {
                "archived"
            };
            actions.push(make_metadata_relation_action(
                uri,
                content,
                &record_id,
                relation_line,
                other,
                format!("ZK: Mark as {other}"),
            ));
        }
    }

    actions
}

fn make_metadata_relation_action(
    uri: &Url,
    content: &str,
    record_id: &str,
    relation_line: metadata::MetadataTextRange,
    new_relation: &str,
    title: String,
) -> CodeActionOrCommand {
    let mut edits = vec![TextEdit {
        range: lsp_line_range(relation_line),
        new_text: format!("relation = \"{new_relation}\""),
    }];
    if metadata::find_field_line_range(content, record_id, "relation-target").is_none() {
        edits.push(TextEdit {
            range: Range {
                start: Position {
                    line: relation_line.line as u32 + 1,
                    character: 0,
                },
                end: Position {
                    line: relation_line.line as u32 + 1,
                    character: 0,
                },
            },
            new_text: "relation-target = [\"\"]\n".to_string(),
        });
    }
    make_workspace_action(uri, title, CodeActionKind::REFACTOR, edits)
}

fn make_metadata_line_action(
    uri: &Url,
    title: String,
    line_range: metadata::MetadataTextRange,
    new_text: String,
) -> CodeActionOrCommand {
    make_workspace_action(
        uri,
        title,
        CodeActionKind::REFACTOR,
        vec![TextEdit {
            range: lsp_line_range(line_range),
            new_text,
        }],
    )
}

fn make_workspace_action(
    uri: &Url,
    title: String,
    kind: CodeActionKind,
    edits: Vec<TextEdit>,
) -> CodeActionOrCommand {
    let workspace_edit = WorkspaceEdit {
        changes: Some([(uri.clone(), edits)].into_iter().collect()),
        ..Default::default()
    };
    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(kind),
        edit: Some(workspace_edit),
        ..Default::default()
    })
}

fn lsp_line_range(line_range: metadata::MetadataTextRange) -> Range {
    Range {
        start: Position {
            line: line_range.line as u32,
            character: line_range.start_col as u32,
        },
        end: Position {
            line: line_range.line as u32,
            character: line_range.end_col as u32,
        },
    }
}

fn metadata_record_end_line(content: &str, id: &str) -> Option<usize> {
    let mut in_record = false;
    let mut last_line = None;
    for (line_idx, line) in content.lines().enumerate() {
        if let Some((current, _, _, _)) = parse_record_header_for_actions(line) {
            if in_record && current != id {
                return last_line;
            }
            in_record = current == id;
        } else if in_record && line.trim_start().starts_with('[') && line.trim_end().ends_with(']')
        {
            return last_line;
        }
        if in_record {
            last_line = Some(line_idx);
        }
    }
    last_line
}

fn parse_record_header_for_actions(line: &str) -> Option<(String, usize, usize, bool)> {
    let trimmed = line.trim_start();
    let after_prefix = trimmed.strip_prefix("[notes.")?;
    let quote = after_prefix.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let after_quote = &after_prefix[1..];
    let end_rel = after_quote.find(quote as char)?;
    let id = &after_quote[..end_rel];
    if !metadata::is_note_id(id) {
        return None;
    }
    Some((id.to_string(), 0, line.len(), true))
}

fn make_replace_action(
    uri: &Url,
    diag: &Diagnostic,
    title: String,
    new_text: String,
) -> CodeActionOrCommand {
    let edit = WorkspaceEdit {
        changes: Some(
            [(
                uri.clone(),
                vec![TextEdit {
                    range: diag.range,
                    new_text,
                }],
            )]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    };
    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(edit),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const METADATA_ACTIVE: &str = concat!(
        "format-version = 1\n\n",
        "[notes.\"2603110000\"]\n",
        "schema-version = 1\n",
        "checklist-status = \"none\"\n",
        "relation = \"active\"\n",
        "relation-target = []\n",
    );

    fn make_uri() -> Url {
        Url::parse("file:///wiki/metadata.toml").unwrap()
    }

    #[test]
    fn metadata_actions_checklist_status_cycle() {
        let actions = get_metadata_actions(
            &make_uri(),
            METADATA_ACTIVE,
            Range {
                start: Position {
                    line: 4,
                    character: 0,
                },
                end: Position {
                    line: 4,
                    character: 0,
                },
            },
        );
        let titles: Vec<&str> = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca.title.as_str()),
                _ => None,
            })
            .collect();
        assert!(titles.iter().any(|t| t.contains("todo")));
        assert!(titles.iter().any(|t| t.contains("wip")));
        assert!(titles.iter().any(|t| t.contains("done")));
    }

    #[test]
    fn code_actions_offer_relation_target_rewrites() {
        let diagnostic = Diagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 6,
                },
                end: Position {
                    line: 0,
                    character: 17,
                },
            },
            source: Some("zk-lsp".into()),
            message: "Note @1111111111 is legacy. New ids: @2222222222, @3333333333".into(),
            data: Some(
                serde_json::to_value(DiagnosticData {
                    kind: "legacy".into(),
                    old_id: "1111111111".into(),
                    new_ids: Some(vec!["2222222222".into(), "3333333333".into()]),
                    replacement: None,
                })
                .unwrap(),
            ),
            ..Default::default()
        };
        let actions = get_code_actions(&make_uri(), &[diagnostic]);
        let titles = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca.title.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(titles.contains(&"Fix: Replace @1111111111 with @2222222222"));
        assert!(titles.contains(&"Fix: Replace @1111111111 with @3333333333"));
        assert!(titles.contains(&"Fix: Replace @1111111111 with all relation-target IDs"));
    }
}
