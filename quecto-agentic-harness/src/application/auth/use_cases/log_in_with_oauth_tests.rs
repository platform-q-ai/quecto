use std::sync::{Arc, Mutex};

use super::*;
use crate::application::auth::ports::{
    AuthorizationCallbackError, BrowserAuthorizationError, CredentialStoreError, IssuedToken,
    PendingAuthorization, TokenExchangeError,
};
use crate::domain::auth::credential::{AuthMethod, Credential};
use crate::domain::auth::pasted_code::PasteRefusal;

const NOW: i64 = 1_000_000;

struct Fakes {
    begun: Mutex<Vec<LoginProvider>>,
    received: Mutex<Vec<(LoginProvider, String)>>,
    exchanged: Mutex<Vec<(LoginProvider, String, String)>>,
    stored: Mutex<Vec<Credential>>,
    returns: AuthorizationReturn,
    unsupported: bool,
    callback: Result<String, AuthorizationCallbackError>,
    exchange: Result<IssuedToken, TokenExchangeError>,
    store_refusal: Option<String>,
}

impl Fakes {
    fn loopback() -> Self {
        Self {
            begun: Mutex::new(Vec::new()),
            received: Mutex::new(Vec::new()),
            exchanged: Mutex::new(Vec::new()),
            stored: Mutex::new(Vec::new()),
            returns: AuthorizationReturn::Loopback {
                waiting_on: "http://localhost:1455".to_string(),
            },
            unsupported: false,
            callback: Ok("fake-callback-code".to_string()),
            exchange: Ok(issued(Some("acct-fake"))),
            store_refusal: None,
        }
    }

    fn pasted() -> Self {
        Self {
            returns: AuthorizationReturn::PastedCode,
            ..Self::loopback()
        }
    }
}

fn issued(account_id: Option<&str>) -> IssuedToken {
    IssuedToken {
        access_token: "fake-access".to_string(),
        refresh_token: Some("fake-refresh".to_string()),
        expires_in: 3_600,
        account_id: account_id.map(str::to_string),
    }
}

impl BrowserAuthorization for Fakes {
    fn begin(
        &self,
        provider: LoginProvider,
    ) -> Result<PendingAuthorization, BrowserAuthorizationError> {
        self.begun.lock().unwrap().push(provider);
        if self.unsupported {
            return Err(BrowserAuthorizationError::Unsupported);
        }
        Ok(PendingAuthorization {
            authorization_url: format!("https://auth.example.test/{}", provider.id()),
            state: "fake-state".to_string(),
            verifier: "fake-verifier".to_string(),
            returns: self.returns.clone(),
        })
    }
}

impl AuthorizationCallback for Fakes {
    fn receive(
        &self,
        provider: LoginProvider,
        state: &str,
    ) -> Result<String, AuthorizationCallbackError> {
        self.received
            .lock()
            .unwrap()
            .push((provider, state.to_string()));
        self.callback.clone()
    }
}

impl TokenExchange for Fakes {
    fn exchange_code(
        &self,
        provider: LoginProvider,
        code: &str,
        verifier: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        self.exchanged
            .lock()
            .unwrap()
            .push((provider, code.to_string(), verifier.to_string()));
        self.exchange.clone()
    }

    fn refresh(
        &self,
        _provider: LoginProvider,
        _refresh_token: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        unreachable!("a browser login refreshes nothing")
    }
}

impl CredentialStorePort for Fakes {
    fn store(&self, credential: Credential) -> Result<(), CredentialStoreError> {
        if let Some(reason) = &self.store_refusal {
            return Err(CredentialStoreError(reason.clone()));
        }
        self.stored.lock().unwrap().push(credential);
        Ok(())
    }

    fn list(&self) -> Result<Vec<Credential>, CredentialStoreError> {
        unreachable!("a browser login lists nothing")
    }

    fn remove(&self, _provider: &str) -> Result<bool, CredentialStoreError> {
        unreachable!("a browser login removes nothing")
    }
}

impl AuthClock for Fakes {
    fn now_unix_secs(&self) -> i64 {
        NOW
    }
}

fn login(fakes: &Arc<Fakes>) -> LogInWithOAuth {
    LogInWithOAuth::new(
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
    )
}

