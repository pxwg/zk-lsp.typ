use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::NoteIndex;
use crate::{metadata, parser};

pub fn get_metadata_hover(
    content: &str,
    position: Position,
    index: &Arc<NoteIndex>,
) -> Option<Hover> {
    get_metadata_hover_with_loader(content, position, index, |path| {
        std::fs::read_to_string(path).ok()
    })
}

fn get_metadata_hover_with_loader<F>(
    content: &str,
    position: Position,
    index: &Arc<NoteIndex>,
    load_note: F,
) -> Option<Hover>
where
    F: Fn(&std::path::Path) -> Option<String>,
{
    let id_pos =
        metadata::id_at_position(content, position.line as usize, position.character as usize)?;
    let info = index.notes.get(&id_pos.target_id)?;
    let note_content = load_note(&info.path)?;
    let preview_content = extract_preview_body(&note_content);

    let markdown = format!(
        "**{}** `{}`\n\n```typst\n{}\n```",
        info.title,
        info.id,
        preview_content.trim_end()
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: None,
    })
}

fn extract_preview_body(content: &str) -> String {
    let Some(header) = parser::parse_header(content) else {
        return content.to_string();
    };
    let lines: Vec<&str> = content.lines().collect();
    lines[header.title_line_idx..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WikiConfig;
    use crate::index::NoteInfo;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_index(id: &str, title: &str, path: PathBuf) -> Arc<NoteIndex> {
        let idx = NoteIndex::new(Arc::new(tokio::sync::RwLock::new(WikiConfig::from_root(
            PathBuf::from("/tmp"),
        ))));
        idx.notes.insert(
            id.to_string(),
            NoteInfo {
                id: id.to_string(),
                title: title.to_string(),
                archived: false,
                legacy: false,
                alt_id: None,
                evo_id: None,
                relation_target: vec![],
                aliases: vec![],
                keywords: vec![],
                abstract_text: None,
                checklist_status: None,
                path,
            },
        );
        Arc::new(idx)
    }

    const METADATA_CONTENT: &str = concat!(
        "format-version = 1\n\n",
        "[notes.\"2603110000\"]\n",
        "schema-version = 1\n",
        "relation-target = [\"2603110001\"]\n",
    );

    const TARGET_NOTE_CONTENT: &str = concat!(
        "#import \"../include.typ\": *\n",
        "#let zk-metadata = zk_metadata(\"2603110001\")\n",
        "#show: zettel.with(metadata: zk-metadata)\n",
        "\n",
        "= Target <2603110001>\n",
        "正文第一行\n",
    );

    #[test]
    fn metadata_hover_on_relation_target_returns_note_preview() {
        let path = PathBuf::from("/virtual/2603110001.typ");
        let index = make_index("2603110001", "Target Note", path);
        let hover = get_metadata_hover_with_loader(
            METADATA_CONTENT,
            Position {
                line: 4,
                character: 22,
            },
            &index,
            |path| {
                if path == PathBuf::from("/virtual/2603110001.typ").as_path() {
                    Some(TARGET_NOTE_CONTENT.to_string())
                } else {
                    None
                }
            },
        )
        .expect("expected hover");

        let HoverContents::Markup(mc) = hover.contents else {
            panic!()
        };
        assert!(mc.value.contains("2603110001"));
        assert!(mc.value.contains("Target Note"));
        assert!(mc.value.contains("= Target <2603110001>"));
        assert!(!mc.value.contains("#let zk-metadata"));
    }
}
