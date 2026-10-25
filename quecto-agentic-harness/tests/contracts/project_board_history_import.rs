//! Real imported format admission and owned full-proof failure depth.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::{BoardHead, history::*};
use quecto::application::project_board::use_cases::ExportBoardHistory;
use quecto::composition::bootstrap::project_board_history::build_board_history_import_from_ports;
use quecto::infrastructure::persistence::project_board::history::GitBoardHistorySink;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct BeforeObjects(Box<dyn PreparedHistoryImport>);
impl BoardHistorySnapshot for BeforeObjects {
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        self.0.algorithm()
    }
    fn frontier(&self) -> HistoryResult<BoardHead> {
        self.0.frontier()
    }
    fn is_original(&self, _: &str) -> HistoryResult<bool> {
        panic!("format refusal must precede real membership access")
    }
    fn next_revision(&self, _: u64) -> HistoryResult<Option<HistoryRevision>> {
        panic!("format refusal must precede real revision access")
    }
    fn object(&self, _: &str) -> HistoryResult<HistoryObject> {
        panic!("format refusal must precede real object access")
    }
}
#[test]
fn real_imported_format_matches_and_mismatch_refuses_before_objects_or_refs_both_formats() {
    for algorithm in [HistoryAlgorithm::Sha1, HistoryAlgorithm::Sha256] {
        let f = Fixture::new(algorithm, 12);
        let handles = f.b_handles();
        let history = handles.source.capture().unwrap();
        let prepared = handles.store.prepare_import(&history).unwrap();
        assert_eq!(prepared.algorithm().unwrap(), Some(algorithm));
        assert_eq!(handles.sink.algorithm().unwrap(), Some(algorithm));
        let before = text(&f.destination, &["count-objects", "-v"]);
        let proof = handles
            .export
            .execute_snapshot(&*prepared, true, None)
            .unwrap();
        assert_eq!(proof.git_head, history.source_head);
        assert_eq!(proof.revisions, 12);
        assert_eq!(text(&f.destination, &["count-objects", "-v"]), before);
        assert!(text(&f.destination, &["for-each-ref"]).is_empty());
        drop(prepared);
        let held = fingerprint(&f.database);
        let wrong = f.repo.parent().unwrap().join("mismatched-format.git");
        let (name, other) = match algorithm {
            HistoryAlgorithm::Sha1 => ("--object-format=sha256", HistoryAlgorithm::Sha256),
            HistoryAlgorithm::Sha256 => ("--object-format=sha1", HistoryAlgorithm::Sha1),
        };
        git(
            &f.repo,
            &["init", "--bare", name, wrong.to_str().unwrap()],
            b"",
        );
        let sink = Arc::new(GitBoardHistorySink::new(
            wrong.clone(),
            "refs/heads/rollback".into(),
        ));
        assert_eq!(sink.algorithm().unwrap(), Some(other));
        let before = text(&wrong, &["count-objects", "-v"]);
        let probe = BeforeObjects(handles.store.prepare_import(&history).unwrap());
        let export = ExportBoardHistory::new(handles.store.clone(), sink);
        assert!(matches!(
            export.execute_snapshot(&probe, true, None),
            Err(HistoryError::Unsupported(_))
        ));
        assert_eq!(text(&wrong, &["count-objects", "-v"]), before);
        assert!(text(&wrong, &["for-each-ref"]).is_empty());
        drop(probe);
        assert_eq!(fingerprint(&f.database), held);
    }
}
struct Captured(LinearHistory);
impl BoardHistorySource for Captured {
    fn capture(&self) -> HistoryResult<LinearHistory> {
        Ok(self.0.clone())
    }
}
struct PreparedReached {
    inner: Arc<dyn BoardHistoryStore>,
    reached: Arc<AtomicBool>,
}
impl BoardHistoryRead for PreparedReached {
    fn snapshot(&self) -> HistoryResult<Box<dyn BoardHistorySnapshot>> {
        self.inner.snapshot()
    }
}
impl BoardHistoryStore for PreparedReached {
    fn prepare_import(&self, h: &LinearHistory) -> HistoryResult<Box<dyn PreparedHistoryImport>> {
        let prepared = self.inner.prepare_import(h)?;
        self.reached.store(true, Ordering::SeqCst);
        Ok(prepared)
    }
}
#[test]
fn real_corrupt_original_reaches_owned_prepared_then_full_export_failure_rolls_back() {
    let f = Fixture::new(HistoryAlgorithm::Sha256, 12);
    SqliteProjectBoardStore::open(&f.database, writer()).unwrap();
    let before = fingerprint(&f.database);
    let handles = f.b_handles();
    let mut history = handles.source.capture().unwrap();
    // Pin the opaque README payload by H0, never by capture/fixture OID order.
    // The sole Reader checks its root shape, but does not decode README bytes.
    let pinned = history.source_head.clone();
    assert_eq!(text(&f.repo, &["rev-parse", BOARD]), pinned);
    let target = text(&f.repo, &["rev-parse", &format!("{pinned}:README.md")]);
    assert_eq!(text(&f.repo, &["cat-file", "-t", &target]), "blob");
    let payload = git(&f.repo, &["cat-file", "blob", &target], b"");
    assert!(matches!(payload.as_slice(), [_, ..]));
    assert_ne!(
        target,
        text(&f.repo, &["rev-parse", &format!("{pinned}:format/board")])
    );
    let blob = history
        .objects
        .iter_mut()
        .find(|o| o.oid == target)
        .unwrap();
    assert_eq!(blob.kind, HistoryKind::Blob);
    assert!(blob.entries.is_empty());
    assert_eq!(blob.bytes, payload);
    let last = blob.bytes.len() - 1;
    blob.bytes[last] ^= 1;
    assert_eq!(blob.oid, target);
    assert_eq!(blob.bytes.len(), payload.len());
    assert_ne!(blob.bytes, payload);
    let reached = Arc::new(AtomicBool::new(false));
    let graph = build_board_history_import_from_ports(
        Arc::new(Captured(history)),
        Arc::new(PreparedReached {
            inner: handles.store,
            reached: reached.clone(),
        }),
        handles.sink,
    );
    let result = graph.import.execute();
    assert!(
        matches!(result, Err(HistoryError::Malformed(_))),
        "{result:?}"
    );
    assert!(
        reached.load(Ordering::SeqCst),
        "real rows must be prepared before application full-export proof fails, not an earlier admission shortcut: {result:?}"
    );
    assert_eq!(fingerprint(&f.database), before);
    assert!(text(&f.destination, &["for-each-ref"]).is_empty());
    assert_eq!(
        text(&f.destination, &["count-objects", "-v"]),
        "count: 0\nsize: 0\nin-pack: 0\npacks: 0\nsize-pack: 0\nprune-packable: 0\ngarbage: 0\nsize-garbage: 0"
    );
}
