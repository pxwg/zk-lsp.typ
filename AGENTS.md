# Repository Guidelines

## Project Structure & Module Organization
`zk-lsp` is a single-binary Rust project. Core code lives in `src/`, with CLI entrypoints in `src/main.rs` and `src/cli.rs`. The LSP runtime is centered in `src/server.rs`, while request-specific logic is split into `src/handlers/` (`definition.rs`, `diagnostics.rs`, `code_actions.rs`, etc.). Wiki operations such as initialization, migration, reconcile, export, and graph checks are implemented as focused modules like `src/init.rs`, `src/migrate.rs`, `src/reconcile.rs`, and `src/graph_check.rs`. Release automation lives in `.github/workflows/`, and the Homebrew formula is maintained in `Formula/zk-lsp.rb`.

## Build, Test, and Development Commands
Use Cargo for the full local workflow:

- `cargo test` runs the current test suite and is the same command used in CI.
- `cargo build --release` builds the production binary that CI and releases ship.
- `cargo run -- lsp` starts the language server over stdio.
- `cargo run -- init --wiki-root /tmp/wiki` scaffolds a test wiki.
- `cargo run -- check --wiki-root /tmp/wiki` validates dead links and orphans in a sample workspace.

## Coding Style & Naming Conventions
Follow standard Rust formatting: 4-space indentation, trailing commas where rustfmt expects them, and `snake_case` for modules/functions with `UpperCamelCase` for types. Keep CLI parsing in `src/cli.rs`, keep protocol wiring in `src/server.rs`, and move feature logic into dedicated modules instead of growing `main.rs` or `server.rs`. Run `cargo fmt` before submitting changes. Prefer small, composable functions and explicit names like `get_diagnostics` or `generate_link_typ`.

## Testing Guidelines
There is no top-level `tests/` directory yet; add focused unit tests next to the code they cover with `#[cfg(test)]` when possible. Use descriptive test names such as `reconcile_skips_archived_notes` or `check_reports_dead_links`. At minimum, run `cargo test` and `cargo build --release` before opening a PR, since both are enforced in GitHub Actions.

## Commit & Pull Request Guidelines
Recent history uses short Conventional Commit subjects: `feat: ...`, `fix: ...`, `chore: ...`. Keep commits imperative and scoped to one change. PRs should include a concise summary, note any CLI or LSP behavior changes, link the relevant issue if one exists, and include example commands or editor screenshots when user-visible diagnostics, hints, or actions change.
