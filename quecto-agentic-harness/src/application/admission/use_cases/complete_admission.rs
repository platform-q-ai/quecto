//! Complete an attempt: its release is durable before it is acknowledged
//! (#1679 P3, #1910).

use super::durable_ledger::persist_current;
use crate::application::admission::dto::broker::{AuthorityError, Credential};
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::{Feedback, RequestState};

pub struct CompleteAdmission {
    journal: SharedJournal,
}

impl CompleteAdmission {
    pub fn new(journal: SharedJournal) -> Self {
        Self { journal }
    }

    /// The release is durable before it is acknowledged; a failed write leaves
    /// the in-memory release but reports the failure so the client retries.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        credential: &Credential,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        let state = authority.complete_unpersisted(credential, sequence, feedback, now)?;
        persist_current(&self.journal, authority, now)?;
        Ok(state)
    }
}
