use std::sync::{Arc, Mutex};

use super::*;
use crate::application::auth::ports::{CredentialStoreError, CredentialStorePort};
use crate::domain::auth::credential::{AuthMethod, Credential};

/// Records every credential it is asked to store; refuses when told to.
#[derive(Default)]
struct FakeStore {
    stored: Mutex<Vec<Credential>>,
    refusal: Option<String>,
}

impl CredentialStorePort for FakeStore {
    fn store(&self, credential: Credential) -> Result<(), CredentialStoreError> {
        if let Some(reason) = &self.refusal {
            return Err(CredentialStoreError(reason.clone()));
        }
        self.stored.lock().unwrap().push(credential);
        Ok(())
    }
}

fn login(store: &Arc<FakeStore>) -> LogInWithToken {
    LogInWithToken::new(Arc::clone(store) as Arc<dyn CredentialStorePort>)
}

#[test]
fn a_token_login_stores_a_token_credential_that_never_expires() {
    let store = Arc::new(FakeStore::default());
    let provider = login(&store).execute("anthropic", "fake-token").unwrap();
    assert_eq!(provider, LoginProvider::Anthropic);
    let stored = store.stored.lock().unwrap();
    assert_eq!(stored.len(), 1);
    let credential = &stored[0];
    assert_eq!(credential.provider, "anthropic");
    assert_eq!(credential.token, "fake-token");
    assert_eq!(credential.method, AuthMethod::Token);
    assert_eq!(credential.expires_at, None);
    assert_eq!(credential.refresh_token, None);
    assert_eq!(credential.account_id, None);
}

#[test]
fn every_login_provider_takes_a_token() {
    for provider in LoginProvider::ALL {
        let store = Arc::new(FakeStore::default());
        assert_eq!(
            login(&store).execute(provider.id(), "fake-token"),
            Ok(provider)
        );
        assert_eq!(store.stored.lock().unwrap()[0].provider, provider.id());
    }
}

#[test]
fn the_token_is_stored_trimmed() {
    let store = Arc::new(FakeStore::default());
    login(&store).execute("openai", "  fake-token \n").unwrap();
    assert_eq!(store.stored.lock().unwrap()[0].token, "fake-token");
}

#[test]
fn an_unknown_provider_is_refused_before_anything_is_stored() {
    let store = Arc::new(FakeStore::default());
    assert_eq!(
        login(&store).execute("bogus", "fake-token"),
        Err(LoginError::UnknownProvider("bogus".to_string()))
    );
    assert!(store.stored.lock().unwrap().is_empty());
}

#[test]
fn an_empty_or_blank_token_is_refused_before_anything_is_stored() {
    for token in ["", "   ", "\n\t"] {
        let store = Arc::new(FakeStore::default());
        assert_eq!(
            login(&store).execute("openai", token),
            Err(LoginError::EmptyToken),
            "{token:?}"
        );
        assert!(store.stored.lock().unwrap().is_empty());
    }
}

#[test]
fn the_provider_is_checked_before_the_token() {
    let store = Arc::new(FakeStore::default());
    assert_eq!(
        login(&store).execute("bogus", ""),
        Err(LoginError::UnknownProvider("bogus".to_string()))
    );
}

#[test]
fn a_store_failure_is_reported_in_the_stores_words() {
    let store = Arc::new(FakeStore {
        refusal: Some("failed to parse credentials: oops".to_string()),
        ..FakeStore::default()
    });
    assert_eq!(
        login(&store).execute("openai", "fake-token"),
        Err(LoginError::Store(
            "failed to parse credentials: oops".to_string()
        ))
    );
}
