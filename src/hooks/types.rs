use std::collections::HashMap;

/// Byte and line/column span for a node in the note.
///
/// All offsets are **byte-based** within the raw note content string.
/// Line and column numbers are 0-based.
#[derive(Debug, Clone)]
pub struct HookSpan {
    /// Byte offset of the first byte of the span (inclusive).
    pub start_byte: usize,
    /// Byte offset just past the last byte of the span (exclusive).
    pub end_byte: usize,
    /// 0-based line number of the first character.
    pub start_line: usize,
    /// 0-based byte column of the first character.
    pub start_col: usize,
    /// 0-based line number of the last character.
    pub end_line: usize,
    /// 0-based byte column just past the last character.
    pub end_col: usize,
}

/// The title heading of the note with its source location.
///
/// Extracted from the `= Title <ID>` line.
#[derive(Debug, Clone)]
pub struct HookTitle {
    /// Heading text without the `= ` prefix or `<ID>` label.
    pub text: String,
    /// Source span of the full heading line.
    pub span: HookSpan,
}

/// A checklist item exposed to Lua hooks.
///
/// Both local items (`- [ ] text`) and ref items (`- [ ] @ID`) are
/// represented by this struct. Hooks can distinguish them via [`kind`].
///
/// [`kind`]: HookCheckbox::kind
#[derive(Debug, Clone)]
pub struct HookCheckbox {
    /// Stable string identifier.
    ///
    /// - Local items: `"local:{line_idx}"`
    /// - Ref items: the first `@ID` target string
    pub id: String,
    /// `"local"` for plain checkboxes; `"ref"` for `@ID`-bearing items.
    pub kind: String,
    /// Whether the checkbox is currently checked in the source file.
    pub checked: bool,
    /// `@ID` target note IDs; empty for local items.
    pub targets: Vec<String>,
    /// Body text after `- [x] ` (the descriptive label).
    pub text: String,
    /// Full-line source span.
    pub span: HookSpan,
    /// 0-based line index of this item within the note content.
    ///
    /// Use together with [`indent`] to reconstruct the parent–child tree
    /// without additional string parsing.
    ///
    /// [`indent`]: HookCheckbox::indent
    pub line_idx: usize,
    /// Number of leading spaces before the `- ` marker.
    pub indent: usize,
}

/// A heading in the note (`=` through `======`).
#[derive(Debug, Clone)]
pub struct HookHeading {
    /// Heading level: 1 for `=`, 2 for `==`, etc.
    pub level: u32,
    /// Heading text without the leading `=` markers.
    pub text: String,
    /// Source span of the full heading line.
    pub span: HookSpan,
}

/// Full note representation passed to every Lua hook's `run(note)` function.
///
/// The `metadata` table mirrors the TOML block verbatim, including any
/// `[user]` sub-table for custom fields declared in the project config.
///
/// See the [Lua Hooks guide](https://docs.rs/zk-lsp/latest/zk_lsp/lua_hooks/)
/// for field-by-field documentation and example hook implementations.
#[derive(Debug, Clone)]
pub struct HookNoteInput {
    /// 10-digit timestamp ID (`YYMMDDHHMM`).
    pub id: String,
    /// Parsed title heading, or `None` when the heading line cannot be found.
    pub title: Option<HookTitle>,
    /// Full raw note content as a UTF-8 string.
    pub content: String,
    /// Parsed TOML metadata block (key → value).
    pub metadata: toml::Table,
    /// Default values for config-declared metadata fields.
    ///
    /// The shape mirrors TOML layout exposed in [`metadata`]. For example,
    /// `user.priority` appears as `{ user = { priority = "normal" } }`.
    pub metadata_defaults: toml::Table,
    /// All checklist items in source order.
    pub checkboxes: Vec<HookCheckbox>,
    /// All headings in source order.
    pub headings: Vec<HookHeading>,
    /// Config-declared metadata fields in declaration order.
    pub metadata_fields: Vec<HookMetadataField>,
}

/// A config-declared metadata field exposed to Lua hooks.
#[derive(Debug, Clone)]
pub struct HookMetadataField {
    /// Dotted field path, e.g. `user.priority`.
    pub path: String,
    /// Declared kind string: `string`, `boolean`, or `array-string`.
    pub kind: String,
    /// Default value from config.
    pub default: toml::Value,
}

/// A single byte-range text replacement returned by a hook.
///
/// The range `[start_byte, end_byte)` is replaced with `text`.
/// Edits are applied in **reverse byte order** so earlier offsets remain
/// valid as later edits are committed.
#[derive(Debug, Clone)]
pub struct HookTextEdit {
    /// Inclusive start byte offset.
    pub start_byte: usize,
    /// Exclusive end byte offset.
    pub end_byte: usize,
    /// Replacement text (may be empty to perform a deletion).
    pub text: String,
}

/// The value returned by the Lua `run(note)` function.
///
/// Both fields are optional.  Return an empty table `{}` when the hook has
/// no changes to apply.
///
/// ```lua
/// function run(note)
///   return {
///     metadata = { ["checklist-status"] = "done" },
///     edits    = { { start_byte = 42, end_byte = 43, text = "x" } },
///   }
/// end
/// ```
#[derive(Debug, Clone, Default)]
pub struct HookResult {
    /// Metadata keys to patch into the TOML block.
    ///
    /// Keys are merged into the existing block; existing keys not listed here
    /// are left untouched.
    pub metadata: HashMap<String, toml::Value>,
    /// Byte-range text edits to apply to the note content.
    pub edits: Vec<HookTextEdit>,
}
