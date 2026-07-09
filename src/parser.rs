/// Stateless parsing of Zettelkasten note headers and content.
use once_cell::sync::Lazy;
use regex::Regex;

pub(crate) static RE_ID_REF: Lazy<Regex> = Lazy::new(|| Regex::new(r"@(\d{10})").unwrap());
pub(crate) static RE_ANGLE_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"<(\d{10})>").unwrap());
pub(crate) static RE_TITLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^=\s+.*<(\d{10})>").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecklistStatus {
    None,
    Todo,
    Wip,
    Done,
}

impl ChecklistStatus {
    pub const ALL: [Self; 4] = [Self::None, Self::Todo, Self::Wip, Self::Done];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Todo => "todo",
            Self::Wip => "wip",
            Self::Done => "done",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "todo" => Some(Self::Todo),
            "wip" => Some(Self::Wip),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Relation {
    Active,
    Archived,
    Legacy,
}

#[derive(Debug, Clone)]
pub struct MetadataBinding {
    pub line_idx: usize,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct NoteHeader {
    pub id: String,
    pub title: String,
    #[allow(dead_code)]
    pub title_line_idx: usize, // 0-based
}

#[derive(Debug, Clone)]
pub struct RefOccurrence {
    pub id: String,
    pub line: u32,
    pub start_char: u32,
    pub end_char: u32,
}

/// Scan `content` for a canonical central metadata binding:
/// `#let zk-metadata = zk_metadata("ID")`.
pub fn find_metadata_binding(content: &str) -> Option<MetadataBinding> {
    static RE_BINDING: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^\s*#let\s+zk-metadata\s*=\s*zk_metadata\("(\d{10})"\)\s*$"#).unwrap()
    });

    let mut found = None;
    for (line_idx, line) in content.lines().enumerate() {
        let Some(caps) = RE_BINDING.captures(line) else {
            continue;
        };
        if found.is_some() {
            return None;
        }
        found = Some(MetadataBinding {
            line_idx,
            id: caps.get(1)?.as_str().to_string(),
        });
    }
    found
}

/// Parse the structural header of a central-metadata note.
pub fn parse_header(content: &str) -> Option<NoteHeader> {
    let lines: Vec<&str> = content.lines().collect();

    let title_line_idx = lines.iter().position(|l| RE_TITLE.is_match(l))?;

    let title_line = lines[title_line_idx];
    let id = RE_TITLE.captures(title_line)?.get(1)?.as_str().to_string();
    let title = RE_TITLE
        .captures(title_line)?
        .get(0)?
        .as_str()
        .trim_start_matches('=')
        .trim()
        .rsplit_once('<')
        .map(|(t, _)| t.trim().to_string())
        .unwrap_or_default();

    let binding = find_metadata_binding(content)?;
    if binding.line_idx >= title_line_idx || binding.id != id {
        return None;
    }

    Some(NoteHeader {
        id,
        title,
        title_line_idx,
    })
}

// ---------------------------------------------------------------------------
// Checklist item model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RefTarget {
    pub target_id: String,
    pub byte_start: u32, // byte offset of '@' within the full line
    pub byte_end: u32,   // byte offset past the last digit
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChecklistItemKind {
    Local,
    Ref { targets: Vec<RefTarget> },
}

#[derive(Debug, Clone)]
pub struct ChecklistItem {
    pub checked: bool,
    pub kind: ChecklistItemKind,
    #[allow(dead_code)]
    pub text: String,
    pub line_idx: usize,
    pub indent: usize,
}

/// Parse all checklist items from `content`, skipping fenced code blocks.
/// Items with `@(\d{10})` in their text become `Ref` items; all others are `Local`.
/// `RefTarget.byte_start`/`byte_end` are byte offsets of `@ID` within the full line.
pub fn parse_checklist_items(content: &str) -> Vec<ChecklistItem> {
    let mut items = Vec::new();
    let mut in_fence = false;

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if !(trimmed.starts_with("- [") && trimmed.len() >= 5) {
            continue;
        }
        let marker = trimmed.chars().nth(3).unwrap_or(' ');
        if marker != 'x' && marker != 'X' && marker != ' ' {
            continue;
        }
        let checked = marker == 'x' || marker == 'X';
        let indent = line.len() - trimmed.len();
        // prefix_len: bytes before the checklist body (indent + "- [x] ")
        let prefix_len = indent + 6;
        // text after `- [x] ` (or `- [ ] `)
        let body = trimmed.get(6..).unwrap_or("");
        let text = body.to_string();
        let targets: Vec<RefTarget> = RE_ID_REF
            .captures_iter(body)
            .map(|c| {
                let full = c.get(0).unwrap();
                let id = c.get(1).unwrap().as_str().to_string();
                RefTarget {
                    target_id: id,
                    byte_start: (prefix_len + full.start()) as u32,
                    byte_end: (prefix_len + full.end()) as u32,
                }
            })
            .collect();
        let kind = if targets.is_empty() {
            ChecklistItemKind::Local
        } else {
            ChecklistItemKind::Ref { targets }
        };
        items.push(ChecklistItem {
            checked,
            kind,
            text,
            line_idx,
            indent,
        });
    }
    items
}

