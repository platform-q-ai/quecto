//! Backups across the schema-5 upgrade (epic #2705 B11 review): a backup
//! reads schema 4 or 5 alike, so a snapshot taken before the upgrade is
//! still admitted (and pruned), and a schema-4 board still served by the
//! old binary while the new one is installed (between the binary's install
//! and the unit's restart) is still copied, as it is: a backup never
//! upgrades.
use super::{board_backup::old_snapshot, fixture::*};
use quecto::application::project_board::ports::backup::BoardBackup;
use rusqlite::Connection;
use std::path::Path;

/// Takes the board file at `path` back to schema 4, as the build before
/// B11's wrote it.
fn to_schema_4(path: &Path) {
    let c = Connection::open(path).unwrap();
    c.execute_batch("ALTER TABLE revisions DROP COLUMN chain; PRAGMA user_version=4;")
        .unwrap();
}

fn version(path: &Path) -> i64 {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    c.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_schema_4_snapshot_is_admitted_and_the_backup_publishes() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let prior = old_snapshot(&f.to, A, 24, 9);
    to_schema_4(&prior);
    integrity(&prior);
    assert_eq!(version(&prior), 4);
    let report = successful(f.adapter().backup(&f.request()));
    assert_eq!(report.outcomes.len(), 1);
    assert!(f.final_path().exists(), "the new snapshot is published");
    assert!(
        prior.exists(),
        "the prior snapshot is kept, as retention says"
    );
    assert_eq!(version(&prior), 4, "a backup never upgrades a snapshot");
}

#[test]
fn a_schema_4_source_is_copied_as_it_is() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    to_schema_4(&f.path);
    successful(f.adapter().backup(&f.request()));
    assert_eq!(version(&f.final_path()), 4, "copied as it is");
    assert_eq!(version(&f.path), 4, "a backup never upgrades its source");
    integrity(&f.final_path());
}
