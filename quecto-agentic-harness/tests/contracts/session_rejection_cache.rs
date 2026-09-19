//! Contract of the rejection cache (R1-H1) over the REAL store and catalogue:
//! a record that cannot be summarised is read once per version — by the
//! store's walk and by the strict catalogue alike — named in exactly one
//! diagnostic per query, and re-read the moment it changes. Only a verdict on
//! bytes that were read is remembered, never an I/O failure (R2-H1); and only
//! in memory (R2-H2): the index carries no rejection and none is taken from
//! it, so each process reads a corrupt record once for itself.
#![cfg(feature = "test-support")]
use quecto::application::sessions::ports::SessionStore;
use quecto::application::sessions::ports::session_home::{
    SessionHomeCatalogue, SessionMetadataSnapshot,
};
use quecto::domain::message::Message;
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_home_catalogue::FileSessionHomeCatalogue;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

struct Process {
    store: Arc<FileSessionStore>,
    catalogue: FileSessionHomeCatalogue,
}

impl Process {
    fn over(layout: &FlatSessionLayout) -> Self {
        let store = Arc::new(FileSessionStore::new(layout.clone()));
        let catalogue = FileSessionHomeCatalogue::with_store(store.clone());
        Self { store, catalogue }
    }
    async fn query(&self) -> SessionMetadataSnapshot {
        self.catalogue.metadata().await.expect("metadata")
    }
    fn reads(&self) -> (usize, usize) {
        (
            self.catalogue.transcript_reads(),
            self.store.summary_transcript_reads(),
        )
    }
}

fn identity(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

async fn save(process: &Process, key: &str, title: &str) {
    let mut session = Session::new(identity(key));
    session.messages.push(Message::user(title));
    process.store.save(&session).await.unwrap();
    process.store.release(&identity(key));
}

fn named(snapshot: &SessionMetadataSnapshot, file: &str) -> usize {
    let lines = snapshot.diagnostics.iter();
    lines.filter(|d| d.starts_with(file)).count()
}

fn keys(snapshot: &SessionMetadataSnapshot) -> Vec<&str> {
    let mut keys: Vec<_> = snapshot
        .records
        .iter()
        .map(|r| r.summary.key.as_str())
        .collect();
    keys.sort_unstable();
    keys
}

const ROT: &[u8] = br#"{"key":"chat-rot","messages":[{"role":"user","content":"AAAA"#;

#[tokio::test]
async fn a_rejected_record_is_read_once_per_version_and_again_when_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let process = Process::over(&layout);
    save(&process, "chat-good", "a good title").await;
    let rot = layout.session_file(&identity("chat-rot"));
    std::fs::write(&rot, ROT).unwrap();
    let first = process.query().await;
    assert_eq!(
        (keys(&first), named(&first, "chat-rot.json")),
        (vec!["chat-good"], 1)
    );
    let reads = process.reads();
    assert_eq!(reads, (2, 2), "each half reads each record once");
    for _ in 0..3 {
        let warm = process.query().await;
        assert_eq!(named(&warm, "chat-rot.json"), 1, "{:?}", warm.diagnostics);
    }
    assert_eq!(
        process.reads(),
        reads,
        "an unchanged failure is never re-read"
    );
    // Repaired in place: a new stamp, so both halves read it again, once.
    let mut repaired = ROT.to_vec();
    repaired.extend_from_slice(b"\"}]}");
    std::fs::write(&rot, repaired).unwrap();
    let healed = process.query().await;
    assert_eq!(keys(&healed), ["chat-good", "chat-rot"]);
    assert_eq!(
        named(&healed, "chat-rot.json"),
        0,
        "{:?}",
        healed.diagnostics
    );
    assert_eq!(process.reads(), (3, 3));
    let index = std::fs::read_to_string(layout.home_catalogue_file()).unwrap();
    assert!(
        !index.contains("rejected"),
        "no rejection outlives its repair"
    );
    // Broken again: the rejection is of the new version, cached in turn.
    std::fs::write(&rot, &ROT[..ROT.len() - 2]).unwrap();
    let _ = process.query().await;
    let again = process.query().await;
    assert_eq!(
        (keys(&again), named(&again, "chat-rot.json")),
        (vec!["chat-good"], 1)
    );
    assert_eq!(process.reads(), (4, 4));
}

#[tokio::test]
async fn a_new_process_reads_a_rejected_record_once_for_itself_and_persists_no_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let writer = Process::over(&layout);
    save(&writer, "chat-good", "a good title").await;
    let rot = layout.session_file(&identity("chat-rot"));
    std::fs::write(&rot, ROT).unwrap();
    let _ = writer.query().await;
    let index = std::fs::read_to_string(layout.home_catalogue_file()).unwrap();
    assert!(!index.contains("rot"), "no verdict is persisted: {index}");
    let cold = Process::over(&layout);
    for _ in 0..3 {
        let seen = cold.query().await;
        assert_eq!((named(&seen, "chat-rot.json"), cold.reads()), (1, (1, 1)));
    }
    // Deleting the index is a complete recovery and costs no re-read of a
    // verdict this process reached itself on the bytes still on disk.
    std::fs::remove_file(layout.home_catalogue_file()).unwrap();
    let seen = cold.query().await;
    assert_eq!(
        (keys(&seen), named(&seen, "chat-rot.json")),
        (vec!["chat-good"], 1)
    );
    assert_eq!(
        cold.reads(),
        (2, 1),
        "only the good record is validated again"
    );
}