/// Evaluate the semantic truth of a single checklist item.
/// `Local` items: truth = checkbox state.
/// `Ref` items: truth = `∀ t ∈ targets: done_lookup(t.target_id)` — never the rendered checkbox.
pub fn eval_item_truth(item: &ChecklistItem, done_lookup: &impl Fn(&str) -> bool) -> bool {
    match &item.kind {
        ChecklistItemKind::Local => item.checked,
        ChecklistItemKind::Ref { targets } => targets.iter().all(|t| done_lookup(&t.target_id)),
    }
}

fn is_leaf(items: &[ChecklistItem], idx: usize) -> bool {
    idx + 1 >= items.len() || items[idx + 1].indent <= items[idx].indent
}

/// Compute whether a note is done based on its checklist items and a dependency lookup.
///
/// Only **leaf items** participate: a leaf is an item with no subsequent item
/// with strictly greater indent before the next same-or-lesser-indent item.
/// Non-leaf LocalItems are derived display views and must not be counted as source facts.
/// If there are no items, returns `false` (caller should check metadata separately).
pub fn compute_note_done_from_items(
    items: &[ChecklistItem],
    done_lookup: &impl Fn(&str) -> bool,
) -> bool {
    let leaves: Vec<&ChecklistItem> = items
        .iter()
        .enumerate()
        .filter(|(i, _)| is_leaf(items, *i))
        .map(|(_, item)| item)
        .collect();
    if leaves.is_empty() {
        return false;
    }
    leaves.iter().all(|item| eval_item_truth(item, done_lookup))
}

/// Convert a byte offset within `s` to a UTF-16 code-unit offset.
/// LSP `character` positions are UTF-16 code units, not bytes or scalar values.
pub fn byte_to_utf16(s: &str, byte_offset: usize) -> u32 {
    s[..byte_offset].chars().map(|c| c.len_utf16() as u32).sum()
}

/// Find all @ID occurrences in content (10-digit IDs).
/// `start_char` / `end_char` are **byte** offsets within the line (not UTF-16).
/// Convert with `byte_to_utf16` before using as LSP character positions.
pub fn find_all_refs(content: &str) -> Vec<RefOccurrence> {
    let mut refs = Vec::new();
    for (line_num, line) in content.lines().enumerate() {
        for cap in RE_ID_REF.captures_iter(line) {
            let m = cap.get(0).unwrap();
            let id_m = cap.get(1).unwrap();
            refs.push(RefOccurrence {
                id: id_m.as_str().to_string(),
                line: line_num as u32,
                start_char: m.start() as u32,
                end_char: m.end() as u32,
            });
        }
    }
    refs
}

