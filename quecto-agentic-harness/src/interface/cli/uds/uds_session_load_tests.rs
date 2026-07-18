use super::*;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{SessionStore, SessionSummary};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

#[derive(Debug)]
struct StubStore {
    loaded: Mutex<Result<Option<Session>, DomainError>>,
    load_keys: Mutex<Vec<String>>,
}

impl StubStore {
    fn with_loaded(loaded: Result<Option<Session>, DomainError>) -> Self {
        Self {
            loaded: Mutex::new(loaded),
            load_keys: Mutex::new(Vec::new()),
        }
    }

    fn load_count(&self) -> usize {
        self.load_keys.lock().unwrap().len()
    }
}

impl SessionStore for StubStore {
    fn load(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
        self.load_keys.lock().unwrap().push(key.to_string());
        let result = match &*self.loaded.lock().unwrap() {
            Ok(session) => Ok(session.clone()),
            Err(error) => Err(DomainError::Session(error.to_string())),
        };
        Box::pin(async move { result })
    }

    fn save(
        &self,
        _session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn exists(
        &self,
        _key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }

    fn list(
        &self,
        _key_prefix: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn ephemeral_session_does_not_touch_store() {
    let store = StubStore::with_loaded(Err(DomainError::Session("must not load".into())));

    let session = load_session(&store, "cli:default", true).await.unwrap();

    assert_eq!(session.key, "cli:default");
    assert!(session.messages.is_empty());
    assert_eq!(
        store.load_count(),
        0,
        "ephemeral sessions must bypass persistence"
    );
}

#[tokio::test]
async fn empty_key_does_not_touch_store() {
    let store = StubStore::with_loaded(Err(DomainError::Session("must not load".into())));

    let session = load_session(&store, "", false).await.unwrap();

    assert_eq!(session.key, "");
    assert_eq!(
        store.load_count(),
        0,
        "empty session key must bypass persistence"
    );
}

#[tokio::test]
async fn missing_persisted_session_returns_fresh_session_with_requested_key() {
    let store = StubStore::with_loaded(Ok(None));

    let session = load_session(&store, "cli:missing", false).await.unwrap();

    assert_eq!(session.key, "cli:missing");
    assert!(session.messages.is_empty());
    assert_eq!(store.load_count(), 1);
}

#[tokio::test]
async fn existing_persisted_session_is_returned() {
    let mut existing = Session::new("cli:existing");
    existing.messages.push(Message::user("hello"));
    let store = StubStore::with_loaded(Ok(Some(existing.clone())));

    let session = load_session(&store, "cli:existing", false).await.unwrap();

    assert_eq!(session.key, existing.key);
    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].content, "hello");
}

#[tokio::test]
async fn load_error_is_stringified() {
    let store = StubStore::with_loaded(Err(DomainError::Session("disk is sad".into())));

    let err = load_session(&store, "cli:bad", false).await.unwrap_err();

    assert!(err.contains("disk is sad"), "got: {err}");
    assert_eq!(store.load_count(), 1);
}
