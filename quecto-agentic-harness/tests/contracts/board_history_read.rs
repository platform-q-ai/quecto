//! Real native adapter contract for the inward BoardHistoryRead port.
use super::project_board_history_fixture::*;
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;

#[test]
fn real_reader_refuses_missing_without_creation_and_pins_native_frontier() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let handles = fixture.handles();
    let reader: &dyn BoardHistoryRead = handles.store.as_ref();
    assert!(!fixture.database.exists());
    assert!(matches!(
        reader.snapshot(),
        Err(HistoryError::Unavailable(_))
    ));
    assert!(!fixture.database.exists());
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    store.bootstrap().unwrap();
    let held = reader.snapshot().unwrap();
    assert_eq!(held.algorithm().unwrap(), None);
    assert_eq!(held.frontier().unwrap(), BoardHead("1:1".into()));
    assert_eq!(
        store
            .write_tasks(
                &BoardHead("1:1".into()),
                &[task("a", "After pin")],
                "board: read contract"
            )
            .unwrap(),
        BoardWrite::Written(BoardHead("1:2".into()))
    );
    let before = fingerprint(&fixture.database);
    assert_eq!(held.frontier().unwrap(), BoardHead("1:1".into()));
    assert!(held.next_revision(1).unwrap().is_none());
    assert_eq!(
        reader.snapshot().unwrap().frontier().unwrap(),
        BoardHead("1:2".into())
    );
    assert_eq!(fingerprint(&fixture.database), before);
    // Compatibility is the same enum type, not a copied DTO or conversion.
    let qualified: BoardHistoryError = HistoryError::Unavailable("same enum".into());
    let compatibility: HistoryError = qualified;
    assert_eq!(
        compatibility,
        BoardHistoryError::Unavailable("same enum".into())
    );
}
