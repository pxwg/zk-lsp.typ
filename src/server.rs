use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{error, info};

use crate::config::WikiConfig;
use crate::handlers::{
    code_actions, completion, definition, diagnostics, hover, inlay_hints, references,
};
use crate::index::NoteIndex;
use crate::{link_gen, note_ops, reconcile, watcher};

pub struct ZkLspServer {
    client: Client,
    index: Arc<NoteIndex>,
    config: Arc<RwLock<WikiConfig>>,
    cli_root: Option<std::path::PathBuf>,
    open_documents: Arc<RwLock<HashMap<Url, String>>>,
}

impl ZkLspServer {
    pub fn new(
        client: Client,
        config: Arc<RwLock<WikiConfig>>,
        cli_root: Option<std::path::PathBuf>,
    ) -> Self {
        let index = Arc::new(NoteIndex::new(Arc::clone(&config)));
        ZkLspServer {
            client,
            index,
            config,
            cli_root,
            open_documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn current_config(&self) -> WikiConfig {
        self.config.read().await.clone()
    }

    async fn publish_diagnostics(&self, uri: Url, content: &str) {
        let file_path = uri.to_file_path().unwrap_or_default();
        let mut diags = diagnostics::get_diagnostics(content, &self.index, uri.path());
        diags.extend(diagnostics::get_schema_diagnostics(content, &self.index));
        let config = self.current_config().await;
        if let Ok(reconcile_diags) =
            reconcile::collect_diagnostics(&config, Some((&file_path, content))).await
        {
            diags.extend(diagnostics::get_reconcile_diagnostics(
                content,
                &file_path,
                &reconcile_diags,
            ));
        }
        if let Some(d) = diagnostics::get_orphan_diagnostic(content, uri.path(), &self.index) {
            diags.push(d);
        }
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    async fn read_document(&self, uri: &Url) -> Option<String> {
        if let Some(content) = self.open_documents.read().await.get(uri).cloned() {
            return Some(content);
        }

        uri.to_file_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for ZkLspServer {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        let init_root = WikiConfig::lsp_root(&params);
        let resolved = WikiConfig::resolve(self.cli_root.clone(), init_root);
        let resolved_root = resolved.root.clone();
        *self.config.write().await = resolved;
        info!("initialize: resolved root to {}", resolved_root.display());

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                        will_save: None,
                        will_save_wait_until: None,
                    },
                )),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["\"".into(), "=".into(), "[".into()]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "zk.newNote".into(),
                        "zk.removeNote".into(),
                        "zk.generateLinkTyp".into(),
                        "zk.exportContext".into(),
                    ],
                    work_done_progress_options: Default::default(),
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "zk-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        info!("server initialized, building index…");
        let index = Arc::clone(&self.index);
        let config = Arc::clone(&self.config);
        let client = self.client.clone();
        let open_documents = Arc::clone(&self.open_documents);

        tokio::spawn(async move {
            match index.rebuild_full().await {
                Ok(n) => {
                    info!("index built: {n} notes");
                    for (uri, content) in open_documents.read().await.iter() {
                        let file_path = uri.to_file_path().unwrap_or_default();
                        let mut diags = diagnostics::get_diagnostics(content, &index, uri.path());
                        diags.extend(diagnostics::get_schema_diagnostics(content, &index));
                        let current_config = config.read().await.clone();
                        if let Ok(reconcile_diags) = reconcile::collect_diagnostics(
                            &current_config,
                            Some((&file_path, content)),
                        )
                        .await
                        {
                            diags.extend(diagnostics::get_reconcile_diagnostics(
                                content,
                                &file_path,
                                &reconcile_diags,
                            ));
                        }
                        if let Some(d) =
                            diagnostics::get_orphan_diagnostic(content, uri.path(), &index)
                        {
                            diags.push(d);
                        }
                        client.publish_diagnostics(uri.clone(), diags, None).await;
                    }
                    // Tell the client to re-request inlay hints now that the index is ready.
                    // Ask the client to re-fetch all inlay hints now that the
                    // index is populated.
                    let _ = client.inlay_hint_refresh().await;
                }
                Err(e) => error!("index build failed: {e}"),
            }
            // Start filesystem watcher
            if let Err(e) = watcher::start_watcher(config, index) {
                error!("watcher start failed: {e}");
            }
        });
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Text document lifecycle
    // -----------------------------------------------------------------------

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let content = params.text_document.text;
        self.open_documents
            .write()
            .await
            .insert(uri.clone(), content.clone());
        // Update index for this file
        if let Ok(path) = uri.to_file_path() {
            let _ = self.index.update_file(&path).await;
        }
        self.publish_diagnostics(uri, &content).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let initial_content = self.read_document(&uri).await.unwrap_or_default();
        let updated_content = {
            let mut documents = self.open_documents.write().await;
            let content = documents.entry(uri.clone()).or_insert(initial_content);
            apply_content_changes(content, &params.content_changes);
            content.clone()
        };

        self.publish_diagnostics(uri, &updated_content).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let content = match params.text {
            Some(t) => t,
            None => match uri
                .to_file_path()
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
            {
                Some(t) => t,
                None => return,
            },
        };

        self.open_documents
            .write()
            .await
            .insert(uri.clone(), content.clone());

        // Update index
        if let Ok(path) = uri.to_file_path() {
            let _ = self.index.update_file(&path).await;
        }

        // Publish diagnostics for the saved file
        self.publish_diagnostics(uri.clone(), &content).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.open_documents.write().await.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for change in params.changes {
            let uri = change.uri.clone();
            if let Ok(path) = uri.to_file_path() {
                match change.typ {
                    FileChangeType::CREATED | FileChangeType::CHANGED => {
                        let _ = self.index.update_file(&path).await;
                        if let Ok(content) = tokio::fs::read_to_string(&path).await {
                            self.publish_diagnostics(uri, &content).await;
                        }
                    }
                    FileChangeType::DELETED => {
                        self.index.remove_by_path(&path);
                    }
                    _ => {}
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Definition
    // -----------------------------------------------------------------------

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let content = self.read_document(uri).await.unwrap_or_default();

        Ok(definition::get_definition(&content, position, &self.index)
            .map(GotoDefinitionResponse::Scalar))
    }

    // -----------------------------------------------------------------------
    // References
    // -----------------------------------------------------------------------

    async fn references(&self, params: ReferenceParams) -> LspResult<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let row = params.text_document_position.position.line as usize;
        let content = match self.read_document(uri).await {
            Some(c) => c,
            None => return Ok(None),
        };
        let line = content.lines().nth(row).unwrap_or("");
        let locs = references::find_references(&self.index, uri, line);
        Ok(Some(locs))
    }

    // -----------------------------------------------------------------------
    // Code actions
    // -----------------------------------------------------------------------

    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let content = self.read_document(uri).await.unwrap_or_default();
        let mut actions = code_actions::get_code_actions(uri, &params.context.diagnostics);
        actions.extend(code_actions::get_metadata_actions(
            uri,
            &content,
            params.range,
        ));
        Ok(Some(actions))
    }

    // -----------------------------------------------------------------------
    // Completion
    // -----------------------------------------------------------------------

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let content = self.read_document(uri).await.unwrap_or_default();
        let items = completion::get_completions(&content, position, &self.index);
        Ok(if items.is_empty() {
            None
        } else {
            Some(CompletionResponse::Array(items))
        })
    }

