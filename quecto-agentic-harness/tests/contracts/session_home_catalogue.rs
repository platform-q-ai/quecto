use quecto::application::sessions::ports::session_home::SessionHomeCatalogue;
use quecto::domain::session_home::SessionHomeScope;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::{
    session_home_catalogue::FileSessionHomeCatalogue, session_layout::FlatSessionLayout,
};

#[test]
fn corrupt_authority_is_unavailable_not_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("chat-test");
    let catalogue = FileSessionHomeCatalogue::with_store(
        layout.clone(),
        Arc::new(FileSessionStore::new(layout.clone())),
    );
    assert_eq!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::LegacyUnscoped
    );
    std::fs::write(layout.home_file(&identity), b"{broken").unwrap();
    assert!(matches!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Unavailable(_)
    ));
}

use quecto::application::sessions::ports::SessionStore;
use quecto::domain::{
    message::Message,
    session::Session,
    session_home::{AssociationProvenance, SessionHome, WorkspaceGroup},
};
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

fn home() -> SessionHome {
    SessionHome {
        execution_dir: "/workspace".into(),
        group: WorkspaceGroup::Folder {
            directory: "/workspace".into(),
        },
        provenance: AssociationProvenance::SavedHere,
    }
}
fn session(identity: SessionIdentity) -> Session {
    Session {
        key: identity,
        messages: vec![Message::user("hello")],
        workflow_run: None,
        subagent_roster: vec![],
    }
}
#[tokio::test]
async fn new_home_survives_restart_and_catalogue_recovers_without_orphans() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(layout.clone(), store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-home");
    catalogue.record_new(&identity, &home()).unwrap();
    assert!(catalogue.list().unwrap().entries.is_empty());
    store.save(&session(identity.clone())).await.unwrap();
    let snapshot = catalogue.list().unwrap();
    assert!(snapshot.rebuilt);
    assert_eq!(
        snapshot.entries,
        vec![(identity.clone(), SessionHomeScope::Scoped(home()))]
    );
    assert!(!catalogue.list().unwrap().rebuilt);
    std::fs::write(layout.home_catalogue_file(), b"corrupt").unwrap();
    let recovered = catalogue.list().unwrap();
    assert!(recovered.rebuilt);
    assert_eq!(snapshot.entries, recovered.entries);
    assert!(!catalogue.list().unwrap().rebuilt);
    assert!(store.load(&identity).await.unwrap().is_some());
    let restarted = FileSessionHomeCatalogue::with_store(
        layout.clone(),
        Arc::new(FileSessionStore::new(layout)),
    );
    assert_eq!(
        restarted.read(&identity).unwrap(),
        SessionHomeScope::Scoped(home())
    );
}
#[tokio::test]
async fn ordinary_full_and_delta_saves_preserve_invalid_authority_bytes_and_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(layout.clone(), store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-legacy");
    let mut value = session(identity.clone());
    store.save(&value).await.unwrap();
    catalogue.record_new(&identity, &home()).unwrap();
    assert_eq!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::LegacyUnscoped
    );
    for bytes in [b"{broken".as_slice(), b"{\"version\":999}".as_slice()] {
        std::fs::write(layout.home_file(&identity), bytes).unwrap();
        store.save(&value).await.unwrap();
        value.messages.push(Message::user("more"));
        store
            .save_delta(&identity, &value.messages, 1, None)
            .await
            .unwrap();
        store
            .save_clean_delta(&identity, &value.messages, value.messages.len(), None)
            .await
            .unwrap();
        catalogue.record_new(&identity, &home()).unwrap();
        assert_eq!(std::fs::read(layout.home_file(&identity)).unwrap(), bytes);
        assert!(matches!(
            catalogue.read(&identity).unwrap(),
            SessionHomeScope::Unavailable(_)
        ));
    }
    let other = SessionIdentity::from_persisted_key("chat-valid");
    catalogue.record_new(&other, &home()).unwrap();
    store.save(&session(other.clone())).await.unwrap();
    std::fs::write(layout.sessions_dir().join("malformed.json"), b"{broken").unwrap();
    let rows = catalogue.list().unwrap();
    assert_eq!(rows.entries.len(), 2);
    assert!(
        rows.entries
            .contains(&(other, SessionHomeScope::Scoped(home())))
    );
    assert!(!rows.diagnostics.is_empty());
}
#[test]
fn ephemeral_metadata_is_never_durable() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let catalogue = FileSessionHomeCatalogue::with_store(
        layout.clone(),
        Arc::new(FileSessionStore::new(layout.clone())),
    );
    catalogue
        .record_new(&SessionIdentity::ephemeral(), &home())
        .unwrap();
    assert!(!layout.sessions_dir().exists());
}

