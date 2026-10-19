use std::sync::{Arc, Mutex};

use super::*;
use crate::application::auth::ports::{CredentialStoreError, IssuedToken};
use crate::domain::auth::provider::LoginProvider;

const NOW: i64 = 1_000_000;

struct Fakes {
    external: Result<Vec<ExternalCredential>, ExternalCredentialsError>,
    refreshed: Mutex<Vec<(LoginProvider, String)>>,
    refresh: Result<IssuedToken, TokenExchangeError>,
    stored: Mutex<Vec<Credential>>,
    refuse_store_for: Option<&'static str>,
}

impl Fakes {
    fn holding(external: Vec<ExternalCredential>) -> Self {
        Self {
            external: Ok(external),
            refreshed: Mutex::new(Vec::new()),
            refresh: Ok(issued(Some("fake-new-refresh"))),
            stored: Mutex::new(Vec::new()),
            refuse_store_for: None,
        }
    }
}

fn issued(refresh_token: Option<&str>) -> IssuedToken {
    IssuedToken {
        access_token: "fake-refreshed-access".to_string(),
        refresh_token: refresh_token.map(str::to_string),
        expires_in: 3_600,
        account_id: Some("acct-refreshed".to_string()),
    }
}

fn external(
    provider: LoginProvider,
    access: &str,
    refresh: &str,
    expires_at: i64,
) -> ExternalCredential {
    ExternalCredential {
        provider,
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        expires_at,
        account_id: (provider == LoginProvider::OpenAi).then(|| "acct-stated".to_string()),
    }
}

impl ExternalCredentialSource for Fakes {
    fn load(&self) -> Result<Vec<ExternalCredential>, ExternalCredentialsError> {
        self.external.clone()
    }
}

impl TokenExchange for Fakes {
    fn exchange_code(
        &self,
        _: LoginProvider,
        _: &str,
        _: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        unreachable!("an import exchanges no authorization code")
    }

    fn refresh(
        &self,
        provider: LoginProvider,
        refresh_token: &str,
    ) -> Result<IssuedToken, TokenExchangeError> {
        self.refreshed
            .lock()
            .unwrap()
            .push((provider, refresh_token.to_string()));
        self.refresh.clone()
    }
}

impl CredentialStorePort for Fakes {
    fn store(&self, credential: Credential) -> Result<(), CredentialStoreError> {
        if self.refuse_store_for == Some(credential.provider.as_str()) {
            return Err(CredentialStoreError("config error: read-only".to_string()));
        }
        self.stored.lock().unwrap().push(credential);
        Ok(())
    }

    fn list(&self) -> Result<Vec<Credential>, CredentialStoreError> {
        unreachable!("an import lists nothing")
    }

    fn remove(&self, _provider: &str) -> Result<bool, CredentialStoreError> {
        unreachable!("an import removes nothing")
    }
}

impl AuthClock for Fakes {
    fn now_unix_secs(&self) -> i64 {
        NOW
    }
}

fn import(fakes: &Arc<Fakes>) -> ImportExternalCredentials {
    ImportExternalCredentials::new(
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
        Arc::clone(fakes) as _,
    )
}

#[test]
fn an_unexpired_credential_is_stored_as_stated_with_the_margin() {
    let fakes = Arc::new(Fakes::holding(vec![external(
        LoginProvider::OpenAi,
        "fake-access",
        "fake-refresh",
        NOW + 7_200,
    )]));
    let report = import(&fakes).execute().unwrap();
    assert_eq!(
        report,
        ImportReport {
            entries: vec![ImportedEntry {
                provider: LoginProvider::OpenAi,
                refreshed: false,
                stored: Ok(()),
            }],
            aborted: None,
        }
    );
    assert_eq!(report.imported(), 1);
    assert!(fakes.refreshed.lock().unwrap().is_empty());
    let stored = fakes.stored.lock().unwrap();
    let credential = &stored[0];
    assert_eq!(credential.provider, "openai");
    assert_eq!(credential.token, "fake-access");
    assert_eq!(credential.method, AuthMethod::OAuth);
    assert_eq!(credential.expires_at, Some(NOW + 7_200 - 300));
    assert_eq!(credential.refresh_token.as_deref(), Some("fake-refresh"));
    assert_eq!(credential.account_id.as_deref(), Some("acct-stated"));
}

#[test]
fn an_expired_credential_is_refreshed_before_it_is_stored() {
    let fakes = Arc::new(Fakes::holding(vec![external(
        LoginProvider::Anthropic,
        "fake-old",
        "fake-refresh",
        NOW,
    )]));
    let report = import(&fakes).execute().unwrap();
    assert!(report.entries[0].refreshed);
    assert_eq!(
        *fakes.refreshed.lock().unwrap(),
        [(LoginProvider::Anthropic, "fake-refresh".to_string())]
    );
    let stored = &fakes.stored.lock().unwrap()[0];
    assert_eq!(stored.token, "fake-refreshed-access");
    assert_eq!(stored.expires_at, Some(NOW + 3_600 - 300));
    assert_eq!(stored.refresh_token.as_deref(), Some("fake-new-refresh"));
    assert_eq!(stored.account_id.as_deref(), Some("acct-refreshed"));
}

