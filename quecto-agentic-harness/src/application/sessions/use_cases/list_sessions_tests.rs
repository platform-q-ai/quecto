use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::*;
use crate::domain::session::Session;
use crate::domain::session_identity::{SessionIdentity, SessionKeyPrefix};

/// A store fake that records the queries it receives and answers with a
/// scripted outcome; it never loads a session (list is summary-only).
struct ScriptedStore {
    queries: Mutex<Vec<SessionListQuery>>,
    outcome: Mutex<Option<Result<Vec<SessionSummary>, DomainError>>>,
    loads: Mutex<usize>,
}

impl ScriptedStore {
    fn answering(outcome: Result<Vec<SessionSummary>, DomainError>) -> Arc<Self> {
        Arc::new(Self {
            queries: Mutex::new(Vec::new()),
            outcome: Mutex::new(Some(outcome)),
            loads: Mutex::new(0),
        })
    }
}

impl SessionStore for ScriptedStore {
    fn load(
        &self,
        _identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
        *self.loads.lock().unwrap() += 1;
        Box::pin(async { Ok(None) })
    }

    fn save(
        &self,
        _session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn exists(
        &self,
        _identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }

    fn list(
        &self,
        query: &SessionListQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        self.queries.lock().unwrap().push(query.clone());
        let outcome = self.outcome.lock().unwrap().take();
        Box::pin(async move { outcome.expect("one list per scripted store") })
    }
}

fn summary(key: &str, updated: Option<u64>) -> SessionSummary {
    SessionSummary {
        key: key.to_string(),
        title: format!("title of {key}"),
        message_count: 2,
        updated_unix_secs: updated,
    }
}

#[tokio::test]
async fn all_query_returns_the_store_summaries_in_store_order() {
    // The adapter promises newest-first; the query passes that order through
    // untouched (no re-sort, no filtering) and performs no full loads.
    let newest_first = vec![
        summary("chat-3", Some(30)),
        summary("cli:two", Some(20)),
        summary("chat-1", Some(10)),
        summary("cli:undated", None),
    ];
    let store = ScriptedStore::answering(Ok(newest_first.clone()));
    let query = ListSessions::new(store.clone());

    let listed = query.execute(&SessionListQuery::All).await.unwrap();

    assert_eq!(listed, newest_first);
    assert_eq!(*store.queries.lock().unwrap(), vec![SessionListQuery::All]);
    assert_eq!(*store.loads.lock().unwrap(), 0, "list is summary-only");
}

#[tokio::test]
async fn prefix_query_is_handed_to_the_store_as_the_identity_prefix() {
    let store = ScriptedStore::answering(Ok(vec![summary("chat-9", Some(9))]));
    let query = ListSessions::new(store.clone());
    let prefix = SessionKeyPrefix::new("chat-").unwrap();

    let listed = query
        .execute(&SessionListQuery::ExistingKeyPrefix(prefix.clone()))
        .await
        .unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(
        *store.queries.lock().unwrap(),
        vec![SessionListQuery::ExistingKeyPrefix(prefix)]
    );
}

#[tokio::test]
async fn empty_store_lists_nothing() {
    let store = ScriptedStore::answering(Ok(Vec::new()));
    let listed = ListSessions::new(store)
        .execute(&SessionListQuery::All)
        .await
        .unwrap();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn store_error_surfaces_unchanged() {
    let store = ScriptedStore::answering(Err(DomainError::Session(
        "failed to read sessions dir: boom".to_string(),
    )));
    let err = ListSessions::new(store)
        .execute(&SessionListQuery::All)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "session error: failed to read sessions dir: boom"
    );
}

#[test]
fn debug_does_not_print_the_store() {
    let store = ScriptedStore::answering(Ok(Vec::new()));
    assert_eq!(
        format!("{:?}", ListSessions::new(store)),
        "ListSessions { .. }"
    );
}

#[tokio::test]
async fn local_discovery_without_workspace_facts_never_broadens_to_global() {
    use crate::application::sessions::dto::{ListSessionsRequest, SessionListScope};
    let store = ScriptedStore::answering(Ok(vec![summary("chat-foreign", Some(1))]));
    let listed = ListSessions::new(store)
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Local,
        })
        .await
        .unwrap();
    assert!(listed.sessions.is_empty());
    assert!(!listed.diagnostics.is_empty());
}