/// Find all @ID occurrences in content, skipping:
/// - Block comments (`/* ... */`, including multi-line)
/// - Fenced code blocks (``` ... ```)
pub fn find_all_refs_filtered(content: &str) -> Vec<RefOccurrence> {
    let mut refs = Vec::new();

    let mut in_block_comment = false;
    let mut in_fence = false;

    for (line_num, line) in content.lines().enumerate() {
        // Handle block comment continuation
        if in_block_comment {
            if let Some(end_offset) = line.find("*/") {
                in_block_comment = false;
                // Process visible content after end of block comment
                // by falling through with adjusted pos below
                let after_offset = end_offset + 2;
                let mut visible_segments: Vec<(usize, usize)> = Vec::new();
                let mut pos = after_offset;
                loop {
                    let remaining = &line[pos..];
                    if let Some(bc_start) = remaining.find("/*") {
                        visible_segments.push((pos, pos + bc_start));
                        let bc_abs = pos + bc_start;
                        if let Some(end_off) = line[bc_abs + 2..].find("*/") {
                            pos = bc_abs + 2 + end_off + 2;
                        } else {
                            in_block_comment = true;
                            break;
                        }
                    } else {
                        visible_segments.push((pos, line.len()));
                        break;
                    }
                }
                for (seg_start, seg_end) in visible_segments {
                    let segment = &line[seg_start..seg_end];
                    for cap in RE_ID_REF.captures_iter(segment) {
                        let m = cap.get(0).unwrap();
                        let id_m = cap.get(1).unwrap();
                        refs.push(RefOccurrence {
                            id: id_m.as_str().to_string(),
                            line: line_num as u32,
                            start_char: (seg_start + m.start()) as u32,
                            end_char: (seg_start + m.end()) as u32,
                        });
                    }
                }
            }
            // Whether we found */ or not, move to next line
            continue;
        }

        // Fence toggle (only when not in block comment)
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        // Normal line: scan for block comment boundaries and collect visible segments
        let mut visible_segments: Vec<(usize, usize)> = Vec::new();
        let mut pos = 0;
        loop {
            let remaining = &line[pos..];
            if let Some(bc_start) = remaining.find("/*") {
                visible_segments.push((pos, pos + bc_start));
                let bc_abs = pos + bc_start;
                if let Some(end_off) = line[bc_abs + 2..].find("*/") {
                    pos = bc_abs + 2 + end_off + 2;
                } else {
                    in_block_comment = true;
                    break;
                }
            } else {
                visible_segments.push((pos, line.len()));
                break;
            }
        }

        for (seg_start, seg_end) in visible_segments {
            let segment = &line[seg_start..seg_end];
            for cap in RE_ID_REF.captures_iter(segment) {
                let m = cap.get(0).unwrap();
                let id_m = cap.get(1).unwrap();
                refs.push(RefOccurrence {
                    id: id_m.as_str().to_string(),
                    line: line_num as u32,
                    start_char: (seg_start + m.start()) as u32,
                    end_char: (seg_start + m.end()) as u32,
                });
            }
        }
    }

    refs
}

/// Find all link target IDs in content, combining both `@ID` and `<ID>` forms,
/// while skipping block comments and fenced code blocks.
///
/// The note's own title ID (`= Title <ID>`) is excluded to avoid treating the
/// note header as a self-link.
pub fn find_all_link_ids_filtered(content: &str) -> Vec<String> {
    let self_id = parse_header(content).map(|h| h.id);
    let mut ids: Vec<String> = find_all_refs_filtered(content)
        .into_iter()
        .map(|r| r.id)
        .collect();

    let mut in_block_comment = false;
    let mut in_fence = false;

    for line in content.lines() {
        let mut visible_segments = Vec::new();
        let mut pos = 0usize;
        loop {
            if in_block_comment {
                let Some(end_offset) = line[pos..].find("*/") else {
                    break;
                };
                in_block_comment = false;
                pos += end_offset + 2;
                continue;
            }

            let trimmed = line[pos..].trim_start();
            if pos == 0 && trimmed.starts_with("```") {
                in_fence = !in_fence;
                break;
            }
            if in_fence {
                break;
            }

            let remaining = &line[pos..];
            let bc_start = remaining.find("/*");
            let fence_start = remaining.find("```");
            let next_cut = match (bc_start, fence_start) {
                (Some(bc), Some(fence)) => Some(bc.min(fence)),
                (Some(bc), None) => Some(bc),
                (None, Some(fence)) => Some(fence),
                (None, None) => None,
            };

            match next_cut {
                Some(offset) => {
                    visible_segments.push((pos, pos + offset));
                    let abs = pos + offset;
                    if remaining[offset..].starts_with("/*") {
                        in_block_comment = true;
                        pos = abs + 2;
                    } else {
                        in_fence = true;
                        break;
                    }
                }
                None => {
                    visible_segments.push((pos, line.len()));
                    break;
                }
            }
        }

        for (seg_start, seg_end) in visible_segments {
            let segment = &line[seg_start..seg_end];
            for cap in RE_ANGLE_ID.captures_iter(segment) {
                let id = cap.get(1).unwrap().as_str();
                if self_id.as_deref() == Some(id) {
                    continue;
                }
                ids.push(id.to_string());
            }
        }
    }

    ids
}

