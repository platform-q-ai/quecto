//! Shared BoardBackup success contract, exercised on the real SQLite adapter by
//! project_board_b13_lite's WAL, writer, retention and selection fixtures.
use quecto::application::project_board::ports::backup::{BackupError, BackupReport};

pub fn successful(result: Result<BackupReport, BackupError>) -> BackupReport {
    assert!(
        result.is_ok(),
        "verified online snapshot must publish, not return missing capability: {result:?}"
    );
    let report = result.unwrap();
    assert!(
        report
            .outcomes
            .iter()
            .all(|o| o.error.is_none() && o.published.is_some()),
        "every selected board has truthful completed publication: {report:?}"
    );
    report
}
