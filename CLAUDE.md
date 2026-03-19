# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# CLAUDE.md — zk-lsp

Rust LSP binary for the `~/wiki` Typst-based Zettelkasten.

## Build & Test

```bash
cargo build          # dev build
cargo build --release
cargo test           # 211 tests across parser, formatting, migrate, reconcile, cycle, diagnostics, code_actions, completion, graph_check, context_export, hooks, config
cargo test <name>    # run a single test by name (substring match)
cargo fmt            # formatting should be run before every commit
```

Zero warnings are expected. Fix all warnings before committing.

## CLI Commands

```bash
zk-lsp [lsp]                        # start LSP on stdin/stdout (default)
zk-lsp generate [--wiki-root PATH]  # regenerate ~/wiki/link.typ
zk-lsp new [--wiki-root PATH]       # create note, print path
zk-lsp remove <ID> [--wiki-root PATH]  # delete note + remove from link.typ
zk-lsp format                       # read note from stdin, write formatted to stdout
zk-lsp migrate [--wiki-root PATH]   # migrate legacy comment-format notes to TOML schema v1
zk-lsp reconcile [--wiki-root PATH] [--dry-run]  # reconcile cross-file checkbox states
zk-lsp export <ID> [--depth N] [--inverse]  # BFS context export to Markdown (default depth: 2; --inverse follows backlinks, ancestors first)
zk-lsp check [--no-orphans] [--no-dead-links]  # graph integrity: dead links + orphans; exits 1 on dead links
```

`WIKI_ROOT` env overrides the `~/wiki` default. `--wiki-root` overrides `WIKI_ROOT`.

## Configuration

Configuration is merged from two TOML files (project overrides user):

| Path | Scope |
|------|-------|
| `$XDG_CONFIG_HOME/zk-lsp/config.toml` | User-level |
| `<wiki-root>/zk-lsp.toml` | Project-level |

Key config sections:

```toml
# Lua hooks (run on willSaveWaitUntil)
[[hooks]]
path = "~/.config/zk-lsp/hooks/my_hook.lua"

# Reconcile DSL rule files (Lisp-based)
[[reconcile_rules]]
path = "~/.config/zk-lsp/rules/checklist.lisp"

disable_default_reconcile_rules = false   # set true to replace built-in rules

# User-defined metadata fields (user.* namespace only)
[[metadata.field]]
path    = "user.priority"
kind    = "string"           # or "boolean", "array-string"
default = "normal"
```

Examples in `examples/hooks/` and `examples/rules/`.

## Wiki Note Structure

Notes use the TOML format (primary, created by `zk-lsp new`):

```
#import "../include.typ": *
#let zk-metadata = toml(bytes("""
schema-version = 1
title = "..."
tags = [...]
checklist-status = "none"   # or "todo", "wip", "done"
relation = "active"         # or "archived", "legacy"
relation-target = []        # required when relation != "active"
generated = false
"""))
#show: zettel

= Title <YYMMDDHHMM>
```

Legacy comment format (read-only; run `zk-lsp migrate` to convert):

```
/* Metadata:
Aliases: ...
Abstract: ...
Keyword: ...
Generated: true
*/
#import "../include.typ": *    <- import_idx  (0-based)
#show: zettel
                               <- import_idx + 2  (blank)
= Title <YYMMDDHHMM>           <- title_line_idx = import_idx + 3
#tag.xxx                       <- tag_line_idx   = import_idx + 4
#evolution_link(<ID>)          <- import_idx + 5  (optional)
```

Parser tries TOML path first; falls back to legacy. `parse_header()` no longer creates legacy-format notes.

## Key Design Rules

- **Parser is stateless** — `src/parser.rs` takes `&str`, returns owned structs. No I/O.
- **Index is async** — `NoteIndex` uses `DashMap`; all file I/O via `tokio::fs`.
- **Atomic writes** — `link.typ` and note files are always written via `tmp → rename`.
- **Tracing to stderr** — stdout is reserved for JSON-RPC. Use `tracing::{info, error, …}`.
- **ID format** — exactly 10 ASCII digits (`YYMMDDHHMM`). Regex: `@(\d{10})`.

## Reference Lua Sources

These files are authoritative for behaviour parity:

| File | Defines |
|------|---------|
| `~/.config/nvim/lua/zk_lsp.lua` | Diagnostics, code actions, references, LSP capabilities |
| `~/.config/nvim/lua/zk_scripts.lua` | Note CRUD, tag formatter, cross-file todo propagation |
| `~/.config/nvim/lua/zk_extmark.lua` | Inlay hint regex and title extraction logic |

