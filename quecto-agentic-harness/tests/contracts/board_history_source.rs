//! Real complete local adapter contract for BoardHistorySource.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::history::*;
#[test]
fn real_source_captures_original_ids_and_raw_objects_without_destination_writes() {
    for algorithm in [HistoryAlgorithm::Sha1, HistoryAlgorithm::Sha256] {
        let f = Fixture::new(algorithm, 12);
        let handles = f.b_handles();
        let source: &dyn BoardHistorySource = handles.source.as_ref();
        let refs = text(&f.repo, &["for-each-ref"]);
        let h = source.capture().unwrap();
        assert_eq!(h.algorithm, algorithm);
        assert_eq!(h.source_head, f.history.source_head);
        assert_eq!(h.source_ref, BOARD);
        assert_eq!(h.revisions.len(), 12);
        for (a, b) in h.revisions.iter().zip(&f.history.revisions) {
            assert_eq!(a.original, b.original);
            assert_eq!(a.root, b.root);
            assert_eq!(a.parent, b.parent);
        }
        for o in &f.history.objects {
            assert_eq!(h.objects.iter().find(|v| v.oid == o.oid), Some(o));
        }
        assert_eq!(text(&f.repo, &["for-each-ref"]), refs);
        assert!(text(&f.destination, &["for-each-ref"]).is_empty());
        assert!(!f.database.exists());
    }
}
