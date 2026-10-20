//! Browser OAuth login over the providers' endpoints (#1839, #1840): the
//! application's `BrowserAuthorization`, `AuthorizationCallback` and
//! `TokenExchange` (code exchange and refresh) for OpenAI, Anthropic and
//! xAI. Each network call runs
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
    generate_pkce, generate_state, parse_loopback_redirect, refresh_anthropic_token,
    refresh_openai_token, refresh_xai_token, wait_for_oauth_callback_at,
};
use crate::infrastructure::processes::tokio_runtime::build_tokio_runtime;

/// How long a login waits for its browser callback.
const CALLBACK_TIMEOUT_SECS: u64 = 300;

/// How the adapter builds the runtime a call runs on.
type RuntimeBuilder = fn() -> std::io::Result<tokio::runtime::Runtime>;

/// Why no runtime can run a blocking call here.
enum NoRuntime {
    /// The caller is itself inside one, where blocking panics.
    InsideAsyncRuntime,
    /// None could be built, in the builder's words.
    Unavailable(String),
}

impl From<NoRuntime> for AuthorizationCallbackError {
    fn from(why: NoRuntime) -> Self {
        match why {
            NoRuntime::InsideAsyncRuntime => Self::InsideAsyncRuntime,
            NoRuntime::Unavailable(reason) => Self::Unavailable(reason),
        }
    }
}

impl From<NoRuntime> for TokenExchangeError {
    fn from(why: NoRuntime) -> Self {
        match why {
            NoRuntime::InsideAsyncRuntime => Self::InsideAsyncRuntime,
            NoRuntime::Unavailable(reason) => Self::Unavailable(reason),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OAuthBrowserLogin {
    endpoints: OAuthEndpoints,
    runtime: RuntimeBuilder,
    callback_timeout_secs: u64,
}

impl OAuthBrowserLogin {
    pub fn new(endpoints: OAuthEndpoints) -> Self {
        Self::with_callback_timeout_secs(endpoints, CALLBACK_TIMEOUT_SECS)
    }

    /// This login, waiting `secs` (at least one) for a browser callback.
    pub fn with_callback_timeout_secs(endpoints: OAuthEndpoints, secs: u64) -> Self {
        Self {
            endpoints,
            runtime: build_tokio_runtime,
            callback_timeout_secs: secs.max(1),
        }
    }

    /// This login on runtimes from `runtime` instead.
    #[cfg(test)]
    fn on_runtimes(endpoints: OAuthEndpoints, runtime: RuntimeBuilder) -> Self {
        Self {
            runtime,
            ..Self::new(endpoints)
        }
    }

    /// A runtime to block a call on, or why there is none.
    fn runtime(&self) -> Result<tokio::runtime::Runtime, NoRuntime> {
        match tokio::runtime::Handle::try_current() {
            Err(_outside_a_runtime) => {}
            Ok(_) => return Err(NoRuntime::InsideAsyncRuntime),
        }
        (self.runtime)().map_err(|error| NoRuntime::Unavailable(error.to_string()))
    }
}

/// Where the browser callback for `redirect_uri` is shown to its user: the
/// redirect's origin (`http://localhost:1455` for OpenAI's
/// `http://localhost:1455/auth/callback`).
fn shown_origin(redirect_uri: &str) -> String {
    reqwest::Url::parse(redirect_uri)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| redirect_uri.to_string())
}

/// The address and path a listener for `redirect_uri` binds: from the
/// registered redirect itself, so the two cannot drift apart (PR #1087
/// review). `localhost` binds `127.0.0.1`, where the browser's callback
/// to it has always been served.
pub(super) fn listener_address(
    redirect_uri: &str,
) -> Result<(String, String), crate::domain::shared::error::DomainError> {
    let (address, path) = parse_loopback_redirect(redirect_uri)?;
    let address = match address.strip_prefix("localhost:") {
        Some(port) => format!("127.0.0.1:{port}"),
        None => address,
    };
    Ok((address, path))
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
                    waiting_on: shown_origin(&config.redirect_uri),
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
        match provider {
            LoginProvider::OpenAi | LoginProvider::Xai => {
                let runtime = self.runtime()?;
                let config = self.endpoints.config_for(provider).ok_or_else(|| {
                    AuthorizationCallbackError::InvalidRedirect(
                        "no OAuth configuration".to_string(),
                    )
                })?;
                let (address, path) = listener_address(&config.redirect_uri).map_err(|error| {
                    AuthorizationCallbackError::InvalidRedirect(error.to_string())
                })?;
                runtime
                    .block_on(wait_for_oauth_callback_at(
                        &address,
                        &path,
                        state,
                        self.callback_timeout_secs,
                    ))
                    .map_err(|error| AuthorizationCallbackError::Failed(error.to_string()))
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
        let config = self.config(provider)?;
        let runtime = self.runtime()?;
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
        Ok(issued(provider, response))
    }

    fn refresh(
        &self,
        provider: LoginProvider,
        refresh_token: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        let config = self.config(provider)?;
        let runtime = self.runtime()?;
        let response = runtime
            .block_on(async {
                match provider {
                    LoginProvider::OpenAi => refresh_openai_token(&config, refresh_token).await,
                    LoginProvider::Anthropic => {
                        refresh_anthropic_token(&config, refresh_token).await
                    }
                    LoginProvider::Xai => refresh_xai_token(&config, refresh_token).await,
                }
            })
            .map_err(|error| TokenExchangeError::Rejected(error.to_string()))?;
        Ok(issued(provider, response))
    }
}

impl OAuthBrowserLogin {
    fn config(
        &self,
        provider: LoginProvider,
    ) -> Result<crate::infrastructure::auth::oauth::OAuthConfig, TokenExchangeError> {
        self.endpoints.config_for(provider).ok_or_else(|| {
            TokenExchangeError::Rejected(format!("no OAuth configuration for {}", provider.id()))
        })
    }
}

/// The tokens `response` carries, with the account the access token names
/// when its provider names one: only OpenAI's token names the account its
/// requests are routed to.
fn issued(
    provider: LoginProvider,
    response: crate::infrastructure::auth::oauth::OAuthTokenResponse,
) -> IssuedToken {
    let account_id = match provider {
        LoginProvider::OpenAi => extract_openai_account_id(&response.access_token),
        LoginProvider::Anthropic | LoginProvider::Xai => None,
    };
    IssuedToken {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        expires_in: response.expires_in,
        account_id,
    }
}

#[cfg(test)]
#[path = "browser_login_tests.rs"]
mod tests;
