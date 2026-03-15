# Lua Hooks — Format Pipeline

`zk-lsp format` runs a pipeline of Lua scripts against each note on every
save. Hooks are the primary extension point for **per-note transformations**:
normalising checkboxes, computing tags, enforcing local conventions, or
generating derived metadata from note content.

Cross-file logic (propagating done-states between notes) belongs to the
[Reconcile DSL](reconcile_dsl/index.html), not here.

---

## Concepts

```text
┌──────────────────────────────────────────────────────────┐
│  willSaveWaitUntil (every save)                          │
│                                                          │
│   parse note                                             │
│       │                                                  │
│       ▼                                                  │
│   built-in hooks (checklist.lua, relation_status.lua)    │
│       │   (unless disable_default_hooks = true)          │
│       ▼                                                  │
│   user hooks  [[hook]] path = "…"                        │
│       │   (user-level first, then project-level)         │
│       ▼                                                  │
│   apply HookResult { metadata, edits }                   │
└──────────────────────────────────────────────────────────┘
```

Each hook is a Lua 5.4 file containing a top-level `run(note)` function.
Hooks are stateless: they receive the current note and return patches; they
cannot read other notes or the wider wiki graph.

---

## Hook Input — `NoteInput`

The `note` argument passed to `run` is a Lua table matching the
[`HookNoteInput`](../zk_lsp/hooks/types/struct.HookNoteInput.html) Rust type:

```lua
---@class NoteInput
---@field id         string              -- 10-digit note ID (YYMMDDHHMM)
---@field title      Title|nil           -- title heading; nil if unparseable
---@field content    string              -- full raw note content
---@field metadata   table<string, any>  -- TOML metadata key→value pairs
---@field checkboxes Checkbox[]          -- all checklist items in source order
---@field headings   Heading[]           -- all headings (level 1..6)
```

### `Title`

```lua
---@class Title
---@field text string   -- heading text without the `= ` prefix or `<ID>` label
---@field span Span
```

### `Checkbox`

Mirrors [`HookCheckbox`](../zk_lsp/hooks/types/struct.HookCheckbox.html):

```lua
---@class Checkbox
---@field id       string    -- "local:{line_idx}" for local; first target_id for ref
---@field kind     string    -- "local" | "ref"
---@field checked  boolean
---@field targets  string[]  -- @ID tokens; empty for local items
---@field text     string    -- body text after "- [x] "
---@field span     Span      -- full-line byte range
---@field line_idx integer   -- 0-based line index inside the note
---@field indent   integer   -- leading space count (indentation level)
```

`line_idx` and `indent` let hooks sort checkboxes and build the
parent–child tree without any string parsing.

### `Span`

Mirrors [`HookSpan`](../zk_lsp/hooks/types/struct.HookSpan.html):

```lua
---@class Span
---@field start_byte  integer
---@field end_byte    integer
---@field start_line  integer  -- 0-based
---@field start_col   integer  -- 0-based byte column
---@field end_line    integer
---@field end_col     integer
```

### `Heading`

```lua
---@class Heading
---@field level integer   -- 1 = "=", 2 = "==", …
---@field text  string
---@field span  Span
```

---

## Hook Output — `HookResult`

`run(note)` must return a table (or an empty table `{}`):

```lua
---@class HookResult
---@field metadata  table<string, any>|nil   -- metadata keys to patch
---@field edits     TextEdit[]|nil           -- byte-range text replacements
```

Mirrors [`HookResult`](../zk_lsp/hooks/types/struct.HookResult.html).

### `TextEdit`

Mirrors [`HookTextEdit`](../zk_lsp/hooks/types/struct.HookTextEdit.html):

```lua
---@class TextEdit
---@field start_byte integer
---@field end_byte   integer
---@field text       string   -- replacement text (may be empty to delete)
```

Edits are applied in **reverse byte order** so that earlier byte offsets
remain valid as later edits are committed.

---

## Configuration

```toml
# ~/.config/zk-lsp/config.toml   (user-level)
# <wiki-root>/zk-lsp.toml        (project-level; merged after user)

# Disable the two built-in hooks entirely:
disable_default_hooks = true

# Add file hooks (user hooks run first, then project hooks):
[[hook]]
path = "~/.config/zk-lsp/hooks/my_hook.lua"

[[hook]]
path = "./hooks/project_hook.lua"
```

Hooks are loaded fresh from disk on every `zk-lsp format` invocation —
editing a `.lua` file takes effect on the next save without recompiling.

---

## Built-in Hooks

Two hooks are embedded in the binary and run by default:

