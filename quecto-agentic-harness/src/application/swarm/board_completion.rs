//! How the completion use cases (#2273) read what `BoardEvidence` and
//! `BoardRuns` answer: the run's criteria, the evidence rows and the tasks
//! as the domain's `completion`, `revalidation` and the `evidence` method
//! read them. Capability-internal helpers, not a use case and not a port.
//!
//! The board writes criteria only through `create` and `amend`, which
//! validate them: a list of objects with a text id and a `command` or
//! `review` kind. Criteria in any other shape are found only in a file
//! edited outside the board and are refused naming the record, where
//! Python raises or compares what it finds (the `outside_edited_contract`
//! divergence).
use serde_json::{Map, Value};

use super::dto::{EvidenceEntry, TaskRow};
use crate::domain::swarm::{
    BoardError, Criterion, CriterionKind, EvidenceRow, TaskRecord, TaskState, python_equal,
    python_truthy,
};

/// The refusal of criteria the board never writes.
pub(crate) fn edited_criteria() -> BoardError {
    BoardError::new("the board's run criteria is not as the board writes it")
}

/// The run's criteria as `completion` reads them: each entry's id and kind
/// (the description is not read, and is kept only when it is text).
///
/// # Errors
/// [`edited_criteria`] for anything but a list of objects each with a text
/// id and a known kind.
pub(crate) fn criteria(value: &Value) -> Result<Vec<Criterion>, BoardError> {
    let Value::Array(entries) = value else {
        return Err(edited_criteria());
    };
    entries
        .iter()
        .map(|entry| {
            let id = entry.get("id").and_then(Value::as_str);
            let kind = entry
                .get("kind")
                .and_then(Value::as_str)
                .and_then(CriterionKind::parse);
            match (entry.is_object(), id, kind) {
                (true, Some(id), Some(kind)) => Ok(Criterion {
                    id: id.to_owned(),
                    kind,
                    description: entry
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }),
                _ => Err(edited_criteria()),
            }
        })
        .collect()
}

/// The evidence rows that can satisfy a criterion: those whose criterion,
/// revision and kind are text and whose kind is known. Any other row (only
/// an edit leaves one) equals no criterion's id and kind, as Python's
/// comparisons find; `accepted` is Python's truthiness of the stored value.
pub(crate) fn evidence_rows(entries: &[EvidenceEntry]) -> Vec<EvidenceRow> {
    entries
        .iter()
        .filter_map(|entry| {
            let kind = entry.kind.as_str().and_then(CriterionKind::parse)?;
            Some(EvidenceRow {
                criterion: entry.criterion.as_str()?.to_owned(),
                revision: entry.revision.as_str()?.to_owned(),
                kind,
                accepted: python_truthy(&entry.accepted),
            })
        })
        .collect()
}

/// A task row as the domain policy reads it: a NULL or non-text status (an
/// edit) is no known status, so it is never `completed`.
pub(crate) fn task_record(task: &TaskRow) -> TaskRecord {
    TaskRecord {
        id: task.get("id").and_then(Value::as_i64).unwrap_or_default(),
        status: TaskState::new(task.text("status").unwrap_or_default()),
        owner: task.text("owner").map(str::to_owned),
        evidence: task.get("evidence").cloned().unwrap_or(Value::Null),
    }
}

/// `next((c for c in criteria if c['id'] == criterion), None)`: the first
/// entry whose id equals `criterion` by Python's `==`, read in order.
///
/// # Errors
/// [`edited_criteria`] where Python raises: criteria that are not a list,
/// or an entry read before the match that is not an object with an id.
pub(crate) fn definition<'c>(
    criteria: &'c Value,
    criterion: &Value,
) -> Result<Option<&'c Map<String, Value>>, BoardError> {
    let Value::Array(entries) = criteria else {
        return Err(edited_criteria());
    };
    for entry in entries {
        let Some((fields, id)) = entry
            .as_object()
            .and_then(|fields| fields.get("id").map(|id| (fields, id)))
        else {
            return Err(edited_criteria());
        };
        if python_equal(id, criterion) {
            return Ok(Some(fields));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "board_completion_tests.rs"]
mod tests;
