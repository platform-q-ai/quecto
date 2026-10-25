//! Real empty-target admission and atomic consuming import store contract.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::{BoardHead, history::*};
#[test]
fn real_store_commits_complete_originals_then_refuses_nonempty_without_changes() {
    let f = Fixture::new(HistoryAlgorithm::Sha256, 12);
    let handles = f.b_handles();
    let store: &dyn BoardHistoryStore = handles.store.as_ref();
    let h = handles.source.capture().unwrap();
    let prepared = store.prepare_import(&h).unwrap();
    let proof = handles
        .export
        .execute_snapshot(&*prepared, true, None)
        .unwrap();
    assert_eq!(proof.git_head, h.source_head);
    assert_eq!(proof.revisions, 12);
    let report = prepared.commit().unwrap();
    assert_eq!(report.head, BoardHead("1:12".into()));
    assert_eq!(report.source_head, h.source_head);
    assert_eq!(
        store.snapshot().unwrap().algorithm().unwrap(),
        Some(h.algorithm)
    );
    let before = fingerprint(&f.database);
    assert!(matches!(
        store.prepare_import(&h),
        Err(HistoryError::Conflict(_))
    ));
    assert_eq!(fingerprint(&f.database), before);
}
