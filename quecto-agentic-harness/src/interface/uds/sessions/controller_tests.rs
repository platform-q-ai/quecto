use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;

struct RecordingStore {
    queries: Mutex<Vec<SessionListQuery>>,
    fail: bool,
}

impl SessionStore for RecordingStore {
    fn load(
        &self,
        _identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
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
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(DomainError::Session("read failed".into()))
            } else {
                Ok(vec![SessionSummary {
                    key: "chat-1".into(),
                    title: "hi".into(),
                    message_count: 1,
                    updated_unix_secs: Some(1),
                }])
            }
        })
    }
}

fn controller(fail: bool) -> (Arc<RecordingStore>, ListSessionsController) {
    let store = Arc::new(RecordingStore {
        queries: Mutex::new(Vec::new()),
        fail,
    });
    let use_case = Arc::new(ListSessions::new(store.clone()));
    (store, ListSessionsController::new(use_case))
}

#[tokio::test]
async fn list_all_maps_the_fieldless_command_to_the_all_query() {
    let (store, controller) = controller(false);
    let listed = controller.list_all().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "chat-1");
    assert_eq!(*store.queries.lock().unwrap(), vec![SessionListQuery::All]);
}

#[tokio::test]
async fn store_errors_pass_through_for_the_presenter() {
    let (_, controller) = controller(true);
    let err = controller.list_all().await.unwrap_err();
    assert_eq!(err.to_string(), "session error: read failed");
    assert_eq!(format!("{controller:?}"), "ListSessionsController { .. }");
}

#[tokio::test]
async fn scoped_controller_uses_application_discovery_without_guessing_home() {
    let (store, controller) = controller(false);
    let local = controller
        .list(SessionListScopeCommand::Local)
        .await
        .unwrap();
    assert!(local.sessions.is_empty());
    assert!(!local.diagnostics.is_empty());
    let global = controller
        .list(SessionListScopeCommand::Global)
        .await
        .unwrap();
    assert_eq!(global.sessions[0].summary.key, "chat-1");
    assert!(!global.sessions[0].resume_eligible);
    assert!(matches!(
        global.sessions[0].home,
        crate::domain::session_home::SessionHomeScope::Unavailable(_)
    ));
    assert!(
        store
            .queries
            .lock()
            .unwrap()
            .iter()
            .all(|q| *q == SessionListQuery::All)
    );
}