#[test]
fn a_login_begins_at_the_providers_authorization_url() {
    let fakes = Arc::new(Fakes::loopback());
    let attempt = login(&fakes).begin("xai").unwrap();
    assert_eq!(attempt.provider(), LoginProvider::Xai);
    assert_eq!(attempt.authorization_url(), "https://auth.example.test/xai");
    assert_eq!(
        attempt.returns(),
        &AuthorizationReturn::Loopback {
            waiting_on: "http://localhost:1455".to_string()
        }
    );
    assert_eq!(*fakes.begun.lock().unwrap(), [LoginProvider::Xai]);
}

#[test]
fn an_unknown_provider_begins_nothing() {
    let fakes = Arc::new(Fakes::loopback());
    assert_eq!(
        login(&fakes).begin("bogus").unwrap_err(),
        LoginError::UnknownProvider("bogus".to_string())
    );
    assert!(fakes.begun.lock().unwrap().is_empty());
}

#[test]
fn a_callback_login_stores_an_oauth_credential_with_the_expiry_margin() {
    let fakes = Arc::new(Fakes::loopback());
    let login = login(&fakes);
    let attempt = login.begin("openai").unwrap();
    let code = login.await_callback(&attempt).unwrap();
    let outcome = login.complete(attempt, code).unwrap();
    assert_eq!(
        outcome,
        OAuthLogin {
            provider: LoginProvider::OpenAi,
            account_id_missing: false,
        }
    );
    assert_eq!(
        *fakes.received.lock().unwrap(),
        [(LoginProvider::OpenAi, "fake-state".to_string())]
    );
    assert_eq!(
        *fakes.exchanged.lock().unwrap(),
        [(
            LoginProvider::OpenAi,
            "fake-callback-code".to_string(),
            "fake-verifier".to_string()
        )]
    );
    let stored = fakes.stored.lock().unwrap();
    assert_eq!(stored.len(), 1);
    let credential = &stored[0];
    assert_eq!(credential.provider, "openai");
    assert_eq!(credential.token, "fake-access");
    assert_eq!(credential.method, AuthMethod::OAuth);
    assert_eq!(credential.expires_at, Some(NOW + 3_600 - 300));
    assert_eq!(credential.refresh_token.as_deref(), Some("fake-refresh"));
    assert_eq!(credential.account_id.as_deref(), Some("acct-fake"));
}

#[test]
fn an_openai_token_without_an_account_is_stored_and_flagged() {
    let fakes = Arc::new(Fakes {
        exchange: Ok(issued(None)),
        ..Fakes::loopback()
    });
    let login = login(&fakes);
    let attempt = login.begin("openai").unwrap();
    let code = login.accept_pasted(&attempt, "bare-code").unwrap();
    assert!(login.complete(attempt, code).unwrap().account_id_missing);
    assert_eq!(fakes.stored.lock().unwrap()[0].account_id, None);
}

#[test]
fn only_openai_expects_an_account() {
    for provider in ["anthropic", "xai"] {
        let fakes = Arc::new(Fakes {
            exchange: Ok(issued(None)),
            ..Fakes::pasted()
        });
        let login = login(&fakes);
        let attempt = login.begin(provider).unwrap();
        let code = login.accept_pasted(&attempt, "code#state").unwrap();
        assert!(!login.complete(attempt, code).unwrap().account_id_missing);
    }
}

#[test]
fn a_failed_callback_lets_the_user_paste_instead() {
    let fakes = Arc::new(Fakes {
        callback: Err(AuthorizationCallbackError::Failed(
            "provider error: timed out".to_string(),
        )),
        ..Fakes::loopback()
    });
    let login = login(&fakes);
    let attempt = login.begin("xai").unwrap();
    assert_eq!(
        login.await_callback(&attempt).unwrap_err(),
        CallbackFailure::PasteInstead("provider error: timed out".to_string())
    );
}

#[test]
fn an_unusable_redirect_or_runtime_ends_the_login() {
    for (error, refusal) in [
        (
            AuthorizationCallbackError::InvalidRedirect("bad".to_string()),
            LoginError::InvalidRedirect("bad".to_string()),
        ),
        (
            AuthorizationCallbackError::Unavailable("EAGAIN".to_string()),
            LoginError::Unavailable("EAGAIN".to_string()),
        ),
    ] {
        let fakes = Arc::new(Fakes {
            callback: Err(error),
            ..Fakes::loopback()
        });
        let login = login(&fakes);
        let attempt = login.begin("xai").unwrap();
        assert_eq!(
            login.await_callback(&attempt).unwrap_err(),
            CallbackFailure::Refused(refusal)
        );
    }
}

