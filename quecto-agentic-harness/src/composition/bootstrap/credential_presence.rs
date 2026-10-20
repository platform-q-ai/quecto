//! Stored credential presence for `quecto status` (#1843): the
//! configuration capability's `StoredCredentialPresence`, answered by the
//! auth capability's `auth status` use case (#1842), so one authority says
//! what is stored and whether it is usable.

use std::sync::Arc;

use crate::application::auth::dto::AuthStatusError;
use crate::application::auth::ports::CredentialStoreFailure;
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
            .map_err(|AuthStatusError::Unreadable { failure, .. }| unavailable(failure))
    }
}

/// The store's failure, as the configuration capability names it. Only the
/// kind, typed where the store failed, crosses: the store's words may
/// describe a hand-edited file (#2617), and are never parsed (#2619 review).
fn unavailable(failure: CredentialStoreFailure) -> StoredCredentialsUnavailable {
    match failure {
        CredentialStoreFailure::Malformed => StoredCredentialsUnavailable::Malformed,
        CredentialStoreFailure::PermissionDenied => StoredCredentialsUnavailable::PermissionDenied,
        CredentialStoreFailure::Unavailable => StoredCredentialsUnavailable::Unreadable,
    }
}

#[cfg(test)]
#[path = "credential_presence_tests.rs"]
mod tests;
