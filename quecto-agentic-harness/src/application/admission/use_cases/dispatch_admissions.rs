//! Dispatch every grant the admission policy allows, each durable before it
//! is visible (#1679 P3, #1910).

use super::durable_ledger::persist_current;
use crate::application::admission::dto::broker::AuthorityError;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::RequestId;

pub struct DispatchAdmissions {
    journal: SharedJournal,
}

impl DispatchAdmissions {
    pub fn new(journal: SharedJournal) -> Self {
        Self { journal }
    }

    /// Dispatch every grant the policy allows, making each durable first. On
    /// a journal failure the undelivered grants are withdrawn and no grant
    /// is returned until a later durable write succeeds. While the journal
    /// is known unhealthy a probe write precedes any selection, so queued
    /// work is held rather than withdrawn one grant at a time.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        now: u64,
    ) -> Result<Vec<RequestId>, AuthorityError> {
        if !authority.journal_healthy() {
            persist_current(&self.journal, authority, now)?;
        }
        let (grants, first_error) = authority.select_grants(now);
        if grants.is_empty() {
            return match first_error {
                Some(error) => Err(error.into()),
                None => Ok(grants),
            };
        }
        if persist_current(&self.journal, authority, now).is_err() {
            authority.withdraw_grants(&grants, now)?;
            return Err(AuthorityError::JournalUnavailable);
        }
        Ok(grants)
    }
}