/// A heading parsed from note content (outside fenced code).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Heading {
    pub level: u32,
    pub text: String,
    pub line_idx: usize,
}

/// Parse all headings from `content`, skipping fenced code blocks.
/// For the title heading (matching `= Title <YYMMDDHHMM>`), the ` <ID>` suffix is stripped.
#[allow(dead_code)]
pub fn parse_headings(content: &str) -> Vec<Heading> {
    static RE_HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(=+)\s+(.+)").unwrap());
    static RE_ID_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+<\d{10}>$").unwrap());

    let mut headings = Vec::new();
    let mut in_fence = false;

    for (line_idx, line) in content.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(cap) = RE_HEADING.captures(line) {
            let level = cap[1].len() as u32;
            let raw_text = cap[2].trim().to_string();
            let text = RE_ID_SUFFIX.replace(&raw_text, "").to_string();
            headings.push(Heading {
                level,
                text,
                line_idx,
            });
        }
    }
    headings
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRAL_NOTE: &str = concat!(
        "#import \"../include.typ\": *\n",
        "#let zk-metadata = zk_metadata(\"2603110000\")\n",
        "#show: zettel.with(metadata: zk-metadata)\n",
        "\n",
        "= Central Note <2603110000>\n",
        "\n",
        "See @2603110002\n",
    );

    #[test]
    fn parse_header_requires_central_binding() {
        let h = parse_header(CENTRAL_NOTE).unwrap();
        assert_eq!(h.id, "2603110000");
        assert_eq!(h.title, "Central Note");
        assert_eq!(h.title_line_idx, 4);
    }

    #[test]
    fn parse_header_rejects_missing_or_mismatched_binding() {
        let missing = "#import \"../include.typ\": *\n\n= Note <2603110000>\n";
        let mismatched = concat!(
            "#let zk-metadata = zk_metadata(\"2603110001\")\n",
            "= Note <2603110000>\n",
        );
        assert!(parse_header(missing).is_none());
        assert!(parse_header(mismatched).is_none());
    }

    #[test]
    fn checklist_status_roundtrip() {
        for status in ChecklistStatus::ALL {
            assert_eq!(ChecklistStatus::from_str(status.as_str()), Some(status));
        }
    }

    #[test]
    fn find_all_refs_extracts_ids() {
        let refs = find_all_refs("see @2602082037 and @2602082106");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].id, "2602082037");
        assert_eq!(refs[1].id, "2602082106");
    }

    #[test]
    fn byte_to_utf16_cjk() {
        let line = "Hello, world 你好 @2602171536";
        let refs = find_all_refs(line);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].start_char, 20);
        assert_eq!(byte_to_utf16(line, refs[0].start_char as usize), 16);
        assert_eq!(byte_to_utf16(line, refs[0].end_char as usize), 27);
    }

    #[test]
    fn ref_target_spans() {
        let line = "  - [ ] @1111111111 and @2222222222";
        let content = format!("{line}\n");
        let items = parse_checklist_items(&content);
        assert_eq!(items.len(), 1);
        if let ChecklistItemKind::Ref { targets } = &items[0].kind {
            assert_eq!(targets.len(), 2);
            assert_eq!(targets[0].target_id, "1111111111");
            assert_eq!(targets[0].byte_start, 8);
            assert_eq!(targets[0].byte_end, 19);
            assert_eq!(targets[1].target_id, "2222222222");
            assert_eq!(targets[1].byte_start, 24);
            assert_eq!(targets[1].byte_end, 35);
        } else {
            panic!("expected Ref kind");
        }
    }

    #[test]
    fn find_refs_skips_block_comment_and_fenced_block() {
        let content = concat!(
            "see @2602082037\n",
            "/* skip @9999999999 */\n",
            "```\n",
            "@8888888888\n",
            "```\n",
            "and @2602082106\n",
        );
        let refs = find_all_refs_filtered(content);
        let ids: Vec<&str> = refs.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"2602082037"));
        assert!(ids.contains(&"2602082106"));
        assert!(!ids.contains(&"9999999999"));
        assert!(!ids.contains(&"8888888888"));
    }

    #[test]
    fn link_ids_skip_self_title_angle_id() {
        let ids = find_all_link_ids_filtered(CENTRAL_NOTE);
        assert_eq!(ids, vec!["2603110002"]);
    }
}