## Source Map

```
src/
├── main.rs               CLI dispatch + LSP server startup
├── lib.rs                Re-exports for integration tests
├── init.rs               LSP initialization helper
├── cli.rs                clap CLI definitions
├── config.rs             WikiConfig resolution; ZkLspConfig (hooks, reconcile_rules, metadata fields)
├── parser.rs             Stateless note parsing (unit-tested)
├── dependency_graph.rs   build_dependency_graph: RefItem → positioned edge list
├── cycle.rs              detect_cycles (Tarjan SCC) + render_cycle_errors (CLI)
├── graph_check.rs        check_graph (dead links + orphans) + render_check_report (CLI)
├── context_export.rs     export_context: BFS Markdown for AI consumption
├── index.rs              NoteIndex (DashMap notes + backlinks)
├── link_gen.rs           link.typ generation and entry management
├── migrate.rs            migrate_wiki / migrate_note (legacy → TOML v1)
├── note_ops.rs           create_note / delete_note
├── server.rs             tower-lsp LanguageServer impl
├── watcher.rs            notify-debouncer-mini (300 ms) on note_dir
├── hooks/
│   ├── types.rs         HookNoteInput, HookCheckbox, HookTextEdit, HookResult structs
│   ├── lua.rs           Lua VM host (mlua); runs hook run(note) → HookResult
│   └── apply.rs         apply_hooks: load + call all configured hooks, apply edits + metadata patch
├── reconcile/            Reconcile DSL v1 — Lisp-based rule engine for workspace-wide reconciliation
│   ├── types.rs         Value, Status, NoteId, ReconcileDiagnostic
│   ├── ast.rs           Module, Expr AST nodes
│   ├── parser.rs        parse_module: s-expression parser for rule files
│   ├── typecheck.rs     type_check_module_with_metadata: static type checker
│   ├── default_module.rs load_module: built-in + user rule files; hot-reload on file change
│   ├── observe.rs       WorkspaceSnapshot: read notes into typed observation structs
│   ├── eval.rs          eval_all_typed: evaluate rules against snapshot → EvalResult
│   ├── materialize.rs   materialize: EvalResult → ReconcileResult (checkboxes + meta patches)
│   ├── writeback.rs     normalize_note_from_checked, is_note_done_with_deps
│   └── mod.rs           run_reconcile, collect_diagnostics (public API)
└── handlers/
    ├── references.rs    find_references (uses backlink index)
    ├── diagnostics.rs   dead link ERROR + archived/legacy/orphan/cycle/schema diagnostics
    ├── code_actions.rs  quick-fixes + metadata toggle actions (checklist-status, relation)
    ├── completion.rs    TOML metadata completions (enum values, note IDs, field names)
    ├── definition.rs    go-to-definition for @ID in relation-target fields
    ├── hover.rs         hover preview for @ID references
    ├── inlay_hints.rs   @ID → title after cursor
    └── formatting.rs    willSaveWaitUntil: apply hooks + tag normalization
```

## Lua Hooks

Hooks are Lua scripts called on `willSaveWaitUntil`. Each must expose:

```lua
function run(note)
  -- note.id, note.content, note.metadata, note.checkboxes, note.headings
  -- note.metadata_defaults (config-declared field defaults)
  -- note.metadata_fields (declared field configs)
  return {
    metadata = { ["checklist-status"] = "done" },  -- optional patch
    edits    = { { start_byte = 42, end_byte = 43, text = "x" } },  -- optional byte edits
  }
end
```

Both fields are optional. Edits must be non-overlapping; overlapping edits error. Applied in reverse byte order.

## Reconcile DSL

Rules are Lisp s-expressions. Three required functions:

```lisp
(module
  (define (materialized_fields n)       ; → list of field names to materialize
    (list "checklist-status"))
  (define (effective_checked c)         ; → bool: should checkbox be checked?
    (observe_checked c))
  (define (effective_meta n field)      ; → value to write for field
    (observe_meta n field)))
```

Built-in observables: `observe_checked`, `observe_meta`, `backlinks`, `children`, `parent`, `owner`.
Built-in combinators: `filter`, `reduce`, `map`, `length`, `contains`, `union`, `dedup`, `if`, `eq?`, `+`, `>`, `>=`.

