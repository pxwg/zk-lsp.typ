/// Materialize layer — v1 identity projection.
use std::collections::HashMap;

use super::eval::EvalResult;
use super::types::{CheckboxId, CheckboxWriteback, NoteId, Value};

/// v1: materialized == effective (identity projection).
pub struct ReconcileResult {
    #[allow(dead_code)]
    pub materialized_meta: HashMap<(NoteId, String), Value>,
    pub materialized_checked: HashMap<CheckboxId, CheckboxWriteback>,
}

/// v1: identity — materialized == effective.
pub fn materialize(eval: EvalResult) -> ReconcileResult {
    ReconcileResult {
        materialized_meta: eval.effective_meta,
        materialized_checked: eval.materialized_checked,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::metadata::MetadataRecord;
    use crate::parser::{ChecklistStatus, Relation};
    use crate::reconcile::default_module::DEFAULT_MODULE;
    use crate::reconcile::eval::eval_all;
    use crate::reconcile::observe::WorkspaceSnapshot;
    use crate::reconcile::parser::parse_module;
    use crate::reconcile::types::Status;

    #[test]
    fn v1_identity() {
        let content = "#import \"../include.typ\": *\n\
             #let zk-metadata = zk_metadata(\"1111111111\")\n\
             #show: zettel.with(metadata: zk-metadata)\n\
             \n\
             = Test <1111111111>\n";
        let map: std::collections::HashMap<NoteId, (PathBuf, String)> = [(
            "1111111111".to_string(),
            (PathBuf::from("1111111111.typ"), content.to_string()),
        )]
        .into_iter()
        .collect();
        let metadata_records = [(
            "1111111111".to_string(),
            MetadataRecord {
                checklist_status: ChecklistStatus::Done,
                relation: Relation::Active,
                ..MetadataRecord::default()
            },
        )]
        .into_iter()
        .collect();
        let snap =
            WorkspaceSnapshot::from_note_map_with_metadata_records(&map, &metadata_records, &[]);
        let module = parse_module(DEFAULT_MODULE).expect("parse");
        let eval_result = eval_all(&module, &snap);

        let result = materialize(eval_result);

        // checklist-status now materializes from the generic meta map.
        assert_eq!(
            result
                .materialized_meta
                .get(&("1111111111".to_string(), "checklist-status".to_string())),
            Some(&Value::Status(Status::Done))
        );
    }
}
