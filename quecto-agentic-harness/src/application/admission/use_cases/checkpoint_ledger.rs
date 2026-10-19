//! Persist the authority's current ledger: the initial checkpoint a starting
//! broker writes before it accepts work (#1679 P3, #1910).

use super::durable_ledger::persist_current;
use crate::application::admission::dto::broker::AuthorityError;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;

pub struct CheckpointLedger {
    journal: SharedJournal,
}

impl CheckpointLedger {
    pub fn new(journal: SharedJournal) -> Self {
        Self { journal }
    }

    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        now: u64,
    ) -> Result<(), AuthorityError> {
        persist_current(&self.journal, authority, now)
    }
}
