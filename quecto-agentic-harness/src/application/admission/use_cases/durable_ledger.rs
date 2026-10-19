//! The one write every durability use case shares (#1910): the authority's
//! current (or given) ledger made durable through the journal, with the
//! journal's outcome recorded on the authority.

use crate::application::admission::dto::broker::AuthorityError;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::AdmissionLedger;

/// Make `ledger` durable and record the journal's health on `authority`.
pub(super) fn persist<S: AdmissionSecretSource>(
    journal: &SharedJournal,
    authority: &mut AdmissionAuthority<S>,
    ledger: &AdmissionLedger,
) -> Result<(), AuthorityError> {
    // A poisoned lock still holds the journal: a panic elsewhere must not
    // turn every later write into a panic.
    let written = journal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .persist(ledger);
    authority.record_journal(written.is_ok());
    written.map_err(|_| AuthorityError::JournalUnavailable)
}

/// Make the authority's current ledger durable.
pub(super) fn persist_current<S: AdmissionSecretSource>(
    journal: &SharedJournal,
    authority: &mut AdmissionAuthority<S>,
    now: u64,
) -> Result<(), AuthorityError> {
    let ledger = authority.ledger(now)?;
    persist(journal, authority, &ledger)
}
