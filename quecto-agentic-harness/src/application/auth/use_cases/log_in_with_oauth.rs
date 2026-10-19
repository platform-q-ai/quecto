//! Log in through a browser (#1839): `auth login` for OpenAI, Anthropic
//! and xAI. The interface drives the dialog — show the URL, wait for the
//! callback or read a paste — through these steps; the use case owns what
//! is trusted and what is stored.

use std::sync::Arc;

use crate::application::auth::dto::{
    AuthorizationCode, CallbackFailure, LoginError, OAuthLogin, OAuthLoginAttempt,
};
use crate::application::auth::ports::{
    AuthClock, AuthorizationCallback, AuthorizationCallbackError, AuthorizationReturn,
    BrowserAuthorization, BrowserAuthorizationError, CredentialStorePort, TokenExchange,
    TokenExchangeError,
};
use crate::domain::auth::credential::{AuthMethod, Credential, oauth_expires_at};
use crate::domain::auth::pasted_code::code_from_paste;
use crate::domain::auth::provider::LoginProvider;

pub struct LogInWithOAuth {
    authorization: Arc<dyn BrowserAuthorization>,
    callback: Arc<dyn AuthorizationCallback>,
    exchange: Arc<dyn TokenExchange>,
    store: Arc<dyn CredentialStorePort>,
    clock: Arc<dyn AuthClock>,
}

impl LogInWithOAuth {
    pub fn new(
        authorization: Arc<dyn BrowserAuthorization>,
        callback: Arc<dyn AuthorizationCallback>,
        exchange: Arc<dyn TokenExchange>,
        store: Arc<dyn CredentialStorePort>,
        clock: Arc<dyn AuthClock>,
    ) -> Self {
        Self {
            authorization,
            callback,
            exchange,
            store,
            clock,
        }
    }

    /// Start `provider`'s authorization. A provider outside the login
    /// allowlist is refused without starting one.
    pub fn begin(&self, provider: &str) -> Result<OAuthLoginAttempt, LoginError> {
        let Some(login) = LoginProvider::parse(provider) else {
            return Err(LoginError::UnknownProvider(provider.to_string()));
        };
        let pending =
            self.authorization
                .begin(login)
                .map_err(|BrowserAuthorizationError::Unsupported| {
                    LoginError::OAuthUnsupported(login)
                })?;
        assert!(
            !pending.state.is_empty(),
            "an authorization is bound to a state"
        );
        Ok(OAuthLoginAttempt::new(login, pending))
    }

    /// Wait for the browser callback of an `attempt` that returns to a
    /// local listener.
    pub fn await_callback(
        &self,
        attempt: &OAuthLoginAttempt,
    ) -> Result<AuthorizationCode, CallbackFailure> {
        debug_assert!(
            matches!(attempt.returns(), AuthorizationReturn::Loopback { .. }),
            "only a loopback login has a callback to wait for"
        );
        let pending = attempt.pending();
        match self.callback.receive(attempt.provider(), &pending.state) {
            Ok(code) => Ok(AuthorizationCode::accepted(code)),
            Err(AuthorizationCallbackError::Failed(reason)) => {
                Err(CallbackFailure::PasteInstead(reason))
            }
            Err(AuthorizationCallbackError::InvalidRedirect(reason)) => Err(
                CallbackFailure::Refused(LoginError::InvalidRedirect(reason)),
            ),
            Err(AuthorizationCallbackError::Unavailable(reason)) => {
                Err(CallbackFailure::Refused(LoginError::Unavailable(reason)))
            }
            Err(AuthorizationCallbackError::InsideAsyncRuntime) => {
                Err(CallbackFailure::Refused(LoginError::InsideAsyncRuntime))
            }
        }
    }

    /// Accept a code its user pasted. A login that returns to a listener
    /// takes a redirect URL bound to its state, or a bare code; one that
    /// returns to a provider-hosted page takes that page's code whole.
    pub fn accept_pasted(
        &self,
        attempt: &OAuthLoginAttempt,
        input: &str,
    ) -> Result<AuthorizationCode, LoginError> {
        match attempt.returns() {
            AuthorizationReturn::Loopback { .. } => {
                code_from_paste(input, &attempt.pending().state)
                    .map(AuthorizationCode::accepted)
                    .map_err(LoginError::Paste)
            }
            AuthorizationReturn::PastedCode => match input.trim() {
                "" => Err(LoginError::EmptyAuthorizationCode),
                code => Ok(AuthorizationCode::accepted(code.to_string())),
            },
        }
    }

    /// Exchange the accepted `code` and store the login's credential, which
    /// expires the domain's margin before the token does.
    pub fn complete(
        &self,
        attempt: OAuthLoginAttempt,
        code: AuthorizationCode,
    ) -> Result<OAuthLogin, LoginError> {
        let provider = attempt.provider();
        let issued = self
            .exchange
            .exchange_code(provider, code.as_str(), &attempt.pending().verifier)
            .map_err(|error| match error {
                TokenExchangeError::Unavailable(reason) => LoginError::Unavailable(reason),
                TokenExchangeError::InsideAsyncRuntime => LoginError::InsideAsyncRuntime,
                TokenExchangeError::Rejected(reason) => LoginError::TokenExchange(reason),
            })?;
        let account_id_missing = provider == LoginProvider::OpenAi && issued.account_id.is_none();
        let credential = Credential {
            provider: provider.id().to_string(),
            token: issued.access_token,
            method: AuthMethod::OAuth,
            expires_at: Some(oauth_expires_at(
                self.clock.now_unix_secs(),
                issued.expires_in,
            )),
            refresh_token: issued.refresh_token,
            account_id: issued.account_id,
        };
        self.store
            .store(credential)
            .map_err(|error| LoginError::Store(error.0))?;
        Ok(OAuthLogin {
            provider,
            account_id_missing,
        })
    }
}

impl std::fmt::Debug for LogInWithOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogInWithOAuth").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "log_in_with_oauth_tests.rs"]
mod tests;
