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
    let catalogue =
        FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout.clone())));
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
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-home");
    catalogue.record_new(&identity, &home()).unwrap();
    let first = catalogue.list().unwrap();
    assert!(first.entries.is_empty());
    assert!(first.rebuilt, "an absent index is recovery, reported once");
    assert!(first.diagnostics.iter().any(|d| d.contains("absent")));
    store.save(&session(identity.clone())).await.unwrap();
    // Newer authority supersedes a well-formed index: refreshed silently.
    let snapshot = catalogue.list().unwrap();
    assert!(!snapshot.rebuilt);
    assert!(
        snapshot.diagnostics.is_empty(),
        "{:?}",
        snapshot.diagnostics
    );
    assert_eq!(
        snapshot.entries,
        vec![(identity.clone(), SessionHomeScope::Scoped(home()))]
    );
    assert!(!catalogue.list().unwrap().rebuilt);
    std::fs::write(layout.home_catalogue_file(), b"corrupt").unwrap();
    let recovered = catalogue.list().unwrap();
    assert!(recovered.rebuilt);
    assert!(recovered.diagnostics.iter().any(|d| d.contains("invalid")));
    assert_eq!(snapshot.entries, recovered.entries);
    assert!(!catalogue.list().unwrap().rebuilt);
    std::fs::write(
        layout.home_catalogue_file(),
        br#"{"version":2,"records":{}}"#,
    )
    .unwrap();
    let recovered = catalogue.list().unwrap();
    assert!(recovered.rebuilt);
    assert!(
        recovered
            .diagnostics
            .iter()
            .any(|d| d.contains("version 2 unsupported"))
    );
    assert_eq!(snapshot.entries, recovered.entries);
    assert!(!catalogue.list().unwrap().rebuilt);
    assert!(store.load(&identity).await.unwrap().is_some());
    let restarted = FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout)));
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
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
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
    let catalogue =
        FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout.clone())));
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
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
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
    let catalogue = FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout)));
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
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-digest");
    let mut value = session(identity);
    value.messages = vec![Message::user("DISTINCTIVE-SECRET-TRANSCRIPT")];
    store.save(&value).await.unwrap();
    assert!(catalogue.list().unwrap().rebuilt);
    let cached = std::fs::read_to_string(layout.home_catalogue_file()).unwrap();
    assert!(!cached.contains("DISTINCTIVE-SECRET-TRANSCRIPT"));
    assert!(!catalogue.list().unwrap().rebuilt);
    // A routine autosave then `/resume`: the index is refreshed to the new
    // content, with no rebuild and no diagnostic for the TUI to toast.
    value.messages.push(Message::user("changed"));
    store.save(&value).await.unwrap();
    let routine = catalogue.list().unwrap();
    assert!(!routine.rebuilt);
    assert!(routine.diagnostics.is_empty(), "{:?}", routine.diagnostics);
    let refreshed = std::fs::read_to_string(layout.home_catalogue_file()).unwrap();
    assert_ne!(cached, refreshed, "superseded index is refreshed silently");
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
    let catalogue = FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout)));
    assert!(catalogue.list().unwrap().entries.is_empty());
}

