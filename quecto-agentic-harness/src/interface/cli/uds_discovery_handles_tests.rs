use crate::application::sessions::dto::{SearchSessionMetadataRequest, SessionListScope};
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use crate::interface::cli::uds::dispatch_session_roster_tests::composed_sessions;

/// Both handles of the composed bundle answer from the one store.
#[tokio::test]
async fn list_and_search_are_answered_by_their_composed_controllers() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = composed_sessions(tmp.path());
    let identity = SessionIdentity::named_cli("bundle").unwrap();
    let mut session = Session::new(identity.clone());
    session.messages.push(Message::user("find the BUNDLE"));
    sessions.store.save(&session).await.unwrap();
    sessions.store.release(&identity);
    let discovery = sessions.list_sessions.clone();
    let listed = discovery.list(SessionListScope::Global).await.unwrap();
    assert_eq!(listed.sessions.len(), 1);
    let request = SearchSessionMetadataRequest {
        query: "bundle".into(),
        scope: SessionListScope::Global,
        ..Default::default()
    };
    let found = discovery.search(&request).await.unwrap();
    assert_eq!(found.rows.len(), 1);
    assert_eq!(found.rows[0].session.summary.key, "cli:bundle");
    assert_eq!(
        found.rows[0].session.home_version(),
        listed.sessions[0].home_version()
    );
    assert!(format!("{discovery:?}").starts_with("SessionDiscoveryHandles"));
}
