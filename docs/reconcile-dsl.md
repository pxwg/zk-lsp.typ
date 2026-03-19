# Reconcile DSL — Cross-File State Propagation

`zk-lsp reconcile` computes **global semantic states** across the whole wiki
by solving a system of equations over the note graph.  The equations are
written in a small Lisp-family DSL that you can extend or replace without
recompiling the binary.

---

## Design Philosophy

The DSL is grounded in the observe → effective → materialize three-layer
model:

```text
┌──────────────┐     ┌──────────────────┐     ┌──────────────────┐
│   observe    │────▶│    effective     │────▶│   materialize    │
│              │     │                  │     │                  │
│ raw values   │     │ values after     │     │ fields written   │
│ read from    │     │ user-defined     │     │ back to disk     │
│ disk         │     │ DSL rules        │     │ (declared by     │
│              │     │                  │     │ materialized_    │
│              │     │                  │     │ fields)          │
└──────────────┘     └──────────────────┘     └──────────────────┘
```

- **`observe_meta(n, field)`** — raw value currently on disk for note `n`
- **`effective_meta(n, field)`** — value after applying your DSL rules
- **`materialized_fields(n)`** — list of field paths that are written back;
  everything else is read-only during the current run

The key insight is that `checklist-status` is just one possible field in this
framework.  Custom `user.*` fields participate in the same evaluation
pipeline once they are declared in `materialized_fields`.

When the dependency graph is a DAG, the evaluator resolves it in a single
topological pass.  Cycles in task dependencies are a hard error reported
with full source locations.

---

## Module Structure

Every rule file must contain a single `(module ...)` form:

```lisp
(module
  ;; Optional policy block
  (policy
    (cycle error)          ;; "error" (default) — abort on cycles
    (unknown-status none)) ;; default status when observation is missing

  ;; Required: which fields to write back
  (define (materialized_fields n)
    (list "checklist-status"))

  ;; Required: effective checkbox state for checkbox c
  (define (effective_checked c)
    ...)

  ;; Required: effective metadata value for note n, field path
  (define (effective_meta n field)
    ...))
```

Helper rules are regular `(define ...)` forms and may call each other and
the built-in observations.

---

## Type System

The DSL is statically type-checked before evaluation.  The types that appear
in rule signatures are:

| Type | Description | Rust mirror |
|------|-------------|-------------|
| `Bool` | `true` / `false` | [`Value::Bool`](../zk_lsp/reconcile/types/enum.Value.html) |
| `Status` | `none` / `todo` / `wip` / `done` | [`Value::Status`](../zk_lsp/reconcile/types/enum.Value.html) |
| `Int` | 64-bit signed integer | [`Value::Int`](../zk_lsp/reconcile/types/enum.Value.html) |
| `Nil` | absence value; returned by `parent` for root items | [`Value::Nil`](../zk_lsp/reconcile/types/enum.Value.html) |
| `String` | arbitrary string; non-status metadata fields | [`Value::String`](../zk_lsp/reconcile/types/enum.Value.html) |
| `List(T)` | homogeneous list | [`Value::List`](../zk_lsp/reconcile/types/enum.Value.html) |
| `NoteRef` | runtime handle to a note | [`Value::NoteRef`](../zk_lsp/reconcile/types/enum.Value.html) |
| `CheckboxRef` | runtime handle to a checklist item | [`Value::CheckboxRef`](../zk_lsp/reconcile/types/enum.Value.html) |

---

## Built-in Standard Library

### Observations (read workspace state)

| Builtin | Signature | Description |
|---------|-----------|-------------|
| `observe_checked(c)` | `CheckboxRef → Status` | Raw checkbox state (`done`/`todo`) |
| `observe_meta(n, field)` | `NoteRef × String → T` | Read a metadata field |
| `targets(c)` | `CheckboxRef → List(NoteRef)` | Ref-item `@ID` targets |
| `backlinks(n)` | `NoteRef → List(NoteRef)` | Notes referencing `n` via `@ID` items |
| `parent(c)` | `CheckboxRef → CheckboxRef\|Nil` | Parent in the indent tree |
| `owner_note(c)` | `CheckboxRef → NoteRef` | Note that owns checkbox `c` |
| `local_checkboxes(n)` | `NoteRef → List(CheckboxRef)` | All checklist items in `n` |
| `children(c)` | `CheckboxRef → List(CheckboxRef)` | Direct child checkboxes |

### Status Operations

| Builtin | Signature | Description |
|---------|-----------|-------------|
| `aggregate_status(xs)` | `List(Status) → Status` | Ignores `none` when any concrete status exists; returns `none` only when all inputs are `none` or the list is empty |
| `done?` / `todo?` / `wip?` / `none?` | `Status → Bool` | Status predicates |
| `all_done(xs)` | `List(Status) → Bool` | True iff every element is `done` |

### Boolean Operations

