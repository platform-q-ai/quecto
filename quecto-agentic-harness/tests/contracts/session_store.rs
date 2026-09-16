//! Contract tests for the `SessionStore` port.
//!
//! Every adapter must honour the round-trip + existence invariants and the
//! list characterisation of #1861 (D1 of #1968): newest-first summaries, the
//! existing identity-prefix policy, tolerant skipping of records that are
//! not sessions or cannot even be summarised, the summary-only promise (a
//! listed session may still fail a full load), the empty result and the
//! explicit read failure. We drive `FileSessionStore` (the production
//! adapter) through a trait object so the tests can't accidentally depend
//! on adapter-specific surface.
//!
//! Epic close (D10 #1979): `FileSessionStore` is the port's only production
//! implementor, `FlatSessionLayout` its only path former, and every caller
//! of the port is a sessions use case composed by `composition/` — the
//! interface neither holds the store on its dispatch context nor calls a
//! store method (`tests/architecture/sessions_epic_close*.rs`).

use quecto::application::sessions::dto::SessionListQuery;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::message::Message;
use quecto::domain::session::Session;
use quecto::domain::session_identity::{SessionIdentity, SessionKeyPrefix};
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

fn under_test(base_dir: &std::path::Path) -> Arc<dyn SessionStore> {
    Arc::new(FileSessionStore::new(FlatSessionLayout::new(base_dir)))
}

fn id(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

async fn save_with_messages(store: &dyn SessionStore, key: &str, messages: &[&str]) {
    let mut session = Session::new(id(key));
    for text in messages {
        session.messages.push(Message::user(*text));
    }
    store.save(&session).await.unwrap();
}

/// Stamp `<base>/sessions/<file>` with a modification time so the order the
/// list promises is deterministic regardless of write speed.
fn set_modified(base: &std::path::Path, file: &str, unix_secs: u64) {
    let path = base.join("sessions").join(file);
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_secs);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[tokio::test]
async fn exists_is_false_before_save_and_true_after() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());

    assert!(
        !store.exists(&id("cli:fresh")).await.unwrap(),
        "a key that was never saved must not exist"
    );

    let mut saved = Session::new(id("cli:fresh"));
    saved.messages.push(Message::user("hello"));
    store.save(&saved).await.unwrap();

    assert!(
        store.exists(&id("cli:fresh")).await.unwrap(),
        "exists must be true after save"
    );
}

#[tokio::test]
async fn load_returns_none_for_unknown_key_and_saved_session_for_known() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());

    assert!(
        store.load(&id("cli:missing")).await.unwrap().is_none(),
        "load on an unknown key must return None"
    );

    let mut saved = Session::new(id("cli:known"));
    saved.messages.push(Message::user("hello"));
    store.save(&saved).await.unwrap();
    let loaded = store
        .load(&id("cli:known"))
        .await
        .unwrap()
        .expect("load must return Some after save");
    assert_eq!(loaded.key, id("cli:known"));
    assert_eq!(loaded.key.persisted_key(), Some("cli:known"));
    assert_eq!(loaded.messages.len(), 1);
}

#[tokio::test]
async fn save_is_overwrite_and_persists_across_instances() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = under_test(tmp.path());
        let mut saved = Session::new(id("cli:persist"));
        saved.messages.push(Message::user("hello"));
        store.save(&saved).await.unwrap();
    }
    // A fresh instance pointed at the same directory must see the session:
    // the port contract is "persistence", not "in-memory-until-drop".
    let store = under_test(tmp.path());
    assert!(
        store.exists(&id("cli:persist")).await.unwrap(),
        "session must survive adapter reconstruction"
    );
}

#[tokio::test]
async fn ephemeral_identity_is_never_persisted_by_the_callers_and_loads_as_absent() {
    // The store itself is total over identities; the callers no-op on the
    // ephemeral identity. Nothing has ever been saved under it here, so it
    // reads back as absent and lists nothing.
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    assert!(!store.exists(&SessionIdentity::ephemeral()).await.unwrap());
    assert!(
        store
            .load(&SessionIdentity::ephemeral())
            .await
            .unwrap()
            .is_none()
    );
    assert!(store.list(&SessionListQuery::All).await.unwrap().is_empty());
}

// ─── #1861 list characterisation ─────────────────────────────────────────────

