//! The operator reset inside the running authority: a durable successor
//! epoch first, then every capability revoked (#1679, #1910). The admin
//! client's side of the reset is `ResetAuthority`.

use super::durable_ledger::persist;
use crate::application::admission::dto::broker::AuthorityError;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;

pub struct AdvanceAuthorityEpoch {
    journal: SharedJournal,
}

impl AdvanceAuthorityEpoch {
    pub fn new(journal: SharedJournal) -> Self {
        Self { journal }
    }

    /// Operator reset: durable successor epoch, every capability revoked.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        now: u64,
    ) -> Result<u64, AuthorityError> {
        let successor = authority.successor_ledger(now)?;
        persist(&self.journal, authority, &successor)?;
        authority.adopt_successor(successor.epoch, now)
    }
}
