//! Core types for the Reconcile DSL v1.
//!
//! The DSL implements the observe → effective → materialize model described
//! in the [Reconcile DSL guide](https://docs.rs/zk-lsp/latest/zk_lsp/reconcile_dsl/).
//!
//! At evaluation time, the engine:
//! 1. Builds a workspace snapshot (notes, checkboxes, typed metadata)
//! 2. Loads and merges DSL modules from disk
//! 3. Type-checks the merged module against the builtin surface
//! 4. Evaluates `effective_checked` / `effective_meta` in topological order
//! 5. Writes back fields declared by `materialized_fields`

use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;

/// 10-digit string note identifier (`YYMMDDHHMM`).
pub type NoteId = String;

/// Stable identifier for a checklist item: note ID plus 0-based line index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CheckboxId {
    /// Note that contains this checklist item.
    pub note_id: NoteId,
    /// 0-based line index within the note content.
    pub line_idx: usize,
}

impl fmt::Display for CheckboxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.note_id, self.line_idx)
    }
}

/// Note / checklist status values used throughout the DSL.
///
/// Maps to the `checklist-status` TOML field and to the `Status` DSL type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// No checklist — status not applicable.
    None,
    /// At least one item incomplete, none in progress.
    Todo,
    /// Mixed: some done, some not.
    Wip,
    /// All leaf items complete.
    Done,
}

impl Status {
    #[allow(dead_code)]
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }

    pub fn to_str(&self) -> &'static str {
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

    pub fn aggregate(statuses: &[Self]) -> Self {
        let applicable: Vec<Self> = statuses
            .iter()
            .copied()
            .filter(|status| *status != Self::None)
            .collect();

        if applicable.is_empty() {
            return Self::None;
        }
        if applicable.iter().all(|status| *status == Self::Done) {
            return Self::Done;
        }
        if applicable.iter().all(|status| *status == Self::Todo) {
            return Self::Todo;
        }
        Self::Wip
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

/// Checkbox writeback directives used during the materialize phase.
///
/// These control how a checkbox line should be written back to disk after
/// `effective_checked` has produced a semantic `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckboxWriteback {
    /// Leave the source checkbox text unchanged.
    Keep,
    /// Materialize as `- [ ]`.
    Unchecked,
    /// Materialize as `- [x]`.
    Checked,
}

impl CheckboxWriteback {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Unchecked => "unchecked",
            Self::Checked => "checked",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "keep" => Some(Self::Keep),
            "unchecked" => Some(Self::Unchecked),
            "checked" => Some(Self::Checked),
            _ => None,
        }
    }
}

impl fmt::Display for CheckboxWriteback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

/// Runtime value in the Reconcile DSL evaluator.
///
/// The type system is checked statically before evaluation; these variants
/// are the concrete runtime representations.  Users never interact with
/// `Value` directly — it is the evaluator's internal currency.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    /// Logical `true` / `false`.
    Bool(bool),
    /// 64-bit signed integer.
    Int(i64),
    /// Absence value; returned by `parent` for root checklist items.
    Nil,
    /// One of `none` / `todo` / `wip` / `done`.
    Status(Status),
    /// One of `keep` / `unchecked` / `checked`.
    CheckboxWriteback(CheckboxWriteback),
    /// Homogeneous list; shared via reference-counting.
    List(Rc<Vec<Value>>),
    /// Runtime handle to a note (evaluator-internal).
    NoteRef(NoteId),
    /// Runtime handle to a checklist item (evaluator-internal).
    CheckboxRef(CheckboxId),
    /// String value returned by `observe_meta` for non-Status metadata fields.
    String(Rc<str>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Any,
    Bool,
    Int,
    Nil,
    Status,
    CheckboxWriteback,
    String,
    NoteRef,
    CheckboxRef,
    List(Box<Type>),
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Any => write!(f, "Any"),
            Type::Bool => write!(f, "Bool"),
            Type::Int => write!(f, "Int"),
            Type::Nil => write!(f, "Nil"),
            Type::Status => write!(f, "Status"),
            Type::CheckboxWriteback => write!(f, "CheckboxWriteback"),
            Type::String => write!(f, "String"),
            Type::NoteRef => write!(f, "NoteRef"),
            Type::CheckboxRef => write!(f, "CheckboxRef"),
            Type::List(inner) => write!(f, "List({inner})"),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    UnexpectedEof,
    UnexpectedToken { got: String, expected: String },
    InvalidExprHead(String),
    DuplicateRule(String),
    InvalidPolicyKey(String),
    InvalidPolicyValue { key: String, value: String },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedEof => write!(f, "unexpected end of input"),
            ParseError::UnexpectedToken { got, expected } => {
                write!(f, "unexpected token '{got}', expected {expected}")
            }
            ParseError::InvalidExprHead(s) => write!(f, "invalid expression head '{s}'"),
            ParseError::DuplicateRule(s) => write!(f, "duplicate rule name '{s}'"),
            ParseError::InvalidPolicyKey(k) => write!(f, "unknown policy key '{k}'"),
            ParseError::InvalidPolicyValue { key, value } => {
                write!(f, "invalid value '{value}' for policy key '{key}'")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    UnknownVariable(String),
    UnknownFunction(String),
    TypeMismatch {
        expected: Type,
        got: Type,
    },
    IfBranchMismatch {
        then_type: Type,
        else_type: Type,
    },
    WrongArgCount {
        name: String,
        expected: usize,
        got: usize,
    },
    UnsupportedHigherOrderArg {
        name: String,
    },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::UnknownVariable(v) => write!(f, "unknown variable '{v}'"),
            TypeError::UnknownFunction(n) => write!(f, "unknown function '{n}'"),
            TypeError::TypeMismatch { expected, got } => {
                write!(f, "type mismatch: expected {expected}, got {got}")
            }
            TypeError::IfBranchMismatch {
                then_type,
                else_type,
            } => {
                write!(
                    f,
                    "if branch type mismatch: then={then_type}, else={else_type}"
                )
            }
            TypeError::WrongArgCount {
                name,
                expected,
                got,
            } => {
                write!(f, "'{name}' expected {expected} args, got {got}")
            }
            TypeError::UnsupportedHigherOrderArg { name } => {
                write!(f, "unsupported higher-order argument in '{name}'")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    UnknownVariable(String),
    UnknownFunction(String),
    TypeMismatch { context: String },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::UnknownVariable(v) => write!(f, "unknown variable '{v}'"),
            EvalError::UnknownFunction(n) => write!(f, "unknown function '{n}'"),
            EvalError::TypeMismatch { context } => write!(f, "type mismatch in {context}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagnosticKind {
    Cycle,
    EvalFallback,
    UnknownMetadataField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLocation {
    pub file_path: PathBuf,
    pub line: usize,
    pub byte_start: u32,
    pub byte_end: u32,
}

#[derive(Debug, Clone)]
pub struct ReconcileDiagnostic {
    pub note_id: NoteId,
    pub message: String,
    #[allow(dead_code)]
    pub kind: DiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub location: Option<DiagnosticLocation>,
    pub related_locations: Vec<DiagnosticLocation>,
}