| Hook | Effect |
|---|---|
| `checklist.lua` | Propagates nested checkbox states bottom-up; sets `checklist-status` from leaf nodes |
| `relation_status.lua` | Forces `checklist-status = "done"` when `relation` is `"archived"` or `"legacy"` |

Source lives under `examples/hooks/` in the repository.

---

## Example Workflows

### 1 — Tag normaliser

Ensure every note with a `user.course` field also has the corresponding tag
in its content. Useful for wikis where tags drive searches.

```lua
-- hooks/tag_sync.lua
-- If metadata contains user.course, emit an edit to ensure the
-- course name appears in the note body after the title line.

function run(note)
  local course = note.metadata["user"] and note.metadata["user"]["course"]
  if not course or course == "" then
    return {}
  end

  local tag = "#tag." .. course:lower():gsub("%s+", "_")
  if note.content:find(tag, 1, true) then
    return {}   -- already present
  end

  -- Find the byte position just after the title line
  local title_end = note.title and note.title.span.end_byte or 0
  return {
    edits = {
      { start_byte = title_end, end_byte = title_end, text = "\n" .. tag }
    }
  }
end
```

### 2 — Auto-abstract generator (AI agent hook)

Call an external CLI to populate `abstract` when the field is empty.
This is the pattern to use from a scripting pipeline (e.g. a Git pre-commit
hook or a nightly cron job that pipes each note through `zk-lsp format`).

```lua
-- hooks/auto_abstract.lua
local function shell(cmd)
  local f = io.popen(cmd, "r")
  if not f then return nil end
  local s = f:read("*a")
  f:close()
  return s and s:match("^%s*(.-)%s*$")  -- trim whitespace
end

function run(note)
  local meta = note.metadata or {}
  if meta["abstract"] and meta["abstract"] ~= "" then
    return {}   -- already set; honour existing value
  end

  -- Write note content to a temp file and call your summariser
  local tmp = os.tmpname()
  local f = io.open(tmp, "w")
  if not f then return {} end
  f:write(note.content)
  f:close()

  local summary = shell("my-summariser --format=one-line " .. tmp)
  os.remove(tmp)

  if not summary or summary == "" then return {} end
  return { metadata = { abstract = summary } }
end
```

Invoke from a Git pre-commit hook:

```bash
#!/usr/bin/env bash
# .git/hooks/pre-commit
set -euo pipefail
for f in $(git diff --cached --name-only -- 'note/*.typ'); do
  zk-lsp format < "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  git add "$f"
done
```

### 3 — Priority badge

Map `user.priority` to a visual badge at the top of each note body.

```lua
-- hooks/priority_badge.lua
local BADGES = { high = "🔴", normal = "🟡", low = "🟢" }

function run(note)
  local user = note.metadata["user"] or {}
  local priority = user["priority"] or "normal"
  local badge = BADGES[priority] or ""
  if badge == "" then return {} end

  -- Insert after the first heading
  if not note.title then return {} end
  local insert_pos = note.title.span.end_byte

  local marker = "<!-- priority:" .. priority .. " -->"
  if note.content:find(marker, 1, true) then
    return {}   -- already annotated
  end

  return {
    edits = {
      { start_byte = insert_pos, end_byte = insert_pos,
        text = "\n" .. badge .. " " .. marker }
    }
  }
end
```

### 4 — Word-count metadata (AI agent pipeline example)

Useful for an AI agent that wants to skip summarising short notes.

```lua
-- hooks/word_count.lua
function run(note)
  local count = 0
  for _ in note.content:gmatch("%S+") do count = count + 1 end
  return { metadata = { ["user"] = { ["word-count"] = count } } }
end
```

An external script can then query the field with `zk-lsp note-info <id>` and
decide whether the note needs further processing.

---

## Tips for AI Agents

- Use `zk-lsp note-info <id>` to inspect a note's current metadata as JSON
  before deciding whether a hook needs to run.
- Pipe through `zk-lsp format` at the end of any pipeline that modifies a
  note — it re-normalises checkboxes and applies all hooks.
- Combine `zk-lsp export <id> --depth 2` with your LLM call to get the
  surrounding context (linked notes) in a single Markdown document.
- `zk-lsp check` exits 1 on dead links — use it as a CI gate after bulk
  note creation or deletion.

---

## Full EmmyLua Type Reference

See `lua/zk_hook_types.lua` in the repository for the complete EmmyLua
annotations. The Rust types they mirror are in the
[`hooks::types`](../zk_lsp/hooks/types/index.html) module.