| Builtin | Signature | Description |
|---------|-----------|-------------|
| `not(b)` | `Bool → Bool` | Logical negation |
| `and(b...)` / `or(b...)` | `Bool... → Bool` | Short-circuit connectives |
| `eq?(a, b)` | `T × T → Bool` | Structural equality |
| `nil?(v)` | `Any → Bool` | True iff `v` is `Nil` |

### Arithmetic

| Builtin | Signature | Description |
|---------|-----------|-------------|
| `+` / `-` | `Int × Int → Int` | Addition / subtraction |
| `<` / `>` / `<=` / `>=` | `Int × Int → Bool` | Integer comparisons |

### List Operations

| Builtin | Signature | Description |
|---------|-----------|-------------|
| `list(x...)` | `T... → List(T)` | Construct a list literal |
| `map(f, xs)` | `(T→U) × List(T) → List(U)` | Apply `f` to each element |
| `filter(f, xs)` | `(T→Bool) × List(T) → List(T)` | Keep elements where `f` is true |
| `reduce(f, init, xs)` | `(A×B→A) × A × List(B) → A` | Fold |
| `length(xs)` | `List(T) → Int` | Element count |
| `empty?(xs)` | `List(T) → Bool` | True iff list is empty |
| `union(xs, ys)` | `List(T) × List(T) → List(T)` | Union (no duplicates) |
| `contains?(xs, v)` | `List(T) × T → Bool` | Membership test |
| `dedup(xs)` | `List(T) → List(T)` | Remove duplicates (first occurrence wins) |

**Higher-order forms:** `map`, `filter`, and `reduce` take a *function
symbol* (not a value) — write the name unquoted:

```lisp
(map done? statuses)
(filter is_done notes)
(reduce + 0 counts)
```

---

## Load Order and Merge Semantics

```text
1. Built-in default module (examples/rules/checklist.lisp)
        ↓  (unless disable_default_reconcile_rules = true)
2. User-level [[reconcile.rule]] files (config.toml order)
        ↓
3. Project-level [[reconcile.rule]] files (zk-lsp.toml order)
```

When a later file defines a rule with the same name, it **replaces** the
earlier definition.  Helper rules that are not redefined are inherited.
`(policy ...)` is replaced only when the later file explicitly declares one.

---

## Configuration

```toml
# ~/.config/zk-lsp/config.toml
[[reconcile.rule]]
path = "~/.config/zk-lsp/reconcile/common.lisp"

# <wiki-root>/zk-lsp.toml
disable_default_reconcile_rules = false   # set true to start from scratch

[[reconcile.rule]]
path = "./reconcile/project_rules.lisp"
```

Rules are loaded fresh from disk on every `zk-lsp reconcile` invocation.

---

## Example Workflows

### 1 — Default checklist behavior (built-in)

This is `examples/rules/checklist.lisp` in the repository, the module that
ships with the binary:

```lisp
(module
  (policy
    (cycle error)
    (unknown-status none))

  (define (materialized_fields n)
    (list "checklist-status"))

  ;; Leaf checkboxes use their observed state; parents derive from children.
  (define (child_status c)
    (if (empty? (children c))
        done
        (aggregate_status (map effective_checked (children c)))))

  (define (local_status c)
    (if (empty? (children c))
        (observe_checked c)
        (child_status c)))

  ;; Ref-item targets must ALL be done before the checkbox is done.
  (define (targets_allow? c)
    (if (empty? (targets c))
        true
        (all_done (map target_status (targets c)))))

  (define (effective_checked c)
    (if (empty? (targets c))
        (local_status c)
        (if (targets_allow? c)
            (child_status c)
            todo)))

  (define (target_status n)
    (effective_meta n "checklist-status"))

  ;; Archived notes are always done; empty-checklist notes keep their
  ;; observed value; otherwise aggregate leaf states.
  (define (effective_meta n field)
    (if (eq? field "checklist-status")
        (if (eq? (observe_meta n "relation") "archived")
            done
            (if (empty? (local_checkboxes n))
                (observe_meta n "checklist-status")
                (aggregate_status (map effective_checked (local_checkboxes n)))))
        (observe_meta n field))))
```

### 2 — Backlink-verified badge

Mark a note `user.verified = "true"` when at least three *done* notes link
to it.  Demonstrates: graph queries, numeric comparisons, custom fields.

```lisp
;; examples/rules/backlink_verified.lisp
(module
  (policy (cycle error) (unknown-status none))

  (define (materialized_fields n)
    (list "checklist-status" "user.verified"))

  (define (done_backlink_count n)
    (length (filter is_done_note (backlinks n))))

  (define (is_done_note n)
    (done? (observe_meta n "checklist-status")))

  (define (effective_checked c)
    (observe_checked c))

  (define (effective_meta n field)
    (if (eq? field "user.verified")
        (if (>= (done_backlink_count n) 3) "true" "false")
        (observe_meta n field))))
```

To enable alongside the default checklist behavior:

