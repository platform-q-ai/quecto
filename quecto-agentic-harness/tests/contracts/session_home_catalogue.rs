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
    assert!(
        !first.rebuilt,
        "an index that never existed is built on first use, not recovered"
    );
    assert!(
        first.diagnostics.is_empty(),
        "first use raises no warning: {:?}",
        first.diagnostics
    );
    assert!(layout.home_catalogue_file().exists());
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
        br#"{"version":3,"records":{}}"#,
    )
    .unwrap();
    let recovered = catalogue.list().unwrap();
    assert!(recovered.rebuilt);
    assert!(
        recovered
            .diagnostics
            .iter()
            .any(|d| d.contains("version 3 unsupported"))
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
    let first = catalogue.list().unwrap();
    assert!(!first.rebuilt, "first use builds the index silently");
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
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
    // The rewrite is detected through ctime; file timestamps use the kernel's
    // coarse clock, so step past the tick in which the save landed.
    std::thread::sleep(std::time::Duration::from_millis(25));
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
    assert_eq!(disk["version"], 2);
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
        // The published entry is written from the row's own observation:
        // either nothing was cached (the sidecar moved under the read) or
        // the cached scope is the row's, never a mix of two versions.
        let cached = &index["records"]["chat-home-race"]["home"];
        let kind = cached["scope"]["kind"].as_str();
        match &rows.entries[0].1 {
            SessionHomeScope::Scoped(observed) => {
                assert_eq!(observed, &home());
                assert!(cached.is_null() || kind == Some("scoped"), "{index}");
            }
            SessionHomeScope::Unavailable(_) => {
                assert!(cached.is_null() || kind == Some("unavailable"), "{index}");
            }
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

/// A legacy pretty-printed transcript that a second, truncated document was
/// appended onto (the shape seen in the field, #2018): the diagnostic must
/// name the record file so the user can find and repair it.
#[test]
fn corrupt_legacy_transcript_diagnostic_names_the_record_file() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    std::fs::create_dir_all(layout.sessions_dir()).unwrap();
    let identity = SessionIdentity::from_persisted_key("cli:slippery-keith");
    let mut doc = String::from("{\n  \"key\": \"cli:slippery-keith\",\n  \"messages\": [\n");
    for i in 0..18 {
        doc.push_str(&format!(
            "    {{\n      \"role\": \"user\",\n      \"content\": \"m{i}\",\n      \"is_pinned\": false\n    }},\n"
        ));
    }
    doc.push_str("    {\n      \"role\": \"user\",\n      \"content\": \"last\"\n    }\n  ]\n}{\n      \"role\": \"user\",\n      \"content\": \"say hello again\",\n      \"is_pinned\": false\n    },\n");
    std::fs::write(layout.session_file(&identity), doc).unwrap();
    let file_name = layout
        .session_file(&identity)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let catalogue = FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout)));
    let snapshot = catalogue.list().unwrap();
    assert!(snapshot.entries.is_empty());
    assert_eq!(snapshot.diagnostics.len(), 1, "{:?}", snapshot.diagnostics);
    let diagnostic = &snapshot.diagnostics[0];
    assert!(
        diagnostic.starts_with(&format!("{file_name}: session record unavailable: ")),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("line "), "{diagnostic}");
}

