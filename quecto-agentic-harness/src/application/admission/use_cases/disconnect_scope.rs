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
    /// When the scope holds no active attempt at all (newly or already
    /// unverified) it is released as well, so an abrupt exit never holds a
    /// scope slot until a reset, while a scope that reconnected and dropped
    /// again keeps its capability for the work it still owes.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        scope: ScopeId,
        now: u64,
    ) -> Result<AbandonReport, AuthorityError> {
        let report = authority.disconnect(scope, now)?;
        debug_assert!(
            report.uncertain <= report.active,
            "newly unverified attempts are active ones"
        );
        if report.active == 0 {
            // A known scope with nothing active always retires; a failure
            // here is an invariant breach, reported rather than ignored.
            authority.release(scope)?;
        }
        Ok(report)
    }
}