#[tokio::test]
async fn catalogue_write_failure_preserves_authority_and_exact_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(layout.clone(), store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-fault");
    catalogue.record_new(&identity, &home()).unwrap();
    store.save(&session(identity.clone())).await.unwrap();
    let before = std::fs::read(layout.session_file(&identity)).unwrap();
    std::fs::create_dir(layout.home_catalogue_file()).unwrap();
    let snapshot = catalogue.list().unwrap();
    assert_eq!(
        snapshot.entries,
        vec![(identity.clone(), SessionHomeScope::Scoped(home()))]
    );
    assert!(
        snapshot
            .diagnostics
            .iter()
            .any(|d| d.contains("replacement unavailable"))
    );
    assert_eq!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Scoped(home())
    );
    assert_eq!(
        std::fs::read(layout.session_file(&identity)).unwrap(),
        before
    );
}

#[test]
fn partially_appended_transcript_cannot_publish_a_catalogue_row() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("chat-partial");
    std::fs::write(layout.session_file(&identity), b"{\"type\":\"snapshot\",\"key\":\"chat-partial\",\"messages\":[]}\n{\"type\":\"append\",\"messages\":[").unwrap();
    let catalogue = FileSessionHomeCatalogue::with_store(
        layout.clone(),
        Arc::new(FileSessionStore::new(layout)),
    );
    let snapshot = catalogue.list().unwrap();
    assert!(snapshot.entries.is_empty());
    assert!(
        snapshot
            .diagnostics
            .iter()
            .any(|d| d.contains("record unavailable"))
    );
}

#[tokio::test]
async fn derived_index_contains_no_transcript_and_detects_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(layout.clone(), store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-digest");
    let mut value = session(identity);
    value.messages = vec![Message::user("DISTINCTIVE-SECRET-TRANSCRIPT")];
    store.save(&value).await.unwrap();
    assert!(catalogue.list().unwrap().rebuilt);
    let cached = std::fs::read_to_string(layout.home_catalogue_file()).unwrap();
    assert!(!cached.contains("DISTINCTIVE-SECRET-TRANSCRIPT"));
    assert!(!catalogue.list().unwrap().rebuilt);
    value.messages.push(Message::user("changed"));
    store.save(&value).await.unwrap();
    assert!(catalogue.list().unwrap().rebuilt);
    assert!(!catalogue.list().unwrap().rebuilt);
}

#[test]
fn syntactically_valid_but_schema_invalid_record_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("chat-invalid");
    std::fs::write(
        layout.session_file(&identity),
        b"{\"key\":\"chat-invalid\",\"messages\":42}",
    )
    .unwrap();
    let catalogue = FileSessionHomeCatalogue::with_store(
        layout.clone(),
        Arc::new(FileSessionStore::new(layout)),
    );
    assert!(catalogue.list().unwrap().entries.is_empty());
}

#[test]
fn dangling_home_authority_is_unavailable_not_absent() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("chat-dangling");
    std::os::unix::fs::symlink(dir.path().join("missing"), layout.home_file(&identity)).unwrap();
    let catalogue = FileSessionHomeCatalogue::with_store(
        layout.clone(),
        Arc::new(FileSessionStore::new(layout)),
    );
    assert!(matches!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Unavailable(_)
    ));
}
