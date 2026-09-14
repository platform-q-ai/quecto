use std::sync::Arc;

use super::*;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;

#[tokio::test]
async fn handles_over_the_file_store_list_what_the_store_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let handles = build_session_handles(SessionLoopInputs {
        base_dir: tmp.path().to_path_buf(),
        store: None,
    });
    let mut session = Session::new(SessionIdentity::named_cli("composed").unwrap());
    session.messages.push(Message::user("hello"));
    handles.store.save(&session).await.unwrap();

    let listed = handles.list_sessions.list_all().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "cli:composed");
    assert!(
        tmp.path().join("sessions/cli_composed.json").exists(),
        "the composed store writes the flat layout"
    );
}

#[tokio::test]
async fn a_supplied_store_is_used_as_is() {
    let tmp = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let store: Arc<dyn SessionStore> = Arc::new(build_file_session_store(elsewhere.path()));
    let handles = build_session_handles(SessionLoopInputs {
        base_dir: tmp.path().to_path_buf(),
        store: Some(store),
    });
    let mut session = Session::new(SessionIdentity::named_cli("override").unwrap());
    session.messages.push(Message::user("hello"));
    handles.store.save(&session).await.unwrap();
    assert!(elsewhere.path().join("sessions/cli_override.json").exists());
    assert!(!tmp.path().join("sessions").exists());
    assert_eq!(handles.list_sessions.list_all().await.unwrap().len(), 1);
}