#[test]
fn dangling_home_authority_is_unavailable_not_absent() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("chat-dangling");
    std::os::unix::fs::symlink(dir.path().join("missing"), layout.home_file(&identity)).unwrap();
    let catalogue = FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout)));
    assert!(matches!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Unavailable(_)
    ));
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn warm_projection_reuses_only_unchanged_validated_transcripts() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-incremental");
    store.save(&session(identity.clone())).await.unwrap();
    assert_eq!(catalogue.list().unwrap().entries.len(), 1);
    let reads = catalogue.transcript_reads();
    assert_eq!(catalogue.list().unwrap().entries.len(), 1);
    assert_eq!(
        catalogue.transcript_reads(),
        reads,
        "warm query reread transcript"
    );
    std::fs::write(layout.session_file(&identity), b"{broken").unwrap();
    assert!(catalogue.list().unwrap().entries.is_empty());
    assert!(catalogue.transcript_reads() > reads);
    store.save(&session(identity.clone())).await.unwrap();
    assert_eq!(catalogue.list().unwrap().entries.len(), 1);
    let reads = catalogue.transcript_reads();
    std::fs::write(layout.home_catalogue_file(), b"broken").unwrap();
    assert_eq!(catalogue.list().unwrap().entries.len(), 1);
    assert!(catalogue.transcript_reads() > reads);
    assert!(store.load(&identity).await.unwrap().is_some());
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn warm_projection_invalidates_same_length_rewrites_and_home_changes() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-rewrite");
    store.save(&session(identity.clone())).await.unwrap();
    catalogue.list().unwrap();
    let path = layout.session_file(&identity);
    let old = std::fs::metadata(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, vec![b'x'; bytes.len()]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old.modified().unwrap()))
        .unwrap();
    assert!(catalogue.list().unwrap().entries.is_empty());
    store.save(&session(identity.clone())).await.unwrap();
    catalogue.list().unwrap();
    let reads = catalogue.transcript_reads();
    std::fs::write(layout.home_file(&identity), b"{broken").unwrap();
    let result = catalogue.list().unwrap();
    assert!(matches!(
        result.entries[0].1,
        SessionHomeScope::Unavailable(_)
    ));
    assert_eq!(reads, catalogue.transcript_reads());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_saves_and_catalogue_publication_recover_to_complete_authority() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = Arc::new(FileSessionHomeCatalogue::with_store(store.clone()));
    let identity = SessionIdentity::from_persisted_key("chat-concurrent");
    store.save(&session(identity.clone())).await.unwrap();
    catalogue.list().unwrap();
    let reader = catalogue.clone();
    let listing = tokio::task::spawn_blocking(move || {
        for _ in 0..30 {
            let rows = reader.list().unwrap();
            assert!(rows.entries.len() <= 1);
        }
    });
    for _ in 0..30 {
        store.save(&session(identity.clone())).await.unwrap();
    }
    listing.await.unwrap();
    assert_eq!(
        catalogue.list().unwrap().entries,
        vec![(identity.clone(), SessionHomeScope::LegacyUnscoped)]
    );
    assert!(store.load(&identity).await.unwrap().is_some());
    let disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(layout.home_catalogue_file()).unwrap()).unwrap();
    assert_eq!(disk["version"], 1);
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn asynchronous_projection_preserves_warm_cache_and_checks_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-permissions");
    store.save(&session(identity.clone())).await.unwrap();
    catalogue.list_async().await.unwrap();
    let reads = catalogue.transcript_reads();
    catalogue.list_async().await.unwrap();
    assert_eq!(reads, catalogue.transcript_reads());
    std::fs::set_permissions(
        layout.session_file(&identity),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    catalogue.list_async().await.unwrap();
    assert!(catalogue.transcript_reads() > reads);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_home_replacement_keeps_published_observation_consistent() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-home-race");
    catalogue.record_new(&identity, &home()).unwrap();
    store.save(&session(identity.clone())).await.unwrap();
    let valid = std::fs::read(layout.home_file(&identity)).unwrap();
    let path = layout.home_file(&identity);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer_stop = stop.clone();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let writer_barrier = barrier.clone();
    let writer = std::thread::spawn(move || {
        writer_barrier.wait();
        while !writer_stop.load(std::sync::atomic::Ordering::Acquire) {
            for bytes in [valid.as_slice(), b"{broken"] {
                let mut temporary =
                    tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
                temporary.write_all(bytes).unwrap();
                temporary.persist(&path).unwrap();
            }
        }
    });
    barrier.wait();
    for _ in 0..100 {
        let rows = catalogue.list_async().await.unwrap();
        let index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(layout.home_catalogue_file()).unwrap()).unwrap();
        let bytes: Vec<u8> =
            serde_json::from_value(index["records"]["home:chat-home-race"].clone()).unwrap();
        match &rows.entries[0].1 {
            SessionHomeScope::Scoped(observed) => {
                assert_eq!(observed, &home());
                assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
            }
            SessionHomeScope::Unavailable(_) => assert_eq!(bytes, b"{broken"),
            SessionHomeScope::LegacyUnscoped => panic!("atomic replacement cannot remove home"),
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Release);
    writer.join().unwrap();
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn summary_projection_skips_warm_reads_and_invalidates_rewrites() {
    use quecto::application::sessions::dto::SessionListQuery;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = FileSessionStore::new(layout.clone());
    let identity = SessionIdentity::from_persisted_key("chat-summary-cache");
    store.save(&session(identity.clone())).await.unwrap();
    let query = SessionListQuery::All;
    assert_eq!(store.list(&query).await.unwrap().len(), 1);
    let reads = store.summary_transcript_reads();
    assert_eq!(store.list(&query).await.unwrap().len(), 1);
    assert_eq!(store.summary_transcript_reads(), reads);
    std::fs::write(layout.session_file(&identity), b"{broken").unwrap();
    assert!(store.list(&query).await.unwrap().is_empty());
    assert!(store.summary_transcript_reads() > reads);
    store.save(&session(identity.clone())).await.unwrap();
    assert_eq!(store.list(&query).await.unwrap().len(), 1);
    std::fs::remove_file(layout.session_file(&identity)).unwrap();
    assert!(store.list(&query).await.unwrap().is_empty());
}

#[test]
fn exact_key_read_never_depends_on_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("chat-exact");
    let catalogue =
        FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout.clone())));
    catalogue.record_new(&identity, &home()).unwrap();
    std::fs::write(
        layout.home_catalogue_file(),
        br#"{"version":1,"records":{"home:chat-exact":[0]}}"#,
    )
    .unwrap();
    assert_eq!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Scoped(home())
    );
    std::fs::write(layout.home_catalogue_file(), b"corrupt").unwrap();
    assert_eq!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Scoped(home())
    );
    std::fs::write(layout.home_file(&identity), b"{broken").unwrap();
    assert!(matches!(
        catalogue.read(&identity).unwrap(),
        SessionHomeScope::Unavailable(_)
    ));
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn warm_listing_does_not_reread_unchanged_transcripts_across_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let first = SessionIdentity::from_persisted_key("chat-warm-a");
    let second = SessionIdentity::from_persisted_key("chat-warm-b");
    store.save(&session(first.clone())).await.unwrap();
    store.save(&session(second.clone())).await.unwrap();
    let listed = catalogue.list().unwrap();
    assert_eq!(listed.entries.len(), 2);
    let cold = catalogue.transcript_reads();
    assert!(cold >= 2, "cold listing must parse each transcript once");
    assert_eq!(catalogue.list().unwrap().entries.len(), 2);
    assert_eq!(
        catalogue.transcript_reads(),
        cold,
        "warm listing reread unchanged transcripts"
    );
    let mut changed = session(first.clone());
    changed.messages.push(Message::user("changed"));
    store.save(&changed).await.unwrap();
    assert_eq!(catalogue.list().unwrap().entries.len(), 2);
    let after_one = catalogue.transcript_reads();
    assert!(after_one > cold, "changed transcript must be revalidated");
    assert_eq!(catalogue.list().unwrap().entries.len(), 2);
    assert_eq!(
        catalogue.transcript_reads(),
        after_one,
        "unchanged sibling transcript was reread after a single save"
    );
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn stale_corrupt_index_recovers_from_authority_without_trusting_disk() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-stale-index");
    catalogue.record_new(&identity, &home()).unwrap();
    store.save(&session(identity.clone())).await.unwrap();
    assert_eq!(catalogue.list().unwrap().entries.len(), 1);
    let cold = catalogue.transcript_reads();
    std::fs::write(layout.home_catalogue_file(), b"not-json").unwrap();
    let recovered = catalogue.list().unwrap();
    assert!(recovered.rebuilt);
    assert_eq!(
        recovered.entries,
        vec![(identity.clone(), SessionHomeScope::Scoped(home()))]
    );
    assert!(
        catalogue.transcript_reads() > cold,
        "externally changed index must invalidate the trusted projection"
    );
    let after_recovery = catalogue.transcript_reads();
    assert!(!catalogue.list().unwrap().rebuilt);
    assert_eq!(catalogue.transcript_reads(), after_recovery);
}
