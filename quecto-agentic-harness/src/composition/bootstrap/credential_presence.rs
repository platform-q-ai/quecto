//! Stored credential presence for `quecto status` (#1843): the
//! configuration capability's `StoredCredentialPresence`, answered by the
//! auth capability's `auth status` use case (#1842), so one authority says
//! what is stored and whether it is usable.

use std::sync::Arc;

use crate::application::auth::dto::AuthStatusError;
use crate::application::auth::use_cases::InspectAuthStatus;
use crate::application::configuration::ports::{
    StoredCredential, StoredCredentialPresence, StoredCredentialsUnavailable,
};

pub struct AuthStatusPresence {
    status: Arc<InspectAuthStatus>,
}

impl AuthStatusPresence {
    pub fn new(status: Arc<InspectAuthStatus>) -> Self {
        Self { status }
    }
}

impl StoredCredentialPresence for AuthStatusPresence {
    fn stored(&self) -> Result<Vec<StoredCredential>, StoredCredentialsUnavailable> {
        self.status
            .execute()
            .map(|statuses| {
                statuses
                    .into_iter()
                    .map(|status| StoredCredential {
                        provider: status.provider,
                        method: status.method.as_str().to_string(),
                        state: status.state.as_str().to_string(),
                    })
                    .collect()
            })
            .map_err(|AuthStatusError::Unreadable(reason)| unavailable(&reason))
    }
}

/// The kind of failure the store's `reason` names. Only the kind crosses
/// into the configuration capability: the reason itself may describe a
/// hand-edited file (#2617).
fn unavailable(reason: &str) -> StoredCredentialsUnavailable {
    if reason.contains("failed to parse credentials") {
        StoredCredentialsUnavailable::Malformed
    } else if reason.to_ascii_lowercase().contains("permission denied") {
        StoredCredentialsUnavailable::PermissionDenied
    } else {
        StoredCredentialsUnavailable::Unreadable
    }
}

#[cfg(test)]
#[path = "credential_presence_tests.rs"]
mod tests;
