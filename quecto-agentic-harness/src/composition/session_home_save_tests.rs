//! Real-adapter acceptance: discovery failures preserve transcripts (spec:
//! "Permission/partial-write failures preserve transcripts"), and a record
//! the strict catalogue rejects stays globally visible, never eligible.
use super::*;
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::dto::{
    ListSessionsRequest, SaveTrigger, SessionListQuery, SessionListScope,
};
use crate::application::sessions::use_cases::{ListSessions, SaveSession};
use crate::application::sessions::{active_session::ActiveSessionState, ports::SessionStore};
use crate::domain::session_home::SessionHomeScope;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::{message::Message, session::Session};
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::path::PathBuf;

fn save_in(
    identity: &SessionIdentity,
    store: Arc<FileSessionStore>,
    context: &SessionHomeContext,
) -> SaveSession {
    SaveSession::new(
        Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
            identity.clone(),
        ))),
        store,
        Arc::new(DurablePrefixLatch::default()),
        None,
        None,
        false,
    )
    .with_home(context.clone())
}

#[tokio::test]
async fn existing_legacy_save_preserves_history_when_workspace_discovery_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let identity = SessionIdentity::user_chat("chat-legacy-save").unwrap();
    let mut session = Session::new(identity.clone());
    session.messages.push(Message::user("before"));
    store.save(&session).await.unwrap();
    let context = session_home_in(store.clone(), Ok(temp.path().join("missing")));
    let save = save_in(&identity, store.clone(), &context);
    session.messages.push(Message::user("after"));
    save.save(&mut session.messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(
        store.load(&identity).await.unwrap().unwrap().messages.len(),
        2
    );
    assert_eq!(
        context.catalogue.read(&identity).unwrap(),
        SessionHomeScope::LegacyUnscoped
    );
    assert!(!layout.home_file(&identity).exists());
}

/// H1 (consequence): the first save of a NEW identity whose discovery fails
/// still writes the transcript, without a `.home`, and stays listable.
#[tokio::test]
async fn new_session_save_preserves_history_when_workspace_discovery_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let identity = SessionIdentity::user_chat("chat-new-nohome").unwrap();
    let context = session_home_in(store.clone(), Ok(temp.path().join("missing")));
    let save = save_in(&identity, store.clone(), &context);
    let mut messages = vec![Message::user("first turn")];
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    messages.push(Message::user("second turn"));
    save.save(&mut messages, SaveTrigger::Routine)
        .await
        .unwrap();
    assert_eq!(
        store.load(&identity).await.unwrap().unwrap().messages.len(),
        2
    );
    assert!(!layout.home_file(&identity).exists());
    assert_eq!(
        context.catalogue.read(&identity).unwrap(),
        SessionHomeScope::LegacyUnscoped
    );
    let listed = ListSessions::new(store)
        .with_home(context)
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Global,
        })
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].home, SessionHomeScope::LegacyUnscoped);
    assert!(!listed.sessions[0].resume_eligible);
    assert!(
        listed
            .diagnostics
            .iter()
            .any(|d| d.contains("canonicalization failed")),
        "the discovery failure is observable: {:?}",
        listed.diagnostics
    );
}

/// L3: an unreadable current directory is a typed unavailable, never `""`.
#[tokio::test]
async fn unreadable_current_directory_is_observably_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(temp.path())));
    let context = session_home_in(
        store.clone(),
        Err("No such file or directory (os error 2)".into()),
    );
    let error = context.current().await.unwrap_err().to_string();
    assert!(error.contains("execution directory unavailable"), "{error}");
    assert!(error.contains("os error 2"), "{error}");
    let listed = ListSessions::new(store)
        .with_home(context)
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Local,
        })
        .await
        .unwrap();
    assert!(listed.diagnostics.iter().any(|d| d.contains("os error 2")));
}

/// M1: a crash-truncated transcript is listable by the tolerant store but
/// rejected by the strict catalogue; it stays a Global row, never eligible.
#[tokio::test]
async fn crash_truncated_transcript_is_listed_globally_as_not_in_catalogue() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let context = session_home_in(store.clone(), Ok(temp.path().to_path_buf()));
    let intact = SessionIdentity::user_chat("chat-intact").unwrap();
    save_in(&intact, store.clone(), &context)
        .save(&mut vec![Message::user("intact")], SaveTrigger::Routine)
        .await
        .unwrap();
    store.release(&intact);
    let truncated = SessionIdentity::user_chat("chat-truncated").unwrap();
    save_in(&truncated, store.clone(), &context)
        .save(&mut vec![Message::user("one")], SaveTrigger::Routine)
        .await
        .unwrap();
    store.release(&truncated);
    let path: PathBuf = layout.session_file(&truncated);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\n{\"partial\": [1, 2");
    std::fs::write(&path, bytes).unwrap();
    let summaries = store.list(&SessionListQuery::All).await.unwrap();
    assert!(
        summaries.iter().any(|s| s.key == truncated.runtime_key()),
        "the tolerant store lists the truncated record"
    );
    for scope in [SessionListScope::Global, SessionListScope::Local] {
        let listed = ListSessions::new(store.clone())
            .with_home(context.clone())
            .discover(&ListSessionsRequest {
                query: SessionListQuery::All,
                scope,
            })
            .await
            .unwrap();
        let row = listed
            .sessions
            .iter()
            .find(|row| row.summary.key == truncated.runtime_key());
        if scope == SessionListScope::Global {
            let row = row.expect("Global lists every saved identity");
            assert_eq!(
                row.home,
                SessionHomeScope::Unavailable("record not in catalogue".into())
            );
            assert!(!row.resume_eligible);
        } else {
            assert!(row.is_none(), "an unavailable home is never local");
        }
        let intact_row = listed
            .sessions
            .iter()
            .find(|row| row.summary.key == intact.runtime_key())
            .expect("one malformed record cannot hide valid sessions");
        assert!(intact_row.resume_eligible);
    }
}