    // -----------------------------------------------------------------------
    // Hover
    // -----------------------------------------------------------------------

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let content = self.read_document(uri).await.unwrap_or_default();
        Ok(hover::get_hover(&content, position, &self.index))
    }

    // -----------------------------------------------------------------------
    // Inlay hints
    // -----------------------------------------------------------------------

    async fn inlay_hint(&self, params: InlayHintParams) -> LspResult<Option<Vec<InlayHint>>> {
        let uri = &params.text_document.uri;
        let content = match self.read_document(uri).await {
            Some(c) => c,
            None => return Ok(None),
        };
        let hints = inlay_hints::get_inlay_hints(&content, params.range, &self.index);
        Ok(Some(hints))
    }

    // -----------------------------------------------------------------------
    // Workspace symbols
    // -----------------------------------------------------------------------

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> LspResult<Option<Vec<SymbolInformation>>> {
        #[allow(deprecated)]
        let symbols = self
            .index
            .search(&params.query)
            .into_iter()
            .map(|info| {
                let uri = Url::from_file_path(&info.path)
                    .unwrap_or_else(|_| Url::parse("file:///unknown").unwrap());
                SymbolInformation {
                    name: format!("[{}] {}", info.id, info.title),
                    kind: SymbolKind::FILE,
                    location: Location {
                        uri,
                        range: Range::default(),
                    },
                    tags: None,
                    deprecated: None,
                    container_name: None,
                }
            })
            .collect();
        Ok(Some(symbols))
    }

    // -----------------------------------------------------------------------
    // Execute command
    // -----------------------------------------------------------------------

    async fn execute_command(&self, params: ExecuteCommandParams) -> LspResult<Option<Value>> {
        match params.command.as_str() {
            "zk.generateLinkTyp" => {
                let config = self.current_config().await;
                match link_gen::generate_link_typ(&config).await {
                    Ok(()) => info!("link.typ regenerated"),
                    Err(e) => error!("generate_link_typ: {e}"),
                }
            }
            "zk.newNote" => {
                let config = self.current_config().await;
                match note_ops::create_note(&config).await {
                    Ok(path) => {
                        info!("created note: {}", path.display());
                        let uri = Url::from_file_path(&path).ok();
                        if let Some(uri) = uri {
                            self.client
                                .show_message(MessageType::INFO, format!("Created: {uri}"))
                                .await;
                        }
                    }
                    Err(e) => error!("create_note: {e}"),
                }
            }
            "zk.removeNote" => {
                if let Some(id) = params.arguments.first().and_then(|v| v.as_str()) {
                    let config = self.current_config().await;
                    match note_ops::delete_note(id, &config).await {
                        Ok(()) => info!("deleted note {id}"),
                        Err(e) => error!("delete_note: {e}"),
                    }
                }
            }
            "zk.exportContext" => {
                let id = params
                    .arguments
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let depth = params
                    .arguments
                    .get(1)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2) as usize;
                let inverse = params
                    .arguments
                    .get(2)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let simple = params
                    .arguments
                    .get(3)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let config = self.current_config().await;
                match crate::context_export::export_context(&id, depth, inverse, simple, &config)
                    .await
                {
                    Ok(text) => return Ok(Some(Value::String(text))),
                    Err(e) => error!("exportContext: {e}"),
                }
            }
            cmd => info!("unhandled command: {cmd}"),
        }
        Ok(None)
    }
}