#[tokio::test]
async fn a_doctored_rejection_names_no_file_outside_the_layout_and_no_other_record() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let writer = Process::over(&layout);
    save(&writer, "chat-good", "a good title").await;
    save(&writer, "chat-other", "another title").await;
    let _ = writer.query().await;
    let path = layout.home_catalogue_file();
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let stamp = index["records"]["chat-good"]["stamp"].clone();
    let short = serde_json::json!([1, 2, 3]);
    index["rejected"] = serde_json::json!({
        // A path, not a record name; a malformed stamp; a stale stamp; and a
        // summary recorded beside a file it does not belong to.
        "../chat-good.json": {"stamp": stamp, "reason": "doctored", "unlisted": true},
        "chat-good.json": {"stamp": short, "reason": "doctored", "unlisted": true},
        "chat-other.json": {"stamp": stamp, "reason": "doctored",
            "listed": ["chat-good", "forged title", 9]},
        "notes.txt": {"stamp": stamp, "reason": "doctored", "unlisted": true},
    });
    std::fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
    let cold = Process::over(&layout);
    let seen = cold.query().await;
    assert_eq!(keys(&seen), ["chat-good", "chat-other"]);
    let titles: Vec<_> = seen
        .records
        .iter()
        .map(|r| r.summary.title.as_str())
        .collect();
    assert!(!titles.contains(&"forged title"), "{titles:?}");
    assert!(seen.diagnostics.is_empty(), "{:?}", seen.diagnostics);
    // The next publication carries none of it.
    let _ = cold.query().await;
    let index = std::fs::read_to_string(&path).unwrap();
    assert!(!index.contains("doctored"), "{index}");
}

/// R2-H1: an I/O failure says nothing about a record's content. Both halves
/// fail to read a VALID record during one query (the file, and so its stamp,
/// untouched); the next query reads it again and the row is back.
#[tokio::test]
async fn a_transient_read_failure_is_reported_once_and_never_remembered() {
    use quecto::infrastructure::persistence::session_record_read::fail_next_reads;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let process = Process::over(&layout);
    save(&process, "chat-good", "a good title").await;
    save(&process, "chat-flaky", "a flaky read").await;
    let flaky = layout.session_file(&identity("chat-flaky"));
    // One failed read by the walk, one by the strict catalogue.
    fail_next_reads(&flaky, 2);
    let starved = process.query().await;
    assert_eq!(
        (keys(&starved), named(&starved, "chat-flaky.json")),
        (vec!["chat-good"], 1),
        "{:?}",
        starved.diagnostics
    );
    let index = std::fs::read_to_string(layout.home_catalogue_file()).unwrap();
    assert!(!index.contains("chat-flaky"), "no persisted trace: {index}");
    let healed = process.query().await;
    assert_eq!(keys(&healed), ["chat-flaky", "chat-good"]);
    assert!(healed.diagnostics.is_empty(), "{:?}", healed.diagnostics);
    // Only the walk fails: the strict catalogue lists the record, the answer
    // cannot carry the row — and says so, by file name, for that answer only.
    let cold = Process::over(&layout);
    fail_next_reads(&flaky, 1);
    std::fs::remove_file(layout.home_catalogue_file()).unwrap();
    let walk_starved = cold.query().await;
    assert_eq!(
        (keys(&walk_starved), named(&walk_starved, "chat-flaky.json")),
        (vec!["chat-good"], 1),
        "{:?}",
        walk_starved.diagnostics
    );
    let healed = cold.query().await;
    assert_eq!(keys(&healed), ["chat-flaky", "chat-good"]);
    assert!(healed.diagnostics.is_empty(), "{:?}", healed.diagnostics);
}

/// R2-H2: the derived index never decides what is hidden. A rejection carrying
/// the CORRECT stamp of a VALID record — doctored, or written by another
/// harness version — hides nothing, names nothing, and is not written back.
#[tokio::test]
async fn a_persisted_rejection_at_the_correct_stamp_of_a_valid_record_hides_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let writer = Process::over(&layout);
    save(&writer, "chat-good", "a good title").await;
    save(&writer, "chat-two", "another title").await;
    let _ = writer.query().await;
    let path = layout.home_catalogue_file();
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let moved = index["records"]
        .as_object_mut()
        .unwrap()
        .remove("chat-good")
        .unwrap();
    index["rejected"] = serde_json::json!({
        "chat-good.json": {"stamp": moved["stamp"], "reason": "doctored", "unlisted": true},
    });
    std::fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
    let cold = Process::over(&layout);
    let seen = cold.query().await;
    assert_eq!(keys(&seen), ["chat-good", "chat-two"]);
    assert!(seen.diagnostics.is_empty(), "{:?}", seen.diagnostics);
    assert!(!seen.rebuilt, "a legacy field is no corruption");
    let listing = Process::over(&layout).catalogue.list().unwrap();
    assert_eq!(listing.entries.len(), 2, "{listing:?}");
    let index = std::fs::read_to_string(&path).unwrap();
    assert!(!index.contains("rejected"), "{index}");
}
