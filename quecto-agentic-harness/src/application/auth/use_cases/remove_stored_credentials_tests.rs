use std::sync::{Arc, Mutex};

use super::*;
use crate::application::auth::ports::{CredentialStoreError, CredentialStoreFailure};
use crate::domain::auth::credential::Credential;

struct FakeStore {
    providers: Mutex<Vec<String>>,
    refusal: Option<String>,
    asked: Mutex<Vec<String>>,
}

impl FakeStore {
    fn holding(providers: &[&str]) -> Arc<Self> {
        Self::with(providers, None)
    }

    fn refusing(reason: &str) -> Arc<Self> {
        Self::with(&[], Some(reason.to_string()))
    }

    fn with(providers: &[&str], refusal: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            providers: Mutex::new(providers.iter().map(|p| p.to_string()).collect()),
            refusal,
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl CredentialStorePort for FakeStore {
    fn store(&self, _credential: Credential) -> Result<(), CredentialStoreError> {
        unreachable!("removing stores nothing")
    }

    fn list(&self) -> Result<Vec<Credential>, CredentialStoreError> {
        unreachable!("removing lists nothing")
    }

    fn remove(&self, provider: &str) -> Result<bool, CredentialStoreError> {
        self.asked.lock().unwrap().push(provider.to_string());
        if let Some(reason) = &self.refusal {
            return Err(CredentialStoreError::new(
                CredentialStoreFailure::Unavailable,
                reason.clone(),
            ));
        }
        let mut providers = self.providers.lock().unwrap();
        let held = providers.iter().any(|held| held == provider);
        providers.retain(|held| held != provider);
        Ok(held)
    }
}

fn remove(store: &Arc<FakeStore>) -> RemoveStoredCredentials {
    RemoveStoredCredentials::new(Arc::clone(store) as Arc<dyn CredentialStorePort>)
}

#[test]
fn a_stored_credential_is_removed_and_no_other() {
    let store = FakeStore::holding(&["openai", "anthropic"]);
    assert_eq!(remove(&store).execute("openai"), Ok(Removal::Removed));
    assert_eq!(*store.providers.lock().unwrap(), ["anthropic"]);
}

#[test]
fn removing_what_is_not_stored_is_no_error() {
    let store = FakeStore::holding(&["anthropic"]);
    assert_eq!(remove(&store).execute("openai"), Ok(Removal::NotStored));
    assert_eq!(*store.providers.lock().unwrap(), ["anthropic"]);
}

#[test]
fn any_stored_provider_name_can_be_removed() {
    // The store is the authority on what is stored: an entry outside the
    // login allowlist (an older or hand-written one) can still be removed.
    let store = FakeStore::holding(&["groq"]);
    assert_eq!(remove(&store).execute("groq"), Ok(Removal::Removed));
    assert_eq!(*store.asked.lock().unwrap(), ["groq"]);
}

#[test]
fn a_store_failure_is_reported_in_its_words() {
    let store = FakeStore::refusing("config error: failed to parse credentials");
    assert_eq!(
        remove(&store).execute("openai"),
        Err(RemoveError::Store(
            "config error: failed to parse credentials".to_string()
        ))
    );
}
