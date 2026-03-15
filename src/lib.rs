//! # zk-lsp
//!
//! A Language Server Protocol (LSP) server for a [Typst]-based
//! [Zettelkasten] wiki, with a configurable format pipeline and a Lisp-family
//! DSL for cross-file state propagation.
//!
//! [Typst]: https://typst.app
//! [Zettelkasten]: https://en.wikipedia.org/wiki/Zettelkasten
//!
//! ---
//!
//! ## What is zk-lsp?
//!
//! `zk-lsp` keeps your Typst-based note graph consistent as it grows:
//!
//! - **Inlay hints** — `@2602082037` is rendered inline as `@ Note Title`
//! - **Diagnostics** — dead links, cycles, archived references, schema errors
//! - **Code actions** — toggle `checklist-status`, switch `relation`
//! - **Completions** — TOML enum values, note IDs, field names
//! - **References** — jump to every note that links to the current one
//! - **Format pipeline** — save-time Lua hooks normalise checkboxes and
//!   compute derived metadata
//! - **Reconcile DSL** — a Lisp-family rule language that propagates
//!   done-states across the wiki graph in a single topological pass
//! - **CLI tools** — `new`, `remove`, `generate`, `export`, `check`,
//!   `reconcile`, `note-info`, and more
//!
//! ---
//!
//! ## Extension Points
//!
//! Two orthogonal extension points let you customise behaviour without
//! touching Rust:
//!
//! | Extension | Scope | Language |
//! |-----------|-------|----------|
//! | [Lua Hooks](lua_hooks/index.html) | per-note, on every save | Lua 5.4 |
//! | [Reconcile DSL](reconcile_dsl/index.html) | cross-file, on demand | Lisp subset |
//!
//! ---
//!
//! ## Key Type Modules
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`config`] | [`ZkLspConfig`](config::ZkLspConfig), [`WikiConfig`](config::WikiConfig) |
//! | [`hooks::types`] | [`HookNoteInput`](hooks::types::HookNoteInput), [`HookResult`](hooks::types::HookResult) |
//! | [`reconcile::types`] | [`Value`](reconcile::types::Value), [`Status`](reconcile::types::Status) |

// --- Public API modules ---

pub mod config;
pub mod hooks;
pub mod reconcile;

// --- Internal modules required by reconcile and handlers (hidden from docs) ---

#[doc(hidden)]
pub mod cli;
#[doc(hidden)]
pub mod context_export;
#[doc(hidden)]
pub mod cycle;
#[doc(hidden)]
pub mod dependency_graph;
#[doc(hidden)]
pub mod graph_check;
#[doc(hidden)]
pub mod handlers;
#[doc(hidden)]
pub mod index;
#[doc(hidden)]
pub mod init;
#[doc(hidden)]
pub mod link_gen;
#[doc(hidden)]
pub mod migrate;
#[doc(hidden)]
pub mod note_ops;
#[doc(hidden)]
pub mod parser;
#[doc(hidden)]
pub mod server;
#[doc(hidden)]
pub mod watcher;

// --- Guide documents embedded via include_str! so they update with the source ---

/// Lua Hooks format pipeline — per-note transformations run on every save.
///
#[doc = include_str!("../docs/lua-hooks.md")]
pub mod lua_hooks {}

/// Reconcile DSL — cross-file state propagation driven by a Lisp rule language.
///
#[doc = include_str!("../docs/reconcile-dsl.md")]
pub mod reconcile_dsl {}
