//! `Workbench.evidence(criterion, artifact, revision, kind, passed)`
//! (#2273): a member records evidence for one of the run's criteria. The
//! coordinator's pass accepts it; anyone else's is a proposal. Completion
//! (`CompleteRun`) needs accepted evidence for every criterion; S10 builds
//! on it.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_completion::definition;
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{
    EvidenceTransition, NewEvidence, RecordEvidenceRequest, RecordedEvidence,
};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{
    Access, BoardError, RefusalKind, bounded, edited_criteria, python_equal,
};

/// The most bytes an artifact reference may take.
const ARTIFACT_MAX_BYTES: usize = 2048;
/// The most bytes an artifact revision may take.
const REVISION_MAX_BYTES: usize = 256;

/// The artifact and the revision are bounded before the operation gate
/// (any member of a running run). The criterion must name a configured
/// criterion (the first whose id equals it by Python's `==`) of the given
/// kind. The record is accepted only when the coordinator passes `true`
/// itself (Python's `is True`). The actor's earlier record for that
/// criterion, when equal in artifact, revision, kind and acceptance, makes
/// this a no-op; otherwise the row is written (replacing it) and the event
/// `evidence{criterion,artifact,revision,accepted}` records it.
pub struct RecordEvidence {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RecordEvidence {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// A bound, an authorisation or budget refusal, `evidence must match a
    /// configured criterion and kind`, criteria the board never writes, or
    /// the store's.
    ///
    /// Answers the evidence as recorded (#2394), the same row for a no-op.
    pub fn execute(&self, request: RecordEvidenceRequest) -> Result<RecordedEvidence, BoardError> {
        let artifact = bounded(&request.artifact, "artifact reference", ARTIFACT_MAX_BYTES)?;
        let revision = bounded(&request.revision, "artifact revision", REVISION_MAX_BYTES)?;
        let actor = request.actor.as_str();
        let running = Access {
            active: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            running,
            |transaction, run| {
                let criteria = transaction.run_criteria()?.ok_or_else(|| {
                    BoardError::new(RefusalKind::RunMissing, "coordination run missing")
                })?;
                let configured = definition(&criteria, &request.criterion)?;
                let kind = match configured {
                    Some(fields) => match (fields.get("kind"), request.kind.as_str()) {
                        (Some(stored), Some(kind)) if python_equal(stored, &request.kind) => kind,
                        (Some(_), _) => {
                            return Err(BoardError::new(
                                RefusalKind::Invalid,
                                "evidence must match a configured criterion and kind",
                            ));
                        }
                        (None, _) => return Err(edited_criteria()),
                    },
                    None => {
                        return Err(BoardError::new(
                            RefusalKind::Invalid,
                            "evidence must match a configured criterion and kind",
                        ));
                    }
                };
                let accepted = run.coordinator.as_deref() == Some(actor)
                    && request.passed == Value::Bool(true);
                let prior = transaction.prior_evidence(&request.criterion, actor)?;
                let same = prior.as_ref().is_some_and(|prior| {
                    python_equal(&prior.artifact, &text(artifact))
                        && python_equal(&prior.revision, &text(revision))
                        && python_equal(&prior.kind, &text(kind))
                        && python_equal(&prior.accepted, &Value::Bool(accepted))
                });
                let evidence = NewEvidence {
                    criterion: request.criterion.clone(),
                    artifact: artifact.to_owned(),
                    revision: revision.to_owned(),
                    kind: kind.to_owned(),
                    actor: actor.to_owned(),
                    accepted,
                };
                if let (true, Some(prior)) = (same, prior) {
                    return Ok(RecordedEvidence {
                        transition: EvidenceTransition::Unchanged,
                        evidence,
                        criterion: prior.criterion,
                    });
                }
                transaction.record_evidence(&evidence)?;
                let stored = transaction
                    .prior_evidence(&request.criterion, actor)?
                    .ok_or_else(|| {
                        BoardError::new(RefusalKind::Store, "the recorded evidence row is gone")
                    })?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "evidence",
                    &detail([
                        ("criterion", request.criterion.clone()),
                        ("artifact", text(artifact)),
                        ("revision", text(revision)),
                        ("accepted", Value::Bool(accepted)),
                    ]),
                )?;
                Ok(RecordedEvidence {
                    transition: EvidenceTransition::Recorded,
                    evidence,
                    criterion: stored.criterion,
                })
            },
        )
    }
}

impl OverRepository for RecordEvidence {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "record_evidence_tests.rs"]
mod tests;
