//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! criterion evidence and task evidence (#2273). The SQLite adapter's
//! contract tests pin the real SQL and the plain `json.dumps` text.
use serde_json::{Value, json};

use super::MemoryTransaction;
use crate::application::swarm::dto::{
    CompletionState, DictRow, EvidenceEntry, NewEvidence, PriorEvidence,
};
use crate::application::swarm::ports::BoardEvidence;
use crate::domain::swarm::BoardError;

/// A criterion as the `evidence` table's TEXT column stores it (#2394
/// final review N-2), as SQLite applies TEXT affinity to the bound value:
/// text as it is, a number (a bool binds as 0 or 1) as its text, NULL as
/// NULL.
fn stored_criterion(criterion: &Value) -> Value {
    match criterion {
        Value::Null | Value::String(_) => criterion.clone(),
        Value::Bool(flag) => Value::from(i64::from(*flag).to_string()),
        other => Value::from(other.to_string()),
    }
}

/// An evidence row `actor` recorded for `criterion` at `revision`, of
/// `kind`, accepted as the board stores it (`1`).
pub fn accepted(
    actor: &str,
    criterion: &str,
    revision: &str,
    kind: &str,
) -> (String, EvidenceEntry) {
    (
        actor.to_owned(),
        EvidenceEntry {
            criterion: json!(criterion),
            artifact: json!("log"),
            revision: json!(revision),
            kind: json!(kind),
            accepted: json!(1),
        },
    )
}

impl BoardEvidence for MemoryTransaction<'_> {
    fn completion_state(&self) -> Result<CompletionState, BoardError> {
        let state = self.state.borrow();
        let run = state.run.as_ref().expect("a run to complete");
        Ok(CompletionState {
            criteria: run.contract.criteria.clone(),
            evidence: state.evidence.iter().map(|(_, row)| row.clone()).collect(),
            tasks: state.tasks.clone(),
            has_reservations: !state.files.is_empty(),
        })
    }

    fn replace_task_evidence(&self, id: &Value, evidence: &Value) -> Result<(), BoardError> {
        self.note(format!("replace_task_evidence {id}"));
        self.change_task(id, |row| row.set("evidence", evidence.clone()));
        Ok(())
    }

    fn delete_all_evidence(&self) -> Result<(), BoardError> {
        self.note("delete_all_evidence".to_owned());
        self.state.borrow_mut().evidence.clear();
        Ok(())
    }

    fn prior_evidence(
        &self,
        criterion: &Value,
        actor: &str,
    ) -> Result<Option<PriorEvidence>, BoardError> {
        Ok(self
            .state
            .borrow()
            .evidence
            .iter()
            .find(|(by, row)| by == actor && row.criterion == stored_criterion(criterion))
            .map(|(_, row)| PriorEvidence {
                criterion: row.criterion.clone(),
                artifact: row.artifact.clone(),
                revision: row.revision.clone(),
                kind: row.kind.clone(),
                accepted: row.accepted.clone(),
            }))
    }

    fn evidence_rows(&self) -> Result<Vec<DictRow>, BoardError> {
        let state = self.state.borrow();
        Ok(state
            .evidence
            .iter()
            .map(|(actor, entry)| DictRow {
                columns: vec![
                    ("criterion".to_owned(), entry.criterion.clone()),
                    ("artifact".to_owned(), entry.artifact.clone()),
                    ("revision".to_owned(), entry.revision.clone()),
                    ("kind".to_owned(), entry.kind.clone()),
                    ("actor".to_owned(), Value::from(actor.as_str())),
                    ("accepted".to_owned(), entry.accepted.clone()),
                ],
            })
            .collect())
    }

    fn record_evidence(&self, evidence: &NewEvidence) -> Result<(), BoardError> {
        self.note(format!("record_evidence {}", evidence.criterion));
        let mut state = self.state.borrow_mut();
        let criterion = stored_criterion(&evidence.criterion);
        state
            .evidence
            .retain(|(by, row)| !(by == &evidence.actor && row.criterion == criterion));
        state.evidence.push((
            evidence.actor.clone(),
            EvidenceEntry {
                criterion,
                artifact: json!(evidence.artifact),
                revision: json!(evidence.revision),
                kind: json!(evidence.kind),
                accepted: json!(i64::from(evidence.accepted)),
            },
        ));
        Ok(())
    }
}
