//! Real destination serializer/dry-run/lease contract for BoardHistorySink.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::history::*;

#[test]
fn real_sink_hashes_without_side_effects_then_publishes_only_the_captured_lease() {
    for algorithm in [HistoryAlgorithm::Sha1, HistoryAlgorithm::Sha256] {
        let fixture = Fixture::new(algorithm, 12);
        let handles = fixture.handles();
        let sink: &dyn BoardHistorySink = handles.sink.as_ref();
        let body = b"native sink contract\n";
        let expected = String::from_utf8(git(
            &fixture.destination,
            &["hash-object", "-t", "blob", "--stdin"],
            body,
        ))
        .unwrap()
        .trim()
        .to_owned();
        assert_eq!(
            sink.raw(HistoryKind::Blob, body, Some(&expected), true)
                .unwrap(),
            expected
        );
        let tree = sink.tree(&[], true).unwrap();
        let identity = writer();
        let metadata = HistoryCommit {
            tree: &tree,
            parent: None,
            writer: &identity,
            message: "board: sink contract",
            at: 1700000000,
        };
        let proposed = sink.commit(metadata, true).unwrap();
        sink.finish_dry_run().unwrap();
        assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
        assert!(
            text(
                &fixture.destination,
                &["cat-file", "--batch-all-objects", "--batch-check"]
            )
            .is_empty()
        );
        assert!(!fixture.database.exists());
        assert_eq!(
            sink.raw(HistoryKind::Blob, body, Some(&expected), false)
                .unwrap(),
            expected
        );
        assert_eq!(sink.tree(&[], false).unwrap(), tree);
        assert_eq!(sink.commit(metadata, false).unwrap(), proposed);
        sink.publish(None, &proposed).unwrap();
        // Written objects are stored, exactly, by the time the ref moves (#2748).
        assert_eq!(
            git(&fixture.destination, &["cat-file", "blob", &expected], b""),
            body
        );
        let loser = sink
            .commit(
                HistoryCommit {
                    parent: Some(&proposed),
                    message: "board: losing lease",
                    ..metadata
                },
                false,
            )
            .unwrap();
        assert!(matches!(
            sink.publish(None, &loser),
            Err(HistoryError::Conflict(_))
        ));
        assert_eq!(text(&fixture.destination, &["rev-parse", TARGET]), proposed);
        assert_eq!(
            text(&fixture.destination, &["rev-list", "--count", TARGET]),
            "1"
        );
        assert!(!fixture.database.exists());
    }
}

/// An export of more objects than one store batch takes (4,096) publishes
/// a head whose every reachable object is stored, and leaves no staging.
#[test]
fn an_export_across_store_batches_publishes_a_fully_connected_head() {
    use super::project_board_store::task;
    use quecto::application::project_board::ports::{BoardWrite, ProjectBoardStore};
    use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    store.bootstrap().unwrap();
    let mut head = store.read().unwrap().head;
    // Each revision adds a task blob, the tasks tree, a root tree and a commit.
    for index in 0..1100 {
        let title = format!("Batch boundary object {index}");
        head = match store.write_tasks(&head, &[task("a", &title)], "board: batches") {
            Ok(BoardWrite::Written(next)) => next,
            other => panic!("expected a valid write: {other:?}"),
        };
    }
    let report = fixture.handles().export.execute(false, None).unwrap();
    assert_eq!((report.revisions, report.frontier), (1101, head));
    let inventory = text(
        &fixture.destination,
        &["cat-file", "--batch-all-objects", "--batch-check"],
    );
    assert!(
        inventory.lines().count() > 4096,
        "the export crosses a store batch: {} objects",
        inventory.lines().count()
    );
    let reachable = text(
        &fixture.destination,
        &["rev-list", "--objects", "--missing=print", TARGET],
    );
    assert!(reachable.lines().count() > 4096);
    assert!(
        // A missing object prints as `?<id>`; a present one as its bare ID.
        reachable
            .lines()
            .all(|line| line.bytes().next().is_some_and(|b| b.is_ascii_hexdigit())),
        "every object the published head reaches is stored"
    );
    git(&fixture.destination, &["fsck", "--connectivity-only"], b"");
    let staging: Vec<_> = std::fs::read_dir(&fixture.destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("quecto-export-"))
        .collect();
    assert!(
        staging.is_empty(),
        "stored batches leave no staging: {staging:?}"
    );
}
