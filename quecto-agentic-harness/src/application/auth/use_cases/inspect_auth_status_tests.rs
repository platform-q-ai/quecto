use std::sync::{Arc, Mutex};

use super::*;
use crate::application::auth::ports::{AuthClock, CredentialStoreError, CredentialStorePort};
use crate::domain::auth::credential::{AuthMethod, Credential, CredentialState};

const NOW: i64 = 5_000;

struct Fakes {
    stored: Result<Vec<Credential>, CredentialStoreError>,
    writes: Mutex<usize>,
}

impl CredentialStorePort for Fakes {
    fn store(&self, _credential: Credential) -> Result<(), CredentialStoreError> {
        *self.writes.lock().unwrap() += 1;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Credential>, CredentialStoreError> {
        self.stored.clone()
    }
}

impl AuthClock for Fakes {
    fn now_unix_secs(&self) -> i64 {
        NOW
    }
}

fn credential(provider: &str, method: AuthMethod, expires_at: Option<i64>) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: format!("fake-{provider}"),
        method,
        expires_at,
        refresh_token: None,
        account_id: None,
    }
}

fn inspect(
    stored: Result<Vec<Credential>, CredentialStoreError>,
) -> (Arc<Fakes>, InspectAuthStatus) {
    let fakes = Arc::new(Fakes {
        stored,
        writes: Mutex::new(0),
    });
    let use_case = InspectAuthStatus::new(Arc::clone(&fakes) as _, Arc::clone(&fakes) as _);
    (fakes, use_case)
}

#[test]
fn no_stored_credential_is_an_empty_status() {
    let (_, use_case) = inspect(Ok(Vec::new()));
    assert_eq!(use_case.execute(), Ok(Vec::new()));
}

#[test]
fn each_credential_reports_its_provider_method_and_state_now() {
    let (fakes, use_case) = inspect(Ok(vec![
        credential("xai", AuthMethod::OAuth, Some(NOW)),
        credential("anthropic", AuthMethod::Token, None),
        credential("openai", AuthMethod::OAuth, Some(NOW + 1)),
    ]));
    assert_eq!(
        use_case.execute(),
        Ok(vec![
            ProviderAuthStatus {
                provider: "anthropic".to_string(),
                method: AuthMethod::Token,
                state: CredentialState::Active,
            },
            ProviderAuthStatus {
                provider: "openai".to_string(),
                method: AuthMethod::OAuth,
                state: CredentialState::Active,
            },
            ProviderAuthStatus {
                provider: "xai".to_string(),
                method: AuthMethod::OAuth,
                state: CredentialState::Expired,
            },
        ])
    );
    assert_eq!(
        *fakes.writes.lock().unwrap(),
        0,
        "inspecting writes nothing"
    );
}

#[test]
fn a_credential_for_any_provider_name_is_reported() {
    // The store is the authority on what is stored: a provider outside the
    // login allowlist (an older or hand-written entry) is still shown.
    let (_, use_case) = inspect(Ok(vec![credential("groq", AuthMethod::Token, None)]));
    assert_eq!(use_case.execute().unwrap()[0].provider, "groq");
}

#[test]
fn an_unreadable_store_is_reported_in_its_words() {
    let (_, use_case) = inspect(Err(CredentialStoreError(
        "config error: failed to parse credentials: oops".to_string(),
    )));
    assert_eq!(
        use_case.execute(),
        Err(AuthStatusError::Unreadable(
            "config error: failed to parse credentials: oops".to_string()
        ))
    );
}

#[test]
fn nothing_secret_reaches_the_status() {
    let (_, use_case) = inspect(Ok(vec![credential("openai", AuthMethod::OAuth, None)]));
    let shown = format!("{:?}", use_case.execute());
    assert!(!shown.contains("fake-openai"), "{shown}");
}
