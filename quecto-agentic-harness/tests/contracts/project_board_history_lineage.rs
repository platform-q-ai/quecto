//! Additive native provenance regressions for D3 review finding 1.
use super::project_board_history_fixture::*;
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;

fn populated(fixture: &Fixture) -> SqliteProjectBoardStore {
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    store.bootstrap().unwrap();
    for title in ["Before promotion", "At promotion"] {
        let head = store.read().unwrap().head;
        assert!(matches!(
            store.write_tasks(&head, &[task("a", title)], "board: lineage"),
            Ok(BoardWrite::Written(_))
        ));
    }
    assert_eq!(
        fixture
            .epoch_store()
            .open_epoch(1, "first promotion", 1700000000)
            .unwrap(),
        BoardHead("2:3".into())
    );
    assert_eq!(
        fixture
            .epoch_store()
            .open_epoch(2, "same revision promotion", 1700000001)
            .unwrap(),
        BoardHead("3:3".into())
    );
    let head = store.read().unwrap().head;
    assert_eq!(
        store
            .write_tasks(
                &head,
                &[task("a", "After promotion")],
                "board: after promotion"
            )
            .unwrap(),
        BoardWrite::Written(BoardHead("3:4".into()))
    );
    store
}
fn refuses(sql: &str) {
    refuses_with_bound(sql, false);
}
fn refuses_with_bound(sql: &str, oversized: bool) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let _store = populated(&fixture);
    let handles = fixture.handles();
    let valid = handles
        .store
        .snapshot()
        .expect("healthy lineage before corruption");
    assert_eq!(valid.algorithm().unwrap(), None);
    drop(valid);
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    c.execute_batch(sql).unwrap();
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    c.close().unwrap();
    let before = fingerprint(&fixture.database);
    assert!(
        matches!(
            (oversized, handles.store.snapshot()),
            (false, Err(HistoryError::Malformed(_))) | (true, Err(HistoryError::Bounds(_)))
        ),
        "noncanonical/impossible stored lineage must refuse: {sql}"
    );
    assert!(
        matches!(
            (oversized, handles.export.execute(false, None)),
            (false, Err(HistoryError::Malformed(_))) | (true, Err(HistoryError::Bounds(_)))
        ),
        "corrupt lineage must not publish: {sql}"
    );
    assert_eq!(fingerprint(&fixture.database), before);
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    assert!(
        text(
            &fixture.destination,
            &["cat-file", "--batch-all-objects", "--batch-check"]
        )
        .is_empty()
    );
}
#[test]
fn native_stored_head_keys_require_canonical_sequence_and_epoch_interval() {
    for sql in [
        "UPDATE revisions SET head='garbage' WHERE revision=1",
        "UPDATE revisions SET head='01:1' WHERE revision=1",
        "UPDATE revisions SET head='1:01' WHERE revision=1",
        "UPDATE revisions SET head='1:999' WHERE revision=1",
        "UPDATE revisions SET head='9:1' WHERE revision=1",
        "UPDATE revisions SET head='2:1' WHERE revision=1",
        "UPDATE revisions SET head='1:4' WHERE revision=4; UPDATE state SET head='1:4'",
    ] {
        refuses(sql);
    }
}
#[test]
fn native_epoch_coverage_requires_bootstrap_revision_one() {
    refuses("UPDATE epochs SET from_revision=2 WHERE epoch=1");
}
#[test]
fn native_same_revision_promotions_preserve_historical_heads_and_export() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    let _store = populated(&fixture);
    let handles = fixture.handles();
    let view = handles.store.snapshot().unwrap();
    assert_eq!(view.algorithm().unwrap(), None);
    assert_eq!(view.frontier().unwrap(), BoardHead("3:4".into()));
    assert_eq!(view.next_revision(2).unwrap().unwrap().sequence, 3);
    let before = fingerprint(&fixture.database);
    let report = handles.export.execute(false, None).unwrap();
    assert_eq!(
        (report.revisions, report.frontier),
        (4, BoardHead("3:4".into()))
    );
    assert_eq!(
        text(&fixture.destination, &["rev-list", "--count", TARGET]),
        "4"
    );
    assert_eq!(fingerprint(&fixture.database), before);
}

#[test]
fn native_oversized_historical_head_is_refused_before_key_parsing() {
    // Finite 128KiB digit prefix in author-owned scratch, far beyond the
    // canonical 39-byte ceiling; the healthy state/tip key is unchanged.
    refuses_with_bound(
        "UPDATE revisions SET head=replace(hex(zeroblob(65536)),'0','9')||':1' WHERE revision=1",
        true,
    );
}