```toml
# <wiki-root>/zk-lsp.toml
[[reconcile.rule]]
path = "./reconcile/backlink_verified.lisp"
```

Because both define `materialized_fields`, the last file wins.  Override
only that rule to merge both field lists:

```lisp
;; reconcile/backlink_verified.lisp  (project layer)
(module
  (define (materialized_fields n)
    (list "checklist-status" "user.verified"))

  ;; … rest of the rules from above …)
```

### 3 — Priority-aware status override (AI agent workflow)

An AI agent adds a `user.priority = "blocked"` field to a note.  The DSL
rule treats any blocked note as `wip` regardless of its checklist state:

```lisp
(module
  (define (materialized_fields n)
    (list "checklist-status"))

  (define (effective_checked c)
    (observe_checked c))

  (define (effective_meta n field)
    (if (eq? field "checklist-status")
        (if (eq? (observe_meta n "user.priority") "blocked")
            wip
            (observe_meta n "checklist-status"))
        (observe_meta n field))))
```

Combine with an AI agent pipeline:

```bash
# Agent marks a note as blocked
note_id="2603151158"
current=$(zk-lsp note-info "$note_id" | jq -r '.metadata["user"]["priority"] // "normal"')
if [ "$current" != "blocked" ]; then
  # patch the TOML via a Lua hook (or direct sed for simplicity in scripts):
  sed -i '' 's/priority = "normal"/priority = "blocked"/' "note/${note_id}.typ"
fi
# Propagate the new state across the wiki
zk-lsp reconcile
```

### 4 — Multi-field materialize (review workflow)

A research wiki where each note tracks both `checklist-status` and a
`user.review-state` (draft / reviewed / published):

```lisp
(module
  (define (materialized_fields n)
    (list "checklist-status" "user.review-state"))

  (define (effective_checked c)
    (observe_checked c))

  (define (effective_meta n field)
    (if (eq? field "checklist-status")
        (if (empty? (local_checkboxes n))
            (observe_meta n "checklist-status")
            (aggregate_status (map effective_checked (local_checkboxes n))))
        ;; review-state: auto-advance from "draft" to "reviewed"
        ;; when checklist-status becomes "done"
        (if (eq? field "user.review-state")
            (if (and (eq? (observe_meta n "user.review-state") "draft")
                     (done? (effective_meta n "checklist-status")))
                "reviewed"
                (observe_meta n "user.review-state"))
            (observe_meta n field)))))
```

### 5 — Minimal custom module (from scratch)

When `disable_default_reconcile_rules = true`, you must provide at least
these three entry points:

```lisp
(module
  (define (materialized_fields n)
    (list "checklist-status"))

  (define (effective_checked c)
    (observe_checked c))

  (define (effective_meta n field)
    (observe_meta n field)))
```

---

## Reconcile Run Lifecycle

```text
zk-lsp reconcile [--dry-run]
        │
        ├─ 1. Scan wiki → build note/checkbox/metadata snapshot
        ├─ 2. Load built-in module (unless disabled)
        ├─ 3. Load + merge [[reconcile.rule]] files in order
        ├─ 4. Parse + type-check the merged module
        ├─ 5. Evaluate: topological sort → effective_checked / effective_meta
        ├─ 6. Collect diagnostics (cycles → hard abort with source locations)
        └─ 7. Write back changed states (unless --dry-run)
               └─ fail if a declared materialized field cannot be patched
```

Use `--dry-run` to preview changes without writing files.

---

## Static Checks the Engine Performs

Following the graph-semantics model, the evaluator can detect several classes
of rule errors at parse / type-check time before any evaluation starts:

| Check | When detected |
|-------|---------------|
| Cycle in task-dependency graph | evaluation phase (hard error) |
| Write conflict (two rules materialize same field without priority) | load / merge |
| Unknown `@ID` reference | index phase |
| Type mismatch in rule body | type-check phase |
| Duplicate rule name | parse phase |
| Unknown policy key or value | parse phase |

---

## Tips for AI Agents

- `zk-lsp reconcile --dry-run` prints what would change without modifying
  files — safe to call from an agent pipeline to assess workspace state.
- `zk-lsp note-info <id>` returns JSON including all materialized `user.*`
  fields, so agents can read derived state without parsing Typst files.
- `zk-lsp export <id> --depth 2` produces a Markdown document containing
  the entry note plus up to two hops of linked context — useful as a
  context window for an LLM summarisation or tagging step.
- `zk-lsp export <id> --depth 2 --inverse` follows backlinks instead,
  listing ancestor notes first — useful for "what led to this idea?" queries.

---

## See Also

- [Lua Hooks guide](lua_hooks/index.html) — per-note format pipeline
- [`reconcile::types`](../zk_lsp/reconcile/types/index.html) — Rust type
  definitions for the DSL value system
- [`config::ZkLspConfig`](../zk_lsp/config/struct.ZkLspConfig.html) — full
  configuration struct reference
