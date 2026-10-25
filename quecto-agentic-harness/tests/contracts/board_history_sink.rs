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
        assert_eq!(
            git(&fixture.destination, &["cat-file", "blob", &expected], b""),
            body
        );
        assert_eq!(sink.tree(&[], false).unwrap(), tree);
        assert_eq!(sink.commit(metadata, false).unwrap(), proposed);
        sink.publish(None, &proposed).unwrap();
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
