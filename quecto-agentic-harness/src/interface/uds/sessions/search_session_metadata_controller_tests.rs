use super::*;
use crate::application::sessions::dto::{QueryGeneration, SessionListScope};
use crate::application::sessions::ports::session_home::{
    HomeCatalogueSnapshot, SessionHomeCatalogue, SessionMetadataRecord, SessionMetadataSnapshot,
    WorkspaceDiscovery,
};
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::{SessionHome, SessionHomeScope};
use crate::domain::session_identity::SessionIdentity;
use std::future::Future;
use std::pin::Pin;

struct Catalogue {
    fail: bool,
}
impl SessionHomeCatalogue for Catalogue {
    fn read(&self, _: &SessionIdentity) -> Result<SessionHomeScope, DomainError> {
        panic!("a controller reads nothing")
    }
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError> {
        panic!("a controller lists nothing")
    }
    fn record_new(&self, _: &SessionIdentity, _: &SessionHome) -> Result<(), DomainError> {
        panic!("a controller writes nothing")
    }
    fn discard_orphan(&self, _: &SessionIdentity) -> Result<(), DomainError> {
        panic!("a controller writes nothing")
    }
    fn metadata(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SessionMetadataSnapshot, DomainError>> + Send + '_>>
    {
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                return Err(DomainError::Session("walk failed".into()));
            }
            Ok(SessionMetadataSnapshot {
                records: vec![SessionMetadataRecord {
                    summary: SessionSummary {
                        key: "chat-1".into(),
                        identity: SessionIdentity::from_persisted_key("chat-1"),
                        title: "Fix the flaky test".into(),
                        message_count: 3,
                        updated_unix_secs: Some(9),
                    },
                    home: SessionHomeScope::LegacyUnscoped,
                }],
                diagnostics: Vec::new(),
                rebuilt: false,
            })
        })
    }
}

struct Nowhere;
impl WorkspaceDiscovery for Nowhere {
    fn discover(&self, _: &std::path::Path) -> Result<SessionHome, DomainError> {
        Err(DomainError::Session("no workspace".into()))
    }
}

fn controller(fail: bool) -> SearchSessionMetadataController {
    let home = SessionHomeContext::at(Arc::new(Catalogue { fail }), Arc::new(Nowhere), "/x".into());
    SearchSessionMetadataController::new(Arc::new(SearchSessionMetadata::new(home)))
}

#[tokio::test]
async fn the_typed_request_reaches_the_owner_and_its_typed_answer_comes_back() {
    let request = SearchSessionMetadataRequest {
        query: "FLAKY".into(),
        scope: SessionListScope::Global,
        generation: QueryGeneration(4),
        ..Default::default()
    };
    let answer = controller(false).search(&request).await.unwrap();
    assert_eq!(answer.generation, QueryGeneration(4));
    assert_eq!(answer.rows.len(), 1);
    assert_eq!(answer.rows[0].session.summary.key, "chat-1");
    let nothing = SearchSessionMetadataRequest {
        query: "absent".into(),
        scope: SessionListScope::Global,
        ..Default::default()
    };
    assert!(
        controller(false)
            .search(&nothing)
            .await
            .unwrap()
            .rows
            .is_empty()
    );
}

#[tokio::test]
async fn the_owners_error_is_the_controllers_error_and_debug_prints_no_graph() {
    let error = controller(true)
        .search(&SearchSessionMetadataRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "session error: walk failed");
    assert_eq!(
        format!("{:?}", controller(false)),
        "SearchSessionMetadataController { .. }"
    );
}
