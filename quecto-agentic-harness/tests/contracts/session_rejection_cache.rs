//! Contract of the rejection cache (R1-H1) over the REAL store and catalogue:
//! a record that cannot be summarised is read once per version — one read
//! serving the store's walk and the strict catalogue alike (#2042) — named
//! in exactly one diagnostic per query, and re-read the moment it changes. Only a verdict on
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
    assert_eq!(
        reads,
        (2, 0),
        "one read per record serves both halves; the store's own walk never ran"
    );
    for _ in 0..3 {
        let warm = process.query().await;
        assert_eq!(named(&warm, "chat-rot.json"), 1, "{:?}", warm.diagnostics);
    }
    assert_eq!(
        process.reads(),
        reads,
        "an unchanged failure is never re-read"
    );
    // Repaired in place: a new stamp, so it is read again — once for both.
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
    assert_eq!(process.reads(), (3, 0));
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
    assert_eq!(process.reads(), (4, 0));
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
        assert_eq!((named(&seen, "chat-rot.json"), cold.reads()), (1, (1, 0)));
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
        (2, 0),
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
    // The one read both halves share fails: neither has a verdict.
    fail_next_reads(&flaky, 1);
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
    // A cold process rebuilding without an index: the same one failure,
    // named by file for that answer only, read again on the next.
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

/// #2042: the metadata query is ONE pass. Warm, the pass stamps each record
/// exactly once and reads nothing (publication re-stamps published rows
/// until slice B); a rebuild (cold process, index deleted) reads each record
/// exactly once — the walk's summary and the catalogue's strict identity come
/// from the same bytes — and the store's own walk is not run by the query.
#[tokio::test]
async fn the_pass_stamps_each_record_once_warm_and_a_rebuild_reads_each_once() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let process = Process::over(&layout);
    for n in 0..5 {
        save(&process, &format!("chat-{n}"), &format!("title {n}")).await;
    }
    std::fs::write(
        layout.session_file(&identity("chat-rot")),
        br#"{"key":"chat-rot","messages":[{"role":"user","content":"AAAA"#,
    )
    .unwrap();
    let records = 6;
    let seen = process.query().await;
    assert_eq!(
        keys(&seen),
        ["chat-0", "chat-1", "chat-2", "chat-3", "chat-4"]
    );
    assert_eq!(named(&seen, "chat-rot.json"), 1, "{:?}", seen.diagnostics);
    assert_eq!(
        process.reads(),
        (records, 0),
        "one read per record, none by the store's own walk"
    );
    assert_eq!(
        process.catalogue.record_stamps(),
        records * 2,
        "cold: a record is stamped before and after its one read"
    );
    let warm = process.query().await;
    assert_eq!(keys(&warm), keys(&seen));
    assert_eq!(process.reads(), (records, 0), "warm: nothing read");
    assert_eq!(
        process.catalogue.record_stamps(),
        records * 3,
        "warm: the pass stamped each record exactly once more"
    );
    // The store's own listing after the query answers from the same cache.
    let listed = process
        .store
        .list(&quecto::application::sessions::dto::SessionListQuery::All)
        .await
        .unwrap();
    assert_eq!(listed.len(), 5);
    assert_eq!(
        process.reads(),
        (records, 0),
        "the walk reuses the pass's verdicts"
    );
    // A rebuild: a new process with the index gone reads each record once.
    std::fs::remove_file(layout.home_catalogue_file()).unwrap();
    let rebuilt = Process::over(&layout);
    let seen = rebuilt.query().await;
    // A missing index is "never existed": built, not recovered — no report.
    assert!(!seen.rebuilt, "{seen:?}");
    assert_eq!(
        keys(&seen),
        ["chat-0", "chat-1", "chat-2", "chat-3", "chat-4"]
    );
    assert_eq!(
        rebuilt.reads(),
        (records, 0),
        "rebuild: each record read once"
    );
}

