//! `Workbench.evidence` (#2273).
use std::sync::Arc;

use crate::application::swarm::dto::RecordEvidenceRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct RecordEvidence {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RecordEvidence {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending.
    pub fn execute(
        &self,
        request: RecordEvidenceRequest,
    ) -> Result<crate::application::swarm::dto::EvidenceTransition, BoardError> {
        let _ = (&self.repository, &self.clock, request);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "record_evidence_tests.rs"]
mod tests;
