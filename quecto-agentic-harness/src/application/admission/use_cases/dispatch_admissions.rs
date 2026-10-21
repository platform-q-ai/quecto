//! Dispatch every grant the admission policy allows, each durable before it
//! is visible (#1679 P3, #1910).

use super::durable_ledger::persist_current;
use crate::application::admission::dto::broker::AuthorityError;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::RequestId;

/// The durability sequence of a dispatch on its own. Settling a turn
/// (`SettleAuthority`) is its one production caller and composition builds
/// it into that; it stays a use case of its own because it is the unit the
/// journal contract tests drive directly (grant before visibility, withdrawal
/// on a failed write, the unhealthy probe), with its errors unflattened.
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
            // A lease that cleared on time left no write of its own: make
            // that durable, so a restart does not bring it back (#2664).
            if authority.checkpoint_due(now)? {
                persist_current(&self.journal, authority, now)?;
            }
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

    /// Make the current ledger durable, as the authority stops (#2664): the
    /// remaining lease bounds then count the uptime already served, so a
    /// restart only lengthens them by its downtime.
    pub fn checkpoint<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        now: u64,
    ) -> Result<(), AuthorityError> {
        persist_current(&self.journal, authority, now)
    }
}