/// A new process (a new adapter over the same directory, as every TUI tab is)
/// must list a large, unchanged directory from the persisted index alone:
/// one `stat` per file, no transcript read. Only a record that changed since
/// the index was written is read again.
#[cfg(feature = "test-support")]
#[tokio::test]
async fn cold_process_lists_from_the_persisted_index_without_reading_transcripts() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let count = 2_000;
    let identities: Vec<SessionIdentity> = (0..count)
        .map(|n| SessionIdentity::from_persisted_key(format!("chat-cold-{n:04}")))
        .collect();
    for identity in &identities {
        store.record_new_home(identity, &home()).unwrap();
        store.save(&session(identity.clone())).await.unwrap();
    }
    let warm = FileSessionHomeCatalogue::with_store(store.clone());
    let first = warm.list().unwrap();
    assert_eq!(first.entries.len(), count);
    assert_eq!(
        warm.transcript_reads(),
        count,
        "a fresh index reads each once"
    );
    let index = std::fs::read(layout.home_catalogue_file()).unwrap();
    assert!(!warm.list().unwrap().rebuilt);
    assert_eq!(
        std::fs::read(layout.home_catalogue_file()).unwrap(),
        index,
        "an unchanged directory must not rewrite the index"
    );
    drop(warm);
    let cold =
        FileSessionHomeCatalogue::with_store(Arc::new(FileSessionStore::new(layout.clone())));
    let started = std::time::Instant::now();
    let listed = cold.list().unwrap();
    let elapsed = started.elapsed();
    assert_eq!(listed.entries, first.entries);
    assert!(!listed.rebuilt);
    assert!(listed.diagnostics.is_empty(), "{:?}", listed.diagnostics);
    assert_eq!(
        cold.transcript_reads(),
        0,
        "a valid persisted index seeds the cold projection: no transcript read"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "cold listing over {count} records took {elapsed:?}"
    );
    let mut changed = session(identities[7].clone());
    changed
        .messages
        .push(Message::user("appended after the index"));
    store.save(&changed).await.unwrap();
    assert_eq!(cold.list().unwrap().entries.len(), count);
    assert_eq!(
        cold.transcript_reads(),
        1,
        "only the appended transcript is read again"
    );
}

/// The index is a cache the user's own tools write; an edited entry can
/// misreport a home in the listing at most. Admission never consults it:
/// `read` is the exact sidecar, so a doctored entry can neither admit nor
/// refuse a resume.
#[cfg(feature = "test-support")]
#[tokio::test]
async fn doctored_index_entry_never_changes_exact_read_or_admission() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
    let identity = SessionIdentity::from_persisted_key("chat-doctored");
    catalogue.record_new(&identity, &home()).unwrap();
    store.save(&session(identity.clone())).await.unwrap();
    let query = quecto::application::sessions::dto::SessionListQuery::All;
    store.list(&query).await.unwrap();
    catalogue.list().unwrap();
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(layout.home_catalogue_file()).unwrap()).unwrap();
    let entry = &mut index["records"]["chat-doctored"];
    assert_eq!(entry["stamp"].as_array().map(Vec::len), Some(8));
    // Real stamps, a lie about where the session was saved.
    entry["home"]["scope"] = serde_json::json!({
        "kind": "scoped",
        "execution_dir": "/elsewhere",
        "group_kind": "folder",
        "group_path": "/elsewhere",
    });
    std::fs::write(
        layout.home_catalogue_file(),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    let restarted = FileSessionHomeCatalogue::with_store(store.clone());
    let listed = restarted.list().unwrap();
    assert_eq!(restarted.transcript_reads(), 0);
    assert_eq!(listed.entries.len(), 1);
    // Exact reads are the sidecar, whatever the index claims.
    assert_eq!(
        restarted.read(&identity).unwrap(),
        SessionHomeScope::Scoped(home())
    );
    // A stamp that does not match the file is never trusted: the record is
    // read and validated again, and the stale row is replaced.
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(layout.home_catalogue_file()).unwrap()).unwrap();
    index["records"]["chat-doctored"]["stamp"][1] = serde_json::json!(1);
    std::fs::write(
        layout.home_catalogue_file(),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    let restarted = FileSessionHomeCatalogue::with_store(store.clone());
    let listed = restarted.list().unwrap();
    assert_eq!(restarted.transcript_reads(), 1);
    assert_eq!(
        listed.entries,
        vec![(identity.clone(), SessionHomeScope::Scoped(home()))]
    );
    // The same rule guards the summary walk: an entry whose stamp does not
    // match the file is read again, whatever summary it carries.
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(layout.home_catalogue_file()).unwrap()).unwrap();
    index["records"]["chat-doctored"]["stamp"][1] = serde_json::json!(1);
    index["records"]["chat-doctored"]["summary"]["title"] = serde_json::json!("doctored");
    std::fs::write(
        layout.home_catalogue_file(),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    let fresh = Arc::new(FileSessionStore::new(layout.clone()));
    let summaries = fresh.list(&query).await.unwrap();
    assert_eq!(fresh.summary_transcript_reads(), 1);
    assert_eq!(summaries[0].title, "hello");
}