fn apply_content_changes(content: &mut String, changes: &[TextDocumentContentChangeEvent]) {
    for change in changes {
        match change.range {
            Some(range) => {
                let start = position_to_byte_offset(content, range.start);
                let end = position_to_byte_offset(content, range.end);
                content.replace_range(start..end, &change.text);
            }
            None => {
                *content = change.text.clone();
            }
        }
    }
}

fn position_to_byte_offset(content: &str, position: Position) -> usize {
    let line_start = if position.line == 0 {
        0
    } else {
        match content
            .match_indices('\n')
            .nth(position.line.saturating_sub(1) as usize)
            .map(|(idx, _)| idx + 1)
        {
            Some(offset) => offset,
            None => content.len(),
        }
    };

    let line_end = content[line_start..]
        .find('\n')
        .map(|idx| line_start + idx)
        .unwrap_or(content.len());
    let line = &content[line_start..line_end];

    let mut utf16_units = 0u32;
    for (byte_idx, ch) in line.char_indices() {
        if utf16_units >= position.character {
            return line_start + byte_idx;
        }
        utf16_units += ch.len_utf16() as u32;
    }

    line_end
}

#[cfg(test)]
mod tests {
    use super::{apply_content_changes, position_to_byte_offset};
    use crate::parser;
    use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent};

    #[test]
    fn apply_content_changes_updates_incremental_edits() {
        let mut content = "第一行\nhello @1234567890\n".to_string();
        apply_content_changes(
            &mut content,
            &[TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position {
                        line: 1,
                        character: 6,
                    },
                    end: Position {
                        line: 1,
                        character: 17,
                    },
                }),
                range_length: None,
                text: "@0000000001".into(),
            }],
        );

        assert_eq!(content, "第一行\nhello @0000000001\n");
    }

    #[test]
    fn position_to_byte_offset_handles_utf16_columns() {
        let content = "第一行\nab界c\n";
        let offset = position_to_byte_offset(
            content,
            Position {
                line: 1,
                character: parser::byte_to_utf16("ab界c", "ab界".len()),
            },
        );

        assert_eq!(&content[offset..], "c\n");
    }
}
