//! Start the running authority from its journal (#1910): read the ledger,
//! refuse a ledger that went missing after prior operation unless the
//! operator accepted that, restore or start the authority, and make its
//! initial ledger durable before it accepts work.

use super::durable_ledger::persist_current;
use crate::application::admission::dto::broker::{AuthorityStart, RestoreError};
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;

/// The epoch a fresh authority starts in.
const FIRST_EPOCH: u64 = 1;

pub struct RestoreAuthority {
    journal: SharedJournal,
}

impl RestoreAuthority {
    pub fn new(journal: SharedJournal) -> Self {
        Self { journal }
    }

    /// The authority `start` describes, restored from the journal's ledger
    /// or fresh when there is none and none is owed. A ledger exists from the
    /// first start onward, so "missing" is never ambiguous again.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        start: AuthorityStart,
        secrets: S,
        now: u64,
    ) -> Result<AdmissionAuthority<S>, RestoreError> {
        let ledger = self
            .journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .load()
            .map_err(|error| RestoreError::Unreadable(format!("{error:?}")))?;
        let mut authority = match ledger {
            Some(ledger) => AdmissionAuthority::restore(start.policy, &ledger, secrets, now),
            None if start.previously_operated && !start.accept_missing_ledger => {
                return Err(RestoreError::MissingLedger);
            }
            None => AdmissionAuthority::new(FIRST_EPOCH, start.policy, secrets),
        }
        .map_err(RestoreError::Incompatible)?;
        persist_current(&self.journal, &mut authority, now).map_err(RestoreError::NotDurable)?;
        Ok(authority)
    }
}
