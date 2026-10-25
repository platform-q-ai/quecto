use super::fixture::*;
use crate::project_board_history_fixture::{Fixture, writer};
use quecto::application::project_board::ports::history::HistoryAlgorithm;
use quecto::application::project_board::ports::{backup::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::{
    SqliteProjectBoardStore, backup::SqliteBoardBackup,
};
use std::time::{Duration, SystemTime};

#[test]
fn r1_backup_wal_snapshot_restores_committed_history() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    // The rolled-back payload is distinguishable from both committed objects.
    f.keeper.execute_batch("BEGIN; INSERT INTO objects(oid,kind,bytes) VALUES ('rollback-only','blob',X'726f6c6c6261636b'); ROLLBACK;").unwrap();
    let before = logical(&f.path);
    assert!(!f.final_path().exists());
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    let report = successful(result);
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].published.as_ref(), Some(&f.final_path()));
    // A restore must have no route to the original WAL or source pathname.
    let final_path = f.final_path();
    let source = f.root.clone();
    drop(f.store);
    drop(f.keeper);
    std::fs::rename(&source, f.dir.path().join("source-unavailable")).unwrap();
    integrity(&final_path);
    assert_eq!(
        logical(&final_path),
        before,
        "all seven B3 tables and exact raw BLOBs restore independently"
    );
    let restored = SqliteProjectBoardStore::open(&final_path, writer()).unwrap();
    assert_eq!(restored.read().unwrap().head, BoardHead("1:2".into()));
}

#[test]
fn r1_imported_history_and_provenance_restore_byte_exactly() {
    let _deadline = ProbeDeadline::new();
    let f = Fixture::new(HistoryAlgorithm::Sha1, 12);
    f.verify();
    let root = f.directory.path().join("canonical-boards");
    std::fs::create_dir(&root).unwrap();
    let database = root.join(format!("{A}.sqlite"));
    crate::project_board_history_fixture::import_handles(
        &f.repo,
        &database,
        &f.destination,
        quecto::domain::project_board::value_objects::telemetry::BoardId::of_hex(A).unwrap(),
    )
    .import
    .execute()
    .unwrap();
    let keeper = rusqlite::Connection::open(&database).unwrap();
    keeper
        .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let store = SqliteProjectBoardStore::open(&database, writer()).unwrap();
    store
        .fixture_put_raw("tasks/native-tail.md", b"synthetic imported continuation")
        .unwrap();
    assert!(
        std::fs::metadata(database.with_extension("sqlite-wal"))
            .unwrap()
            .len()
            > 32
    );
    let before = logical(&database);
    let to = f.directory.path().join("backup");
    std::fs::create_dir(&to).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let request = BackupRequest {
        board: Some(A.into()),
        to: to.clone(),
        at: quecto::domain::project_board::value_objects::timestamp::Timestamp::parse(
            "2026-10-25T12:00:00Z",
        )
        .unwrap(),
        limits: short_limits(),
    };
    let adapter = Reached {
        inner: SqliteBoardBackup::new(database.parent().unwrap().into()),
        calls: Default::default(),
    };
    let result = adapter.backup(&request);
    adapter.assert_called();
    successful(result);
    drop(store);
    drop(keeper);
    std::fs::rename(&database, database.with_extension("unavailable")).unwrap();
    let snapshot = to.join(format!("{A}-20261025T120000Z.sqlite"));
    integrity(&snapshot);
    assert_eq!(
        logical(&snapshot),
        before,
        "original Git headers, object identities and imported metadata survive DB-wide backup"
    );
}

pub(super) fn old_snapshot(
    to: &std::path::Path,
    board: &str,
    day: u8,
    hour: u8,
) -> std::path::PathBuf {
    let path = to.join(format!("{board}-202610{day:02}T{hour:02}0000Z.sqlite"));
    let store = SqliteProjectBoardStore::open(&path, writer()).unwrap();
    store.bootstrap().unwrap();
    drop(store);
    integrity(&path);
    path
}

