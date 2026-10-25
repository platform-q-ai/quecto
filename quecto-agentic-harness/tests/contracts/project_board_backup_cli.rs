//! Real offline composition through the built binary, using owned HOME/XDG roots.
use quecto::{
    application::project_board::ports::ProjectBoardStore,
    infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore,
};
use std::{
    path::Path,
    process::{Command, Output},
};
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
        .current_dir(home)
        .output()
        .unwrap()
}
#[test]
fn backup_cli_real_offline_graph_publishes_independent_restore_and_typed_unknown_board() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("data/quecto/boards");
    let to = home.path().join("backups");
    let missing = cli(
        home.path(),
        &["board-server", "backup", "--to", to.to_str().unwrap()],
    );
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&missing.stdout).unwrap()["error"],
        "no_targets"
    );
    assert_eq!(
        std::fs::read_dir(home.path()).unwrap().count(),
        0,
        "missing sources leave all owned HOME roots untouched"
    );
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("0123456789abcdef.sqlite");
    let writer = crate::project_board_history_fixture::writer();
    let store = SqliteProjectBoardStore::open(&source, writer).unwrap();
    store.bootstrap().unwrap();
    let keeper = rusqlite::Connection::open(&source).unwrap();
    keeper.execute_batch("PRAGMA wal_autocheckpoint=0").unwrap();
    store
        .fixture_put_raw("tasks/sentinel.md", b"real offline CLI WAL snapshot")
        .unwrap();
    let before = crate::project_board_history_fixture::fingerprint(&source);
    let result = cli(
        home.path(),
        &["board-server", "backup", "--to", to.to_str().unwrap()],
    );
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let outcome = &report["outcomes"][0];
    assert_eq!(outcome["board"], "0123456789abcdef");
    assert_eq!(outcome["retained"], 1);
    assert_eq!(outcome["deleted"], 0);
    assert_eq!(outcome["stage"], "complete");
    assert!(outcome["error"].is_null());
    assert!(outcome["taken_at"].as_str().unwrap().ends_with('Z'));
    let published = std::path::PathBuf::from(outcome["published"].as_str().unwrap());
    assert_eq!(
        crate::project_board_history_fixture::fingerprint(&source),
        before
    );
    let unknown = cli(
        home.path(),
        &[
            "board-server",
            "backup",
            "--to",
            to.to_str().unwrap(),
            "--board-id",
            "fedcba9876543210",
        ],
    );
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&unknown.stdout).unwrap()["error"],
        "unknown_board"
    );
    drop(store);
    drop(keeper);
    std::fs::rename(&root, home.path().join("unavailable-source")).unwrap();
    assert_eq!(
        crate::project_board_history_fixture::fingerprint(&published),
        before
    );
    let restored = rusqlite::Connection::open(&published).unwrap();
    assert_eq!(
        restored
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let mut final_names: Vec<_> = std::fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    final_names.sort();
    assert_eq!(
        final_names,
        ["backups", "data", "unavailable-source"],
        "offline graph never constructs config, token, or listener state"
    );
}
