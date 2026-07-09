use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::NoteIndex;
use crate::{metadata, parser};

pub fn find_references(
    index: &Arc<NoteIndex>,
    uri: &Url,
    content: &str,
    position: Position,
    metadata_uri: &Url,
    metadata_content: &str,
    include_declaration: bool,
) -> Vec<Location> {
    let Some(id) = id_at_position(content, position) else {
        return Vec::new();
    };

    let mut locs = Vec::new();

    if include_declaration {
        if let Some(info) = index.get(&id) {
            if let Ok(note_content) = std::fs::read_to_string(&info.path) {
                if let Some(header) = parser::parse_header(&note_content) {
                    if let Ok(uri) = Url::from_file_path(&info.path) {
                        locs.push(Location {
                            uri,
                            range: Range {
                                start: Position {
                                    line: header.title_line_idx as u32,
                                    character: 0,
                                },
                                end: Position {
                                    line: header.title_line_idx as u32,
                                    character: 0,
                                },
                            },
                        });
                    }
                }
            }
        }
    }

    locs.extend(index.get_backlinks(&id).into_iter().map(|loc| Location {
        uri: Url::from_file_path(&loc.file).unwrap_or_else(|_| uri.clone()),
        range: Range {
            start: Position {
                line: loc.line,
                character: loc.start_char,
            },
            end: Position {
                line: loc.line,
                character: loc.end_char,
            },
        },
    }));

    locs.extend(
        metadata::all_id_positions(metadata_content)
            .into_iter()
            .filter(|id_pos| id_pos.target_id == id)
            .map(|id_pos| Location {
                uri: metadata_uri.clone(),
                range: Range {
                    start: Position {
                        line: id_pos.range.line as u32,
                        character: id_pos.range.start_col as u32,
                    },
                    end: Position {
                        line: id_pos.range.line as u32,
                        character: id_pos.range.end_col as u32,
                    },
                },
            }),
    );

    locs
}

fn id_at_position(content: &str, position: Position) -> Option<String> {
    if let Some(id_pos) =
        metadata::id_at_position(content, position.line as usize, position.character as usize)
    {
        return Some(id_pos.target_id);
    }
    let line = content.lines().nth(position.line as usize)?;
    find_id_at_col(line, position.character as usize)
}

fn find_id_at_col(line: &str, col: usize) -> Option<String> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        let (start, end_delim) = match bytes[i] {
            b'@' => (i + 1, None),
            b'<' => (i + 1, Some(b'>')),
            b'"' => (i + 1, Some(b'"')),
            _ => {
                i += 1;
                continue;
            }
        };
        let end = start + 10;
        if end <= len
            && bytes[start..end].iter().all(|b| b.is_ascii_digit())
            && end_delim
                .map(|d| end < len && bytes[end] == d)
                .unwrap_or(true)
            && col >= i
            && col <= end
        {
            return Some(line[start..end].to_string());
        }
        i += 1;
    }
    None
}