#[test]
fn a_refresh_that_issues_no_refresh_token_keeps_the_old_one() {
    let fakes = Arc::new(Fakes {
        refresh: Ok(issued(None)),
        ..Fakes::holding(vec![external(
            LoginProvider::OpenAi,
            "fake-old",
            "fake-refresh",
            0,
        )])
    });
    import(&fakes).execute().unwrap();
    assert_eq!(
        fakes.stored.lock().unwrap()[0].refresh_token.as_deref(),
        Some("fake-refresh")
    );
}

#[test]
fn an_expired_credential_without_a_refresh_token_is_stored_as_stated() {
    let fakes = Arc::new(Fakes::holding(vec![external(
        LoginProvider::Anthropic,
        "fake-old",
        "",
        NOW - 100,
    )]));
    let report = import(&fakes).execute().unwrap();
    assert!(!report.entries[0].refreshed);
    assert!(fakes.refreshed.lock().unwrap().is_empty());
    let stored = &fakes.stored.lock().unwrap()[0];
    assert_eq!(stored.token, "fake-old");
    assert_eq!(stored.refresh_token, None, "an empty refresh token is none");
}

#[test]
fn an_entry_without_an_access_token_is_skipped() {
    for provider in [LoginProvider::Anthropic, LoginProvider::OpenAi] {
        let fakes = Arc::new(Fakes::holding(vec![external(
            provider,
            "",
            "fake-refresh",
            0,
        )]));
        let report = import(&fakes).execute().unwrap();
        assert!(report.entries.is_empty(), "{provider:?}");
        assert_eq!(report.imported(), 0);
        assert!(fakes.refreshed.lock().unwrap().is_empty());
    }
}

#[test]
fn a_failed_refresh_ends_the_import_keeping_what_was_stored() {
    let fakes = Arc::new(Fakes {
        refresh: Err(TokenExchangeError::Rejected("(400)".to_string())),
        ..Fakes::holding(vec![
            external(LoginProvider::Anthropic, "fake-a", "fake-ra", NOW + 10),
            external(LoginProvider::OpenAi, "fake-o", "fake-ro", NOW - 1),
        ])
    });
    let report = import(&fakes).execute().unwrap();
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].provider, LoginProvider::Anthropic);
    assert_eq!(
        report.aborted,
        Some((
            LoginProvider::OpenAi,
            RefreshFailure::Rejected("(400)".to_string())
        ))
    );
    assert_eq!(fakes.stored.lock().unwrap().len(), 1);
}

#[test]
fn a_refresh_that_cannot_run_ends_the_import_saying_why() {
    for (error, failure) in [
        (
            TokenExchangeError::Unavailable("EAGAIN".to_string()),
            RefreshFailure::Unavailable("EAGAIN".to_string()),
        ),
        (
            TokenExchangeError::InsideAsyncRuntime,
            RefreshFailure::InsideAsyncRuntime,
        ),
    ] {
        let fakes = Arc::new(Fakes {
            refresh: Err(error),
            ..Fakes::holding(vec![external(
                LoginProvider::OpenAi,
                "fake-o",
                "fake-ro",
                0,
            )])
        });
        assert_eq!(
            import(&fakes).execute().unwrap().aborted,
            Some((LoginProvider::OpenAi, failure))
        );
    }
}

#[test]
fn a_failed_store_is_reported_and_the_import_goes_on() {
    let fakes = Arc::new(Fakes {
        refuse_store_for: Some("anthropic"),
        ..Fakes::holding(vec![
            external(LoginProvider::Anthropic, "fake-a", "", NOW + 10),
            external(LoginProvider::OpenAi, "fake-o", "", NOW + 10),
        ])
    });
    let report = import(&fakes).execute().unwrap();
    assert_eq!(
        report.entries[0].stored,
        Err("config error: read-only".to_string())
    );
    assert_eq!(report.entries[1].stored, Ok(()));
    assert_eq!(report.imported(), 1);
}

#[test]
fn an_unusable_external_store_imports_nothing() {
    for (error, refusal) in [
        (
            ExternalCredentialsError::NotFound {
                path: "/x/auth.json".to_string(),
            },
            ImportError::NotFound {
                path: "/x/auth.json".to_string(),
            },
        ),
        (
            ExternalCredentialsError::Unreadable {
                path: "/x".to_string(),
                reason: "denied".to_string(),
            },
            ImportError::Unreadable {
                path: "/x".to_string(),
                reason: "denied".to_string(),
            },
        ),
        (
            ExternalCredentialsError::Malformed("eof".to_string()),
            ImportError::Malformed("eof".to_string()),
        ),
    ] {
        let fakes = Arc::new(Fakes {
            external: Err(error),
            ..Fakes::holding(Vec::new())
        });
        assert_eq!(import(&fakes).execute(), Err(refusal));
        assert!(fakes.stored.lock().unwrap().is_empty());
    }
}

#[test]
fn nothing_external_is_an_empty_report() {
    let fakes = Arc::new(Fakes::holding(Vec::new()));
    let report = import(&fakes).execute().unwrap();
    assert_eq!(report.imported(), 0);
    assert_eq!(report.aborted, None);
}