#[tokio::test]
async fn list_of_an_absent_or_empty_store_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    assert!(
        store.list(&SessionListQuery::All).await.unwrap().is_empty(),
        "no sessions dir yet: empty, not an error"
    );
    std::fs::create_dir_all(tmp.path().join("sessions")).unwrap();
    assert!(
        store.list(&SessionListQuery::All).await.unwrap().is_empty(),
        "empty sessions dir: empty"
    );
}

#[tokio::test]
async fn list_returns_summaries_newest_first_with_title_and_count() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    save_with_messages(store.as_ref(), "chat-1-old", &["first question", "more"]).await;
    save_with_messages(store.as_ref(), "cli:middle", &["middle question"]).await;
    save_with_messages(store.as_ref(), "chat-3-new", &["newest question"]).await;
    set_modified(tmp.path(), "chat-1-old.json", 1_000);
    set_modified(tmp.path(), "cli_middle.json", 2_000);
    set_modified(tmp.path(), "chat-3-new.json", 3_000);

    let listed = store.list(&SessionListQuery::All).await.unwrap();

    let keys: Vec<&str> = listed.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["chat-3-new", "cli:middle", "chat-1-old"]);
    assert_eq!(listed[0].title, "newest question");
    assert_eq!(listed[0].message_count, 1);
    assert_eq!(listed[0].updated_unix_secs, Some(3_000));
    assert_eq!(listed[2].title, "first question");
    assert_eq!(listed[2].message_count, 2);
}

#[tokio::test]
async fn list_with_an_existing_key_prefix_keeps_only_matching_identities() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    save_with_messages(store.as_ref(), "chat-1", &["a"]).await;
    save_with_messages(store.as_ref(), "chat-2", &["b"]).await;
    save_with_messages(store.as_ref(), "cli:named", &["c"]).await;
    save_with_messages(store.as_ref(), "telegram:1", &["d"]).await;

    let chats = store
        .list(&SessionListQuery::ExistingKeyPrefix(
            SessionKeyPrefix::new("chat-").unwrap(),
        ))
        .await
        .unwrap();
    let mut keys: Vec<&str> = chats.iter().map(|s| s.key.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["chat-1", "chat-2"]);

    let cli = store
        .list(&SessionListQuery::ExistingKeyPrefix(
            SessionKeyPrefix::new("cli:").unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(cli.len(), 1);
    assert_eq!(cli[0].key, "cli:named");

    let none = store
        .list(&SessionListQuery::ExistingKeyPrefix(
            SessionKeyPrefix::new("nothing-").unwrap(),
        ))
        .await
        .unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
async fn list_skips_non_session_records_and_unsummarisable_files() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    save_with_messages(store.as_ref(), "cli:good", &["kept"]).await;
    let dir = tmp.path().join("sessions");
    std::fs::write(dir.join("corrupt.json"), b"{not json").unwrap();
    std::fs::write(dir.join("cli_good.owner"), b"123").unwrap();
    std::fs::write(
        dir.join("notes.txt"),
        b"{\"key\":\"cli:txt\",\"messages\":[]}",
    )
    .unwrap();
    std::fs::write(
        dir.join("cli_good.tmp"),
        b"{\"key\":\"cli:tmp\",\"messages\":[]}",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("cli_good")).unwrap();

    let listed = store.list(&SessionListQuery::All).await.unwrap();

    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].key, "cli:good");
}

#[tokio::test]
async fn list_accepts_unknown_message_details_and_is_summary_only() {
    // The summary parser reads only the key, role and content; a message
    // detail the full record rejects must neither hide the session from the
    // list nor be promised loadable: the port is summary-only.
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    let dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("cli_summary-only.json"),
        br#"{"key":"cli:summary-only","messages":[{"role":"user","content":"listed title","tool_calls":"not-an-array","future_field":{"x":1}}]}"#,
    )
    .unwrap();

    let listed = store.list(&SessionListQuery::All).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "cli:summary-only");
    assert_eq!(listed[0].title, "listed title");
    assert_eq!(listed[0].message_count, 1);

    let load = store.load(&id("cli:summary-only")).await;
    assert!(
        load.is_err(),
        "a listed session is not a load guarantee; got {load:?}"
    );
}

#[tokio::test]
async fn list_reports_an_unreadable_sessions_dir_as_an_error() {
    // `sessions` exists but is a regular file: the read failure is explicit
    // rather than indistinguishable from "no sessions yet".
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("sessions"), b"not a directory").unwrap();
    let store = under_test(tmp.path());
    let err = store.list(&SessionListQuery::All).await.unwrap_err();
    assert!(
        err.to_string()
            .starts_with("session error: failed to read sessions dir"),
        "{err}"
    );
}
