//! Common-file/state regressions through existing runtime APIs, independent of resolver internals.
use quecto::{
    application::project_board::ports::{ProjectBoardStore, backup::*},
    domain::project_board::value_objects::timestamp::Timestamp,
    infrastructure::persistence::sqlite::project_board::{
        SqliteProjectBoardStore, backup::SqliteBoardBackup,
    },
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::mpsc,
    time::Duration,
};

const A: &str = "0123456789abcdef";
const B: &str = "fedcba9876543210";

// Same hard bound as the permanent backup probes; parent runner owns scratch cleanup.
struct Deadline(
    Option<mpsc::Sender<()>>,
    Option<std::thread::JoinHandle<()>>,
);
impl Deadline {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            if matches!(
                rx.recv_timeout(Duration::from_secs(45)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                unsafe { libc::_exit(124) }
            }
        });
        Self(Some(tx), Some(thread))
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        self.0.take().unwrap().send(()).ok();
        self.1.take().unwrap().join().unwrap();
    }
}
fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new("timeout")
        .args(["--signal=KILL", "20", env!("CARGO_BIN_EXE_quecto")])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("QUECTO_BASE_DIR", home.join("base"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .current_dir(home)
        .output()
        .unwrap()
}
fn inventory(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, rows: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                rows.push((relative, Vec::new()));
                visit(root, &path, rows);
            } else {
                rows.push((relative, std::fs::read(path).unwrap()));
            }
        }
    }
    let mut rows = Vec::new();
    visit(root, root, &mut rows);
    rows.sort();
    rows
}
struct Source {
    store: SqliteProjectBoardStore,
    keeper: rusqlite::Connection,
    path: PathBuf,
}
impl Source {
    fn new(path: PathBuf, binding: Option<&str>) -> Self {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let store =
            SqliteProjectBoardStore::open(&path, crate::project_board_history_fixture::writer())
                .unwrap();
        store.bootstrap().unwrap();
        let keeper = rusqlite::Connection::open(&path).unwrap();
        keeper
            .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        // Existing storage bootstrap owns this schema and native metadata row.
        // Simulate server ID binding by changing ONLY its nullable board_id field.
        assert_eq!(
            keeper
                .execute("UPDATE history_meta SET board_id=?1 WHERE id=1", [binding])
                .unwrap(),
            1
        );
        store
            .fixture_put_raw(
                "tasks/common-file.md",
                b"common file committed WAL sentinel",
            )
            .unwrap();
        assert!(
            std::fs::metadata(PathBuf::from(format!("{}-wal", path.display())))
                .unwrap()
                .len()
                > 32
        );
        let ro = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            ro.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        let meta: (Option<String>, String, Option<String>, Option<String>, Option<String>, i64) = ro.query_row("SELECT board_id,algorithm,source_remote,source_ref,source_head,imported_count FROM history_meta WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
        assert_eq!(
            meta,
            (
                binding.map(str::to_owned),
                "sha256".into(),
                None,
                None,
                None,
                0
            )
        );
        assert!(store.read().is_ok(), "real native store admits the fixture");
        println!(
            "valid native fixture {} binding={binding:?} integrity=ok",
            path.display()
        );
        Self {
            store,
            keeper,
            path,
        }
    }
}
fn assert_cli_restore(explicit: bool, bound: bool) {
    let _deadline = Deadline::new();
    let home = tempfile::tempdir().unwrap();
    // Contract path asserted independently, never derived by the production helper under test.
    let state = if explicit {
        home.path().join("custom-state")
    } else {
        home.path().join(format!("base/board-server/{A}"))
    };
    let source = Source::new(state.join("board.sqlite3"), bound.then_some(A));
    let before = crate::project_board_history_fixture::fingerprint(&source.path);
    let to = home.path().join("snapshots");
    let mut args = vec![
        "board-server",
        "backup",
        "--to",
        to.to_str().unwrap(),
        "--board-id",
        A,
    ];
    if explicit {
        args.extend(["--state", state.to_str().unwrap()]);
    }
    let result = cli(home.path(), &args);
    assert_eq!(
        result.status.code(),
        Some(0),
        "common-file backup must publish: stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["outcomes"].as_array().unwrap().len(), 1);
    let outcome = &report["outcomes"][0];
    assert_eq!(outcome["board"], A);
    assert_eq!(outcome["stage"], "complete");
    assert!(outcome["error"].is_null());
    let published = PathBuf::from(outcome["published"].as_str().unwrap());
    assert_eq!(
        crate::project_board_history_fixture::fingerprint(&source.path),
        before
    );
    drop(source.store);
    drop(source.keeper);
    std::fs::rename(&state, home.path().join("source-unavailable")).unwrap();
    assert_eq!(
        crate::project_board_history_fixture::fingerprint(&published),
        before
    );
    assert!(
        !home.path().join("data").exists(),
        "no legacy source or writable source creation"
    );
    assert!(
        !home.path().join("config").exists(),
        "no token/listener graph"
    );
}
#[test]
fn default_common_board_sqlite3_restores_native_null() {
    assert_cli_restore(false, false);
}
#[test]
fn absolute_state_board_sqlite3_restores_native_null() {
    assert_cli_restore(true, false);
}
#[test]
fn default_common_board_sqlite3_restores_matching_bound_native() {
    assert_cli_restore(false, true);
}
fn invalid_state(state: &str) {
    let _deadline = Deadline::new();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("sentinel"), b"preserve entire owned home").unwrap();
    let _source = Source::new(
        home.path()
            .join(format!("base/board-server/{A}/board.sqlite3")),
        None,
    );
    let to = home.path().join("snapshots");
    let before = inventory(home.path());
    let result = cli(
        home.path(),
        &[
            "board-server",
            "backup",
            "--to",
            to.to_str().unwrap(),
            "--board-id",
            A,
            "--state",
            state,
        ],
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(
        result.stdout.is_empty(),
        "parse refusal, not unrelated no_targets JSON"
    );
    let stderr = String::from_utf8(result.stderr).unwrap();
    let reason = stderr.lines().next().unwrap();
    assert!(
        [
            "backup: option value required",
            "backup: unsupported or repeated option",
            "backup: --state must be a non-empty absolute path",
        ]
        .contains(&reason),
        "explicit allowlisted parse rejection required, got {reason:?}"
    );
    assert_eq!(
        inventory(home.path()),
        before,
        "invalid state refuses before any side effect"
    );
}
#[test]
fn empty_state_refuses_without_side_effects() {
    invalid_state("");
}
#[test]
fn relative_state_refuses_without_side_effects() {
    invalid_state("relative-state");
}
fn native_admission(binding: Option<&str>, accepted: bool) {
    let _deadline = Deadline::new();
    let home = tempfile::tempdir().unwrap();
    // Existing adapter API takes an explicit source root. Isolate metadata admission
    // from CLI layout discovery, so a matching-bound failure cannot be a path error.
    let root = home.path().join("adapter-root");
    let source = Source::new(root.join(format!("{A}.sqlite")), binding);
    let before = crate::project_board_history_fixture::fingerprint(&source.path);
    let to = home.path().join("snapshots");
    let request = BackupRequest {
        board: Some(A.into()),
        to: to.clone(),
        at: Timestamp::parse("2026-10-25T12:00:00Z").unwrap(),
        limits: BackupLimits::default(),
    };
    let result = SqliteBoardBackup::new(root).backup(&request).unwrap();
    assert_eq!(result.outcomes.len(), 1);
    let outcome = &result.outcomes[0];
    if accepted {
        assert_eq!(outcome.error, None, "valid native binding must be admitted");
        assert_eq!(outcome.stage, BackupStage::Complete);
        let published = outcome.published.as_ref().unwrap();
        assert_eq!(
            crate::project_board_history_fixture::fingerprint(published),
            before
        );
    } else {
        assert_eq!(outcome.error, Some(BackupError::IdentityMismatch));
        assert!(outcome.published.is_none());
        assert!(!to.join(format!("{A}-20261025T120000Z.sqlite")).exists());
    }
    assert_eq!(
        crate::project_board_history_fixture::fingerprint(&source.path),
        before
    );
}
#[test]
fn adapter_matching_bound_native_is_accepted() {
    native_admission(Some(A), true);
}
#[test]
fn adapter_mismatched_bound_native_is_refused() {
    native_admission(Some(B), false);
}
#[test]
fn adapter_unbound_native_null_remains_accepted() {
    native_admission(None, true);
}
