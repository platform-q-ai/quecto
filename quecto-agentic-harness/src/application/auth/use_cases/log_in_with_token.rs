//! Log in with an API token (#1839): `auth login --provider P --token T`
//! stores `T` as `P`'s token credential, which never expires.

use std::sync::Arc;

use crate::application::auth::dto::LoginError;
use crate::application::auth::ports::CredentialStorePort;
use crate::domain::auth::credential::{AuthMethod, Credential};
use crate::domain::auth::provider::LoginProvider;

pub struct LogInWithToken {
    store: Arc<dyn CredentialStorePort>,
}

impl LogInWithToken {
    pub fn new(store: Arc<dyn CredentialStorePort>) -> Self {
        Self { store }
    }

    /// Store `token`, trimmed, as `provider`'s credential. Refuses, storing
    /// nothing, a provider outside the login allowlist (checked first) or a
    /// token that is empty once trimmed.
    pub fn execute(&self, provider: &str, token: &str) -> Result<LoginProvider, LoginError> {
        let Some(login) = LoginProvider::parse(provider) else {
            return Err(LoginError::UnknownProvider(provider.to_string()));
        };
        let token = token.trim();
        if token.is_empty() {
            return Err(LoginError::EmptyToken);
        }
        let credential = Credential {
            provider: login.id().to_string(),
            token: token.to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        };
        debug_assert!(
            !credential.token.is_empty() && credential.provider == login.id(),
            "a stored token is non-empty and keyed by its login provider"
        );
        self.store
            .store(credential)
            .map_err(|error| LoginError::Store(error.0))?;
        Ok(login)
    }
}

impl std::fmt::Debug for LogInWithToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogInWithToken").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "log_in_with_token_tests.rs"]
mod tests;
