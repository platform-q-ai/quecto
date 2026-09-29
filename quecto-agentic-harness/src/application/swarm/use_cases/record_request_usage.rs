//! `Workbench._record_request(record)` (#2274): the harness records one
//! model request a member made, and the token budget applies.
use std::sync::Arc;

use crate::application::swarm::dto::{RecordRequestUsageRequest, RecordedRequest};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::BoardError;

/// The record is measured before the operation gate
/// (`request_measurement`). Through the gate for reading (any member, a
/// dead one included) its encoded text is bounded to 32,768 bytes; a
/// request id the ledger holds is a redelivery (the same actor and record,
/// the runtime's digest possibly now known, which replaces the stored
/// record) or refused; a new one is inserted while the ledger holds fewer
/// than 10,000 rows. The budget then applies, and the answer is the
/// control receipt.
pub struct RecordRequestUsage {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl RecordRequestUsage {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// An invalid record, an oversized one, a reused request id, a full
    /// ledger, an authorisation refusal, an edited record, or the store's.
    pub fn execute(
        &self,
        request: RecordRequestUsageRequest,
    ) -> Result<RecordedRequest, BoardError> {
        let _ = (&self.repository, &self.clock, &self.encoding, request);
        Err(BoardError::new("pending #2274"))
    }
}

#[cfg(test)]
#[path = "record_request_usage_tests.rs"]
mod tests;