#[test]
fn a_pasted_redirect_must_carry_the_logins_state() {
    let fakes = Arc::new(Fakes::loopback());
    let login = login(&fakes);
    let attempt = login.begin("openai").unwrap();
    assert_eq!(
        login
            .accept_pasted(&attempt, "http://localhost/cb?code=C&state=other")
            .unwrap_err(),
        LoginError::Paste(PasteRefusal::StateMismatch)
    );
    assert_eq!(
        login
            .accept_pasted(&attempt, "http://localhost/cb?code=C")
            .unwrap_err(),
        LoginError::Paste(PasteRefusal::StateMissing)
    );
    assert_eq!(
        login.accept_pasted(&attempt, "  ").unwrap_err(),
        LoginError::Paste(PasteRefusal::NoCode)
    );
    let code = login
        .accept_pasted(&attempt, "http://localhost/cb?code=C&state=fake-state")
        .unwrap();
    login.complete(attempt, code).unwrap();
    assert_eq!(fakes.exchanged.lock().unwrap()[0].1, "C");
}

#[test]
fn a_hosted_page_code_is_passed_on_whole_but_never_empty() {
    let fakes = Arc::new(Fakes::pasted());
    let login = login(&fakes);
    let attempt = login.begin("anthropic").unwrap();
    assert_eq!(
        login.accept_pasted(&attempt, " \n").unwrap_err(),
        LoginError::EmptyAuthorizationCode
    );
    let code = login.accept_pasted(&attempt, " code#state\n").unwrap();
    login.complete(attempt, code).unwrap();
    // Anthropic's code carries its state after `#`; the exchange splits it.
    assert_eq!(fakes.exchanged.lock().unwrap()[0].1, "code#state");
}

#[test]
fn a_refused_exchange_stores_nothing() {
    for (error, refusal) in [
        (
            TokenExchangeError::Rejected("(401)".to_string()),
            LoginError::TokenExchange("(401)".to_string()),
        ),
        (
            TokenExchangeError::Unavailable("EAGAIN".to_string()),
            LoginError::Unavailable("EAGAIN".to_string()),
        ),
    ] {
        let fakes = Arc::new(Fakes {
            exchange: Err(error),
            ..Fakes::loopback()
        });
        let login = login(&fakes);
        let attempt = login.begin("openai").unwrap();
        let code = login.await_callback(&attempt).unwrap();
        assert_eq!(login.complete(attempt, code).unwrap_err(), refusal);
        assert!(fakes.stored.lock().unwrap().is_empty());
    }
}

#[test]
fn a_store_failure_is_reported_in_the_stores_words() {
    let fakes = Arc::new(Fakes {
        store_refusal: Some("config error: failed to parse credentials".to_string()),
        ..Fakes::loopback()
    });
    let login = login(&fakes);
    let attempt = login.begin("openai").unwrap();
    let code = login.await_callback(&attempt).unwrap();
    assert_eq!(
        login.complete(attempt, code).unwrap_err(),
        LoginError::Store("config error: failed to parse credentials".to_string())
    );
}

#[test]
fn debug_never_shows_the_verifier_or_the_code() {
    let fakes = Arc::new(Fakes::loopback());
    let login = login(&fakes);
    let attempt = login.begin("openai").unwrap();
    let code = login.await_callback(&attempt).unwrap();
    let shown = format!("{attempt:?} {code:?}");
    assert!(!shown.contains("fake-verifier"), "{shown}");
    assert!(!shown.contains("fake-callback-code"), "{shown}");
}

#[test]
fn a_provider_without_an_oauth_configuration_is_unsupported() {
    let fakes = Arc::new(Fakes {
        unsupported: true,
        ..Fakes::loopback()
    });
    assert_eq!(
        login(&fakes).begin("anthropic").unwrap_err(),
        LoginError::OAuthUnsupported(LoginProvider::Anthropic)
    );
}

#[test]
fn a_login_started_inside_a_runtime_is_refused_as_such() {
    let fakes = Arc::new(Fakes {
        callback: Err(AuthorizationCallbackError::InsideAsyncRuntime),
        exchange: Err(TokenExchangeError::InsideAsyncRuntime),
        ..Fakes::loopback()
    });
    let login = login(&fakes);
    let attempt = login.begin("openai").unwrap();
    assert_eq!(
        login.await_callback(&attempt).unwrap_err(),
        CallbackFailure::Refused(LoginError::InsideAsyncRuntime)
    );
    let code = login.accept_pasted(&attempt, "bare-code").unwrap();
    assert_eq!(
        login.complete(attempt, code).unwrap_err(),
        LoginError::InsideAsyncRuntime
    );
}
