//! A connection binds the capability it presents to the running authority
//! (#1910).

use crate::application::admission::dto::broker::{AuthorityError, Credential};
use crate::application::admission::ports::inference_authority_ports::AdmissionSecretSource;
use crate::application::admission::services::inference_authority::AdmissionAuthority;

#[derive(Debug, Default)]
pub struct BindScope;

impl BindScope {
    pub fn new() -> Self {
        Self
    }

    /// Verify `credential` and answer the next sequence its owner may use.
    /// When the binding `supersedes` another live connection of the same
    /// scope, that connection's queued work can never be answered and its
    /// active work is unverified until this connection completes it, so the
    /// scope is disconnected first; a forged credential disconnects nothing.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        credential: &Credential,
        supersedes: bool,
        now: u64,
    ) -> Result<u64, AuthorityError> {
        let high_water = authority.verify(credential)?;
        if supersedes {
            authority.disconnect(credential.scope, now)?;
        }
        Ok(high_water.saturating_add(1))
    }
}
