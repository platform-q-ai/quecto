//! Real captured cursor/object contract for BoardHistorySnapshot.
use super::project_board_history_fixture::*;
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;

#[test]
fn real_snapshot_keeps_contiguous_native_cursor_and_objects_through_write_and_epoch() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    store.bootstrap().unwrap();
    store
        .write_tasks(
            &store.read().unwrap().head,
            &[task("a", "Captured")],
            "board: snapshot contract",
        )
        .unwrap();
    let handles = fixture.handles();
    let captured = handles.store.snapshot().unwrap();
    let view: &dyn BoardHistorySnapshot = captured.as_ref();
    assert_eq!(view.algorithm().unwrap(), None);
    assert_eq!(view.frontier().unwrap(), BoardHead("1:2".into()));
    let first = view.next_revision(0).unwrap().unwrap();
    assert_eq!((first.sequence, first.parent), (1, None));
    assert_eq!(view.object(&first.root).unwrap().kind, HistoryKind::Tree);
    assert!(!view.is_original(&first.root).unwrap());
    let second = view.next_revision(1).unwrap().unwrap();
    assert_eq!((second.sequence, second.parent), (2, Some(1)));
    assert_eq!(second.writer, writer());
    assert_eq!(second.message, "board: snapshot contract");
    assert_eq!(view.object(&second.root).unwrap().kind, HistoryKind::Tree);
    assert!(second.at > 0);
    assert_eq!(
        fixture
            .epoch_store()
            .open_epoch(1, "contract promotion", 1700000000)
            .unwrap(),
        BoardHead("2:2".into())
    );
    assert_eq!(
        store
            .write_tasks(
                &BoardHead("2:2".into()),
                &[task("a", "Later")],
                "board: after snapshot"
            )
            .unwrap(),
        BoardWrite::Written(BoardHead("2:3".into()))
    );
    let before = fingerprint(&fixture.database);
    assert_eq!(view.frontier().unwrap(), BoardHead("1:2".into()));
    assert!(view.next_revision(2).unwrap().is_none());
    assert_eq!(view.next_revision(1).unwrap().unwrap(), second);
    assert_eq!(
        handles.store.snapshot().unwrap().frontier().unwrap(),
        BoardHead("2:3".into())
    );
    assert_eq!(fingerprint(&fixture.database), before);
}