#[test]
fn r3_latest_fourteen_utc_days_per_board() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    // Insert in reverse date order and deliberately invert mtimes.
    for day in (9..=24).rev() {
        for hour in [8, 9] {
            let path = old_snapshot(&f.to, A, day, hour);
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_times(
                std::fs::FileTimes::new().set_modified(
                    SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(25 - day)),
                ),
            )
            .unwrap();
        }
    }
    let other = old_snapshot(&f.to, B, 1, 8);
    let other_bytes = std::fs::read(&other).unwrap();
    let unknown = f.to.join(format!("{A}-unknown.sqlite"));
    std::fs::write(&unknown, b"foreign content").unwrap();
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    successful(result);
    let held = names(&f.to);
    let actual: Vec<_> = held
        .iter()
        .filter(|n| {
            n.strip_prefix(&format!("{A}-"))
                .and_then(|s| s.strip_suffix(".sqlite"))
                .is_some_and(|s| {
                    s.len() == 16
                        && s.is_ascii()
                        && s.as_bytes()[8] == b'T'
                        && s.as_bytes()[15] == b'Z'
                        && quecto::domain::project_board::value_objects::timestamp::Timestamp::parse(
                            &format!("{}-{}-{}T{}:{}:{}Z", &s[0..4], &s[4..6], &s[6..8], &s[9..11], &s[11..13], &s[13..15]),
                        ).is_ok()
                })
        })
        .cloned()
        .collect();
    let mut expected: Vec<_> = (12..=24)
        .map(|day| format!("{A}-202610{day:02}T090000Z.sqlite"))
        .collect();
    expected.push(format!("{A}-20261025T120000Z.sqlite"));
    assert_eq!(
        actual, expected,
        "latest representative of newest fourteen represented UTC dates only"
    );
    assert_eq!(std::fs::read(other).unwrap(), other_bytes);
    assert_eq!(std::fs::read(unknown).unwrap(), b"foreign content");
}

#[test]
fn r4_collision_preserves_old_healthy_and_foreign_partials() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let old = old_snapshot(&f.to, A, 24, 9);
    let old_bytes = std::fs::read(&old).unwrap();
    std::fs::write(f.final_path(), b"occupied, never replace").unwrap();
    let foreign = f.to.join(".foreign.partial");
    std::fs::write(&foreign, b"not owned by this invocation").unwrap();
    let before = names(&f.to);
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(std::fs::read(old).unwrap(), old_bytes);
    assert_eq!(
        std::fs::read(f.final_path()).unwrap(),
        b"occupied, never replace"
    );
    assert_eq!(
        std::fs::read(foreign).unwrap(),
        b"not owned by this invocation"
    );
    assert_eq!(
        names(&f.to),
        before,
        "no owned partial or sidecar leaks on collision"
    );
    error(result, BackupError::Collision);
}

#[test]
fn r4_unknown_prior_content_is_never_pruned() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let unknown = f.to.join(format!("{A}-20200101T000000Z.sqlite"));
    std::fs::write(&unknown, b"not SQLite despite canonical old name").unwrap();
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(
        std::fs::read(unknown).unwrap(),
        b"not SQLite despite canonical old name"
    );
    // Admission must refuse at the schema stage rather than report retention complete.
    error(result, BackupError::UnsupportedSchema);
}

#[test]
fn r4_real_source_contention_is_bounded_and_preserves_healthy_snapshot() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let request = f.request();
    let old = old_snapshot(&f.to, A, 24, 9);
    let old_bytes = std::fs::read(&old).unwrap();
    let root = f.root.clone();
    let path = f.path.clone();
    let to = f.to.clone();
    let old_names = names(&to);
    drop(f.store);
    drop(f.keeper);
    let lock = rusqlite::Connection::open(&path).unwrap();
    lock.execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE;")
        .unwrap();
    let adapter = SqliteBoardBackup::new(root);
    let start = std::time::Instant::now();
    let result = adapter.backup(&request);
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "shared injected board deadline bounds contention"
    );
    assert_eq!(std::fs::read(&old).unwrap(), old_bytes);
    integrity(&old);
    let after_names = names(&to);
    assert!(
        after_names
            .iter()
            .all(|n| old_names.contains(n) || n.ends_with(".lock")),
        "only previous entries or cooperative lock may remain"
    );
    assert!(
        old_names.iter().all(|n| after_names.contains(n)),
        "previous entries preserved"
    );
    error(result, BackupError::BusyDeadline);
}

#[test]
fn r4_integrity_check_requires_exactly_one_ok_before_publication() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let old = old_snapshot(&f.to, A, 24, 9);
    let old_bytes = std::fs::read(&old).unwrap();
    // Real SQLite integrity fault, not a fake success/failure result: an
    // unreachable row violates the existing schema CHECK but leaves the board readable.
    f.keeper.execute_batch("PRAGMA ignore_check_constraints=ON; INSERT INTO objects(oid,kind,bytes) VALUES ('bad-integrity','invalid',X'00'); PRAGMA ignore_check_constraints=OFF;").unwrap();
    let check: String = f
        .keeper
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert!(check.contains("CHECK constraint failed"));
    println!("real integrity fault confirmed: {check}");
    let old_names = names(&f.to);
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert!(!f.final_path().exists());
    assert_eq!(std::fs::read(old).unwrap(), old_bytes);
    let after_names = names(&f.to);
    assert!(
        after_names
            .iter()
            .all(|n| old_names.contains(n) || n.ends_with(".lock")),
        "only previous entries or cooperative lock may remain"
    );
    assert!(
        old_names.iter().all(|n| after_names.contains(n)),
        "previous entries preserved"
    );
    error(result, BackupError::IntegrityFailed);
}
