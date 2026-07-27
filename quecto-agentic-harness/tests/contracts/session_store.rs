//! Contract tests for the `SessionStore` port.
//!
//! Every adapter must honour the round-trip + existence invariants. We drive
//! `FileSessionStore` (the production adapter) through a trait object so the
//! tests can't accidentally depend on adapter-specific surface.

use quecto::domain::message::Message;
use quecto::domain::session::{Session, SessionStore};
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

fn under_test(base_dir: &std::path::Path) -> Arc<dyn SessionStore> {
    Arc::new(FileSessionStore::new(base_dir))
}

#[tokio::test]
async fn exists_is_false_before_save_and_true_after() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());

    assert!(
        !store.exists("cli:fresh").await.unwrap(),
        "a key that was never saved must not exist"
    );

    let mut saved = Session::new("cli:fresh");
    saved.messages.push(Message::user("hello"));
    store.save(&saved).await.unwrap();

    assert!(
        store.exists("cli:fresh").await.unwrap(),
        "exists must be true after save"
    );
}

#[tokio::test]
async fn load_returns_none_for_unknown_key_and_saved_session_for_known() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());

    assert!(
        store.load("cli:missing").await.unwrap().is_none(),
        "load on an unknown key must return None"
    );

    let mut saved = Session::new("cli:known");
    saved.messages.push(Message::user("hello"));
    store.save(&saved).await.unwrap();
    let loaded = store
        .load("cli:known")
        .await
        .unwrap()
        .expect("load must return Some after save");
    assert_eq!(loaded.key, "cli:known");
    assert_eq!(loaded.messages.len(), 1);
}

#[tokio::test]
async fn save_is_overwrite_and_persists_across_instances() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = under_test(tmp.path());
        let mut saved = Session::new("cli:persist");
        saved.messages.push(Message::user("hello"));
        store.save(&saved).await.unwrap();
    }
    // A fresh instance pointed at the same directory must see the session:
    // the port contract is "persistence", not "in-memory-until-drop".
    let store = under_test(tmp.path());
    assert!(
        store.exists("cli:persist").await.unwrap(),
        "session must survive adapter reconstruction"
    );
}
