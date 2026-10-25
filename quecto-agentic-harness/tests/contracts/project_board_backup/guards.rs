use super::fixture::*;
use crate::project_board_history_fixture::writer;
use quecto::application::project_board::ports::{ProjectBoardStore, backup::*};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

#[test]
fn parent_defaults_are_independently_literal_pinned() {
    let limits = BackupLimits::default();
    assert_eq!(limits.copy, Duration::from_secs(120));
    assert_eq!(limits.busy, Duration::from_millis(250));
    assert_eq!(limits.board, Duration::from_secs(300));
    assert_eq!(limits.invocation, Duration::from_secs(1200));
    assert_eq!(
        (limits.entries, limits.boards, limits.candidates),
        (4096, 256, 256)
    );
}
fn corrupt(sql: &str, expected: BackupError) {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    f.keeper.execute_batch(sql).unwrap();
    let before = logical(&f.path);
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(
        logical(&f.path),
        before,
        "admission cannot repair or initialize source"
    );
    assert!(
        names(&f.to).iter().all(|n| n.ends_with(".lock")),
        "no publication or leaked partial on admission refusal"
    );
    error(result, expected);
}
#[test]
fn r5_missing_metadata_is_not_native_null() {
    corrupt("DELETE FROM history_meta", BackupError::IdentityMismatch);
}
#[test]
fn r5_populated_wrong_identity_refuses() {
    corrupt(
        "UPDATE history_meta SET board_id='fedcba9876543210'",
        BackupError::IdentityMismatch,
    );
}
#[test]
fn r5_native_invalid_provenance_refuses() {
    corrupt(
        "UPDATE history_meta SET imported_count=1",
        BackupError::IdentityMismatch,
    );
}
#[test]
fn r5_missing_state_is_no_board() {
    corrupt("DELETE FROM state", BackupError::NoBoard);
}
#[test]
fn r5_unknown_schema_is_not_migrated() {
    corrupt("PRAGMA user_version=900", BackupError::UnsupportedSchema);
}
#[test]
fn r5_all_canonical_boards_sorted_without_cwd_inference() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let second =
        SqliteProjectBoardStore::open(&f.root.join(format!("{B}.sqlite")), writer()).unwrap();
    second.bootstrap().unwrap();
    std::fs::write(f.root.join("foreign.sqlite"), b"ignored").unwrap();
    let adapter = f.adapter();
    let mut request = f.request();
    request.board = None;
    let result = adapter.backup(&request);
    adapter.assert_called();
    let report = successful(result);
    assert_eq!(
        report
            .outcomes
            .iter()
            .map(|o| o.board.as_str())
            .collect::<Vec<_>>(),
        [A, B]
    );
    assert_eq!(
        std::fs::read(f.root.join("foreign.sqlite")).unwrap(),
        b"ignored"
    );
}
#[test]
fn r5_missing_source_root_is_never_created() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let missing = f.dir.path().join("absent");
    let adapter =
        quecto::infrastructure::persistence::sqlite::project_board::backup::SqliteBoardBackup::new(
            missing.clone(),
        );
    let mut request = f.request();
    request.board = None;
    let result = adapter.backup(&request);
    assert!(!missing.exists());
    assert!(names(&f.to).is_empty());
    error(result, BackupError::NoTargets);
}
#[test]
fn r5_source_overlap_refuses_without_changing_database() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let before = logical(&f.path);
    let adapter = f.adapter();
    let mut request = f.request();
    request.to = f.root.clone();
    let result = adapter.backup(&request);
    adapter.assert_called();
    assert_eq!(logical(&f.path), before);
    error(result, BackupError::UnsafePath);
}
#[cfg(unix)]
#[test]
fn r5_legitimate_existing_parent_alias_is_allowed() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let alias = f.dir.path().join("parent-alias");
    std::os::unix::fs::symlink(f.dir.path(), &alias).unwrap();
    let adapter = f.adapter();
    let mut request = f.request();
    request.to = alias.join("snapshots");
    let result = adapter.backup(&request);
    adapter.assert_called();
    successful(result);
    integrity(&f.final_path());
}
#[cfg(unix)]
#[test]
fn r5_actual_canonical_source_symlink_is_rejected() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let target = f.root.join("real-source.sqlite");
    std::fs::rename(&f.path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &f.path).unwrap();
    let before = std::fs::read(&target).unwrap();
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(std::fs::read(target).unwrap(), before);
    assert!(f.path.is_symlink());
    assert!(names(&f.to).is_empty());
    error(result, BackupError::UnsafePath);
}
#[test]
fn r5_actual_source_hardlink_is_rejected() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    std::fs::hard_link(&f.path, f.dir.path().join("alias.sqlite")).unwrap();
    let before = logical(&f.path);
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(logical(&f.path), before);
    assert!(names(&f.to).is_empty());
    error(result, BackupError::UnsafePath);
}
#[test]
fn r5_future_existing_timestamp_refuses_without_pruning() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let future = f.to.join(format!("{A}-20261026T000000Z.sqlite"));
    let store = SqliteProjectBoardStore::open(&future, writer()).unwrap();
    store.bootstrap().unwrap();
    drop(store);
    let before = std::fs::read(&future).unwrap();
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(std::fs::read(future).unwrap(), before);
    assert!(!f.final_path().exists());
    error(result, BackupError::ClockRegression);
}
fn bound(kind: &str) {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    match kind {
        "entries" => {
            for n in 0..4097 {
                std::fs::write(f.root.join(format!("foreign-{n}")), b"").unwrap();
            }
        }
        "boards" => {
            for n in 0..257 {
                std::fs::write(f.root.join(format!("{n:016x}.sqlite")), b"").unwrap();
            }
        }
        "candidates" => {
            for n in 0..257 {
                std::fs::write(
                    f.to.join(format!(
                        "{A}-20261024T{:02}{:02}{:02}Z.sqlite",
                        n / 3600,
                        (n / 60) % 60,
                        n % 60
                    )),
                    b"unknown preserve",
                )
                .unwrap();
            }
        }
        _ => unreachable!("test allowlist"),
    }
    let old = names(&f.to);
    let adapter = f.adapter();
    let mut request = f.request();
    request.board = None;
    let result = adapter.backup(&request);
    adapter.assert_called();
    assert_eq!(
        names(&f.to),
        old,
        "resource excess cannot truncate or prune"
    );
    error(result, BackupError::ResourceLimit);
}
#[test]
fn r5_directory_entry_limit_plus_one_is_explicit() {
    bound("entries");
}
#[test]
fn r5_board_limit_plus_one_is_explicit() {
    bound("boards");
}
#[test]
fn r5_candidate_limit_plus_one_is_explicit() {
    bound("candidates");
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new("timeout")
        .current_dir(home)
        .args(["--signal=KILL", "20", env!("CARGO_BIN_EXE_quecto")])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("QUECTO_BASE_DIR", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}
#[test]
fn r9_backup_help_preserves_delivered_action_usage() {
    let home = tempfile::tempdir().unwrap();
    let output = cli(home.path(), &["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for usage in [
        "board-server serve",
        "board-server token",
        "board-server import --repo <source> --board-id <16-hex> [--remote <name>]",
        "board-server export",
    ] {
        assert!(help.contains(usage), "preserve {usage}");
    }
    {
        let usage = "board-server backup --to <dir> [--board-id <16-hex>]";
        assert!(
            help.contains(usage),
            "positive offline maintenance help missing: {usage}"
        );
    }
}
#[test]
fn r9_backup_allowlist_refuses_before_io_control() {
    for args in [
        vec!["board-server", "backup"],
        vec!["board-server", "backup", "--to", "out", "--to", "other"],
        vec![
            "board-server",
            "backup",
            "--to",
            "out",
            "--source",
            "arbitrary",
        ],
        vec![
            "board-server",
            "backup",
            "--to",
            "out",
            "--board-id",
            "../escape",
        ],
    ] {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("owned-sentinel"),
            b"unchanged scratch state",
        )
        .unwrap();
        let before = super::admission::inventory(home.path());
        let output = cli(home.path(), &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(!home.path().join("data/quecto/boards").exists());
        assert!(!home.path().join("out").exists());
        assert_eq!(
            super::admission::inventory(home.path()),
            before,
            "no relative destination, token, config, state, or listener artifacts anywhere in isolated HOME/cwd"
        );
    }
}

#[test]
fn r5_partial_multi_board_reports_preserve_completed_success() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let second_path = f.root.join(format!("{B}.sqlite"));
    let second = SqliteProjectBoardStore::open(&second_path, writer()).unwrap();
    second.bootstrap().unwrap();
    drop(second);
    let connection = rusqlite::Connection::open(second_path).unwrap();
    connection
        .execute_batch("DELETE FROM history_meta")
        .unwrap();
    let mut request = f.request();
    request.board = None;
    let adapter = f.adapter();
    let result = adapter.backup(&request);
    adapter.assert_called();
    assert!(
        result.is_ok(),
        "selected-board failures must remain in explicit aggregate report: {result:?}"
    );
    let report = result.unwrap();
    assert_eq!(
        report
            .outcomes
            .iter()
            .map(|o| o.board.as_str())
            .collect::<Vec<_>>(),
        [A, B]
    );
    assert_eq!(report.outcomes[0].published.as_ref(), Some(&f.final_path()));
    assert_eq!(report.outcomes[0].error, None);
    assert_eq!(report.outcomes[1].published, None);
    assert_eq!(
        report.outcomes[1].error,
        Some(BackupError::IdentityMismatch)
    );
    integrity(&f.final_path());
}

#[cfg(unix)]
#[test]
fn r5_final_candidate_symlink_never_follows_or_deletes_target() {
    let _deadline = ProbeDeadline::new();
    let f = Database::new();
    let target = f.dir.path().join("foreign-owned-data");
    std::fs::write(&target, b"preserve linked target").unwrap();
    std::os::unix::fs::symlink(&target, f.final_path()).unwrap();
    let adapter = f.adapter();
    let result = adapter.backup(&f.request());
    adapter.assert_called();
    assert_eq!(std::fs::read(target).unwrap(), b"preserve linked target");
    assert!(f.final_path().is_symlink());
    error(result, BackupError::UnsafePath);
}
