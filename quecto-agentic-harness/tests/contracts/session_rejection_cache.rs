//! Contract of the rejection cache (R1-H1) over the REAL store and catalogue:
//! a record that cannot be summarised is read once per version — by the
//! store's walk and by the strict catalogue alike — named in exactly one
//! diagnostic per query, re-read the moment it changes, and remembered across
//! processes through the index, which is trusted only through stamps.
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
async fn a_new_process_reuses_a_persisted_rejection_only_at_its_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let writer = Process::over(&layout);
    save(&writer, "chat-good", "a good title").await;
    let rot = layout.session_file(&identity("chat-rot"));
    std::fs::write(&rot, ROT).unwrap();
    let _ = writer.query().await;
    let cold = Process::over(&layout);
    let seen = cold.query().await;
    assert_eq!((named(&seen, "chat-rot.json"), cold.reads()), (1, (0, 0)));
    // The file changes between processes: the persisted rejection is stale.
    std::fs::write(&rot, &ROT[..ROT.len() - 1]).unwrap();
    let next = Process::over(&layout);
    let seen = next.query().await;
    assert_eq!((named(&seen, "chat-rot.json"), next.reads()), (1, (1, 1)));
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
