//! Real uncommitted ownership and all-row abandonment contract.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::history::*;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
#[test]
fn real_prepared_view_exposes_every_original_and_drop_restores_empty_fingerprint() {
    let f = Fixture::new(HistoryAlgorithm::Sha1, 12);
    SqliteProjectBoardStore::open(&f.database, writer()).unwrap();
    let before = fingerprint(&f.database);
    let handles = f.b_handles();
    let h = handles.source.capture().unwrap();
    let prepared: Box<dyn PreparedHistoryImport> = handles.store.prepare_import(&h).unwrap();
    assert_eq!(prepared.algorithm().unwrap(), Some(h.algorithm));
    for o in &h.objects {
        assert!(prepared.is_original(&o.oid).unwrap());
        assert_eq!(prepared.object(&o.oid).unwrap(), *o);
    }
    let proof = handles
        .export
        .execute_snapshot(&*prepared, true, None)
        .unwrap();
    assert_eq!(proof.revisions, 12);
    assert_eq!(proof.git_head, h.source_head);
    assert_eq!(
        fingerprint(&f.database),
        before,
        "uncommitted rows invisible"
    );
    drop(prepared);
    assert_eq!(fingerprint(&f.database), before);
}