See `examples/rules/` for working examples.

## Neovim Integration

```lua
vim.lsp.config("zk-lsp", {
  cmd = { "zk-lsp", "lsp" },
  filetypes = { "typst" },
  root_dir = vim.fn.expand("~/wiki"),
})
```

## Checklist Semantics

Two item types exist in checklists:

1. **`LocalItem`** — truth = `item.checked` (source fact; user-authored)
2. **`RefItem`** (`- [ ] @A @B …`) — truth = `∀ t ∈ targets: done_lookup(t.target_id)` (all referenced notes must be done; **never** the rendered checkbox)

**`RefTarget`** carries `target_id`, `byte_start`, `byte_end` (byte offsets of `@ID` within the full line). Used by `dependency_graph` for positioned error reporting and by LSP diagnostics via `byte_to_utf16`.

**Leaf items rule:** Only leaf items participate in note status aggregation. A leaf has no subsequent item with strictly greater indent. Non-leaf LocalItems are derived display views of their children and must not be used as source facts.

**Rendered `[x]` on a RefItem is NEVER a source of truth in the solver** — `dep_states["B"]` is authoritative.

**Note done formula:**
```
note.done = ∀ leaf_item ∈ items: eval_item_truth(leaf_item) == true
          (falls back to metadata.checklist_status when items list is empty)
```

**Responsibility split:**
- **Formatter** (`formatting.rs`): runs hooks, then normalizes current-file checkboxes using dep_states from reconciled metadata. `is_note_done_with_deps` is the canonical semantic evaluator.
- **Reconcile** (`reconcile/`): load rules → typecheck → observe workspace snapshot → eval all rules → materialize → batch write-back. Fails fast on cycles.
- **Cycles** — hard error. Tarjan SCC in `reconcile/mod.rs` and `cycle.rs`; CLI renders Typst-style errors with ANSI colour and CJK-aware `^` alignment; LSP emits per-file `ERROR` diagnostics.

**Key functions:**
- `parser::find_all_refs_filtered(content)` → `Vec<RefOccurrence>` (skips TOML block, `/* */` comments, fenced blocks)
- `dependency_graph::build_dependency_graph(notes)` → `DependencyGraph`
- `reconcile::run_reconcile(config, dry_run)` → `ReconcileStats`
- `reconcile::collect_diagnostics(config, overlay)` → `Vec<ReconcileDiagnostic>` (LSP path)
- `reconcile::writeback::is_note_done_with_deps(content, deps)` → bool
- `reconcile::writeback::normalize_note_from_checked(content, checked_by_line)` → String
- `diagnostics::get_schema_diagnostics(content, index)` → `Vec<Diagnostic>`
- `diagnostics::get_orphan_diagnostic(content, uri_path, index)` → `Option<Diagnostic>`
- `diagnostics::get_checklist_diagnostics(content)` → `Vec<Diagnostic>`
- `graph_check::check_graph(config)` → `CheckReport`
- `context_export::export_context(entry_id, depth, inverse, config)` → `String`
- `code_actions::get_metadata_actions(uri, content, range)` → `Vec<CodeActionOrCommand>`
- `completion::get_completions(content, position, index)` → `Vec<CompletionItem>`
- `hooks::apply::apply_hooks(content, note_id, config)` → `Result<String>`

## LSP Commands

| Command | Arguments | Returns |
|---------|-----------|---------|
| `zk.newNote` | — | — |
| `zk.removeNote` | `id: string` | — |
| `zk.generateLinkTyp` | — | — |
| `zk.exportContext` | `id: string, depth?: number, inverse?: bool` | `string` (Markdown) |

## Diagnostics Summary

| Source | Severity | Trigger |
|--------|----------|---------|
| dead `@ID` ref | ERROR | referenced note does not exist in index |
| cycle | ERROR | `@ID` participates in a task-dependency cycle |
| orphan note | HINT | note has no inbound `@ID` references |
| archived `@ID` | WARNING | referenced note has `relation = "archived"` |
| legacy `@ID` | INFORMATION | referenced note has `relation = "legacy"` |
| schema | ERROR/WARNING | invalid TOML field values or missing `relation-target` |
| non-leaf RefItem | WARNING | `@ID` checklist item has child items; dependency silently ignored |

## Install

```bash
cargo build --release
ln -sf $(pwd)/target/release/zk-lsp ~/.local/bin/zk-lsp
```