/// #2042: a record rewritten under the one read is a verdict for nobody —
/// named for that answer, remembered by neither half, read again next time.
#[tokio::test]
async fn a_record_rewritten_under_its_read_is_remembered_by_neither_half() {
    use quecto::infrastructure::persistence::session_record_read::rewrite_after_next_read;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let process = Process::over(&layout);
    save(&process, "chat-good", "a good title").await;
    save(&process, "chat-racy", "the old title").await;
    let racy = layout.session_file(&identity("chat-racy"));
    let mut newer = std::fs::read(&racy).unwrap();
    newer.extend_from_slice(b" ");
    rewrite_after_next_read(&racy, newer);
    let torn = process.query().await;
    assert_eq!(keys(&torn), ["chat-good"], "{:?}", torn.diagnostics);
    assert_eq!(named(&torn, "chat-racy.json"), 1, "{:?}", torn.diagnostics);
    assert_eq!(process.reads(), (2, 0));
    let settled = process.query().await;
    assert_eq!(keys(&settled), ["chat-good", "chat-racy"]);
    assert!(settled.diagnostics.is_empty(), "{:?}", settled.diagnostics);
    assert_eq!(
        process.reads(),
        (3, 0),
        "the torn read was no verdict: read again"
    );
    let _ = process.query().await;
    assert_eq!(process.reads(), (3, 0), "and remembered once stable");
}

/// #2042 slice B: what the caches remember is what the scan saw — a deleted
/// record leaves the projection and the rejections on the next query without
/// any `exists` sweep, and nothing is stamped twice: a warm query is exactly
/// one stamp per record, publication included.
#[tokio::test]
async fn a_deleted_record_leaves_both_caches_and_a_warm_query_stamps_each_record_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let process = Process::over(&layout);
    save(&process, "chat-good", "a good title").await;
    save(&process, "chat-gone", "soon gone").await;
    let rot = layout.session_file(&identity("chat-rot"));
    std::fs::write(&rot, ROT).unwrap();
    let seen = process.query().await;
    assert_eq!(keys(&seen), ["chat-gone", "chat-good"]);
    assert_eq!(
        process.catalogue.remembered_records(),
        3,
        "two projected, one rejected"
    );
    std::fs::remove_file(layout.session_file(&identity("chat-gone"))).unwrap();
    std::fs::remove_file(&rot).unwrap();
    let after = process.query().await;
    assert_eq!(keys(&after), ["chat-good"]);
    assert!(after.diagnostics.is_empty(), "{:?}", after.diagnostics);
    assert_eq!(
        process.catalogue.remembered_records(),
        1,
        "only what the scan saw"
    );
    let stamps = process.catalogue.record_stamps();
    let _ = process.query().await;
    assert_eq!(
        process.catalogue.record_stamps() - stamps,
        1,
        "one counted stamp per warm record (that publication takes none is pinned by the source ratchet)"
    );
}

/// #2042 slice B: a record larger than the cap is refused from its stamp —
/// never read — named once per answer, remembered by stamp like any
/// content verdict, and read once it is small again.
#[tokio::test]
async fn an_oversized_record_is_refused_from_its_stamp_and_never_read() {
    use quecto::infrastructure::persistence::session_record_read::MAX_RECORD_BYTES;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let process = Process::over(&layout);
    save(&process, "chat-good", "a good title").await;
    let huge = layout.session_file(&identity("chat-huge"));
    // Sparse: the size says everything, nothing is on disk.
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(MAX_RECORD_BYTES + 1)
        .unwrap();
    let seen = process.query().await;
    assert_eq!(keys(&seen), ["chat-good"]);
    assert_eq!(named(&seen, "chat-huge.json"), 1, "{:?}", seen.diagnostics);
    assert!(
        seen.diagnostics
            .iter()
            .any(|d| d.contains("record too large")),
        "{:?}",
        seen.diagnostics
    );
    assert_eq!(process.reads(), (1, 0), "the huge record was never read");
    let again = process.query().await;
    assert_eq!(named(&again, "chat-huge.json"), 1);
    assert_eq!(process.reads(), (1, 0), "remembered by stamp");
    // Repaired: a small valid record under the same name.
    save(&process, "chat-huge", "no longer huge").await;
    let healed = process.query().await;
    assert_eq!(keys(&healed), ["chat-good", "chat-huge"]);
    assert!(healed.diagnostics.is_empty(), "{:?}", healed.diagnostics);
    assert_eq!(process.reads(), (2, 0));
}
