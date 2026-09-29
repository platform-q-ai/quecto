//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! criterion evidence and task evidence (#2273). The SQLite adapter's
//! contract tests pin the real SQL and the plain `json.dumps` text.
use serde_json::{Value, json};

use super::MemoryTransaction;
use crate::application::swarm::dto::{CompletionState, EvidenceEntry, NewEvidence, PriorEvidence};
use crate::application::swarm::ports::BoardEvidence;
use crate::domain::swarm::BoardError;

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
            .find(|(by, row)| by == actor && &row.criterion == criterion)
            .map(|(_, row)| PriorEvidence {
                artifact: row.artifact.clone(),
                revision: row.revision.clone(),
                kind: row.kind.clone(),
                accepted: row.accepted.clone(),
            }))
    }

    fn record_evidence(&self, evidence: &NewEvidence) -> Result<(), BoardError> {
        self.note(format!("record_evidence {}", evidence.criterion));
        let mut state = self.state.borrow_mut();
        state
            .evidence
            .retain(|(by, row)| !(by == &evidence.actor && row.criterion == evidence.criterion));
        state.evidence.push((
            evidence.actor.clone(),
            EvidenceEntry {
                criterion: evidence.criterion.clone(),
                artifact: json!(evidence.artifact),
                revision: json!(evidence.revision),
                kind: json!(evidence.kind),
                accepted: json!(i64::from(evidence.accepted)),
            },
        ));
        Ok(())
    }
}
