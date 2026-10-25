use super::*;
#[test]
fn backup_system_copy_maps_actual_destination_full_to_publication_failure() {
    let source = Connection::open_in_memory().unwrap();
    source
        .execute_batch(
            "CREATE TABLE payload(bytes BLOB); INSERT INTO payload VALUES(zeroblob(1048576));",
        )
        .unwrap();
    let mut destination = Connection::open_in_memory().unwrap();
    destination
        .pragma_update(None, "max_page_count", 1)
        .unwrap();
    let pages: i64 = source
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    assert!(pages > 128);
    // Establish the real pinned dependency's actual FULL error independently.
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut destination).unwrap();
        let failure = backup.step(128).unwrap_err();
        assert_eq!(
            failure.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DiskFull)
        );
    }
    let mut destination = Connection::open_in_memory().unwrap();
    destination
        .pragma_update(None, "max_page_count", 1)
        .unwrap();
    assert_eq!(
        SystemBackupSqliteOps.copy(
            &source,
            &mut destination,
            Duration::from_secs(1),
            Duration::from_millis(10)
        ),
        Err(BackupError::PublishFailed)
    );
    assert_eq!(
        source
            .query_row("SELECT length(bytes) FROM payload", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1048576
    );
}
#[test]
fn backup_system_copy_zero_budget_is_bounded_before_first_step() {
    let source = Connection::open_in_memory().unwrap();
    let mut destination = Connection::open_in_memory().unwrap();
    assert_eq!(
        SystemBackupSqliteOps.copy(&source, &mut destination, Duration::ZERO, Duration::ZERO),
        Err(BackupError::BusyDeadline)
    );
}
