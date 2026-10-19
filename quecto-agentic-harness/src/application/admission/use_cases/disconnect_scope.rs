//! A client's connection to the running authority ended (#1910).

use crate::application::admission::dto::broker::AuthorityError;
use crate::application::admission::ports::inference_authority_ports::AdmissionSecretSource;
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::{AbandonReport, ScopeId};

#[derive(Debug, Default)]
pub struct DisconnectScope;

impl DisconnectScope {
    pub fn new() -> Self {
        Self
    }

    /// The scope's queued work is cancelled and its active work stays
    /// unverified until the same capability reconnects and completes it.
    /// When nothing unverified remains the scope is released as well, so an
    /// abrupt exit never holds a scope slot until a reset.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        scope: ScopeId,
        now: u64,
    ) -> Result<AbandonReport, AuthorityError> {
        let report = authority.disconnect(scope, now)?;
        if report.uncertain == 0 {
            authority.release(scope)?;
        }
        Ok(report)
    }
}
