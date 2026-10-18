//! Browser OAuth login over the providers' endpoints (#1839): the
//! application's `BrowserAuthorization`, `AuthorizationCallback` and
//! `TokenExchange` for OpenAI, Anthropic and xAI. Each network call runs
//! on the harness's own runtime builder (#1718: a thread-exhausted process
//! gets an error, not an abort), so the CLI's per-command call needs no
//! ambient runtime.

use crate::application::auth::ports::{
    AuthorizationCallback, AuthorizationCallbackError, AuthorizationReturn, BrowserAuthorization,
    BrowserAuthorizationError, IssuedToken, PendingAuthorization, TokenExchange,
    TokenExchangeError,
};
use crate::domain::auth::provider::LoginProvider;
use crate::infrastructure::auth::oauth::{
    OAuthEndpoints, build_anthropic_auth_url, build_openai_auth_url, build_xai_auth_url,
    exchange_anthropic_code, exchange_openai_code, exchange_xai_code, extract_openai_account_id,
    generate_pkce, generate_state, parse_loopback_redirect, wait_for_oauth_callback,
    wait_for_oauth_callback_at,
};
use crate::infrastructure::processes::tokio_runtime::build_tokio_runtime;

/// How long a login waits for its browser callback.
const CALLBACK_TIMEOUT_SECS: u64 = 300;

/// Where OpenAI's fixed callback listener (`127.0.0.1:1455`, the redirect
/// OpenAI has registered) is shown to its user.
const OPENAI_CALLBACK_SHOWN: &str = "http://localhost:1455";

/// How the adapter builds the runtime a call runs on.
type RuntimeBuilder = fn() -> std::io::Result<tokio::runtime::Runtime>;

#[derive(Debug, Clone)]
pub struct OAuthBrowserLogin {
    endpoints: OAuthEndpoints,
    runtime: RuntimeBuilder,
}

impl OAuthBrowserLogin {
    pub fn new(endpoints: OAuthEndpoints) -> Self {
        Self {
            endpoints,
            runtime: build_tokio_runtime,
        }
    }

    /// This login on runtimes from `runtime` instead.
    #[cfg(test)]
    fn on_runtimes(endpoints: OAuthEndpoints, runtime: RuntimeBuilder) -> Self {
        Self { endpoints, runtime }
    }

    /// A runtime to block a call on, or why there is none: the caller is
    /// itself inside one (blocking there panics), or none can be built.
    fn runtime(&self) -> Result<tokio::runtime::Runtime, String> {
        match tokio::runtime::Handle::try_current() {
            Err(_outside_a_runtime) => {}
            Ok(_) => return Err("called from inside an async runtime".to_string()),
        }
        (self.runtime)().map_err(|error| error.to_string())
    }
}

impl BrowserAuthorization for OAuthBrowserLogin {
    fn begin(
        &self,
        provider: LoginProvider,
    ) -> Result<PendingAuthorization, BrowserAuthorizationError> {
        let config = self
            .endpoints
            .config_for(provider)
            .ok_or(BrowserAuthorizationError::Unsupported)?;
        let pkce = generate_pkce();
        let state = generate_state();
        let (authorization_url, returns) = match provider {
            LoginProvider::OpenAi => (
                build_openai_auth_url(&config, &pkce, &state),
                AuthorizationReturn::Loopback {
                    waiting_on: OPENAI_CALLBACK_SHOWN.to_string(),
                },
            ),
            LoginProvider::Xai => (
                build_xai_auth_url(&config, &pkce, &state),
                AuthorizationReturn::Loopback {
                    waiting_on: config.redirect_uri.clone(),
                },
            ),
            // Anthropic redirects to its own console page, which shows the
            // code for the user to paste.
            LoginProvider::Anthropic => (
                build_anthropic_auth_url(&config, &pkce, &state),
                AuthorizationReturn::PastedCode,
            ),
        };
        Ok(PendingAuthorization {
            authorization_url,
            state,
            verifier: pkce.verifier,
            returns,
        })
    }
}

impl AuthorizationCallback for OAuthBrowserLogin {
    fn receive(
        &self,
        provider: LoginProvider,
        state: &str,
    ) -> Result<String, AuthorizationCallbackError> {
        let failed = |error: crate::domain::shared::error::DomainError| {
            AuthorizationCallbackError::Failed(error.to_string())
        };
        match provider {
            LoginProvider::OpenAi => {
                let runtime = self
                    .runtime()
                    .map_err(AuthorizationCallbackError::Unavailable)?;
                runtime
                    .block_on(wait_for_oauth_callback(state, CALLBACK_TIMEOUT_SECS))
                    .map_err(failed)
            }
            LoginProvider::Xai => {
                let runtime = self
                    .runtime()
                    .map_err(AuthorizationCallbackError::Unavailable)?;
                let config = self.endpoints.config_for(provider).ok_or_else(|| {
                    AuthorizationCallbackError::InvalidRedirect(
                        "no OAuth configuration".to_string(),
                    )
                })?;
                // The listener's address and path come from the registered
                // redirect URI so they cannot drift apart (PR #1087 review).
                let (address, path) =
                    parse_loopback_redirect(&config.redirect_uri).map_err(|error| {
                        AuthorizationCallbackError::InvalidRedirect(error.to_string())
                    })?;
                runtime
                    .block_on(wait_for_oauth_callback_at(
                        &address,
                        &path,
                        state,
                        CALLBACK_TIMEOUT_SECS,
                    ))
                    .map_err(failed)
            }
            LoginProvider::Anthropic => Err(AuthorizationCallbackError::Failed(
                "Anthropic has no browser callback".to_string(),
            )),
        }
    }
}

impl TokenExchange for OAuthBrowserLogin {
    fn exchange_code(
        &self,
        provider: LoginProvider,
        code: &str,
        verifier: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        let config = self.endpoints.config_for(provider).ok_or_else(|| {
            TokenExchangeError::Rejected(format!("no OAuth configuration for {}", provider.id()))
        })?;
        let runtime = self.runtime().map_err(TokenExchangeError::Unavailable)?;
        let response = runtime
            .block_on(async {
                match provider {
                    LoginProvider::OpenAi => exchange_openai_code(&config, code, verifier).await,
                    LoginProvider::Anthropic => {
                        exchange_anthropic_code(&config, code, verifier).await
                    }
                    LoginProvider::Xai => exchange_xai_code(&config, code, verifier).await,
                }
            })
            .map_err(|error| TokenExchangeError::Rejected(error.to_string()))?;
        // Only OpenAI's token names the account its requests are routed to.
        let account_id = match provider {
            LoginProvider::OpenAi => extract_openai_account_id(&response.access_token),
            LoginProvider::Anthropic | LoginProvider::Xai => None,
        };
        Ok(IssuedToken {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_in: response.expires_in,
            account_id,
        })
    }
}

#[cfg(test)]
#[path = "browser_login_tests.rs"]
mod tests;
