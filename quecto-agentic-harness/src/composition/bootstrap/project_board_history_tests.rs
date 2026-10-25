//! #2741 review M2: import and export reach the board's state directory
//! only through serve's allowlist (absolute, no trailing `/`, no symlink,
//! this user's at mode 0700), and import holds serve's directory lock.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::*;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::interface::cli::board_server::history::{HistoryCliEnvironment, ImportCliEnvironment};

fn board() -> BoardId {
    BoardId::of_hex("0123456789abcdef").unwrap()
}

fn import(base: &Path, state: Option<PathBuf>) -> Result<(), String> {
    build_history_import_cli(ImportCliEnvironment {
        repo: PathBuf::from("/nonexistent/source"),
        board: board(),
        remote: "origin".into(),
        base_dir: base.to_path_buf(),
        state,
    })
    .map(drop)
}

fn export(base: &Path, state: Option<PathBuf>) -> Result<(), String> {
    build_history_cli(HistoryCliEnvironment {
        repo: PathBuf::from("/nonexistent/destination"),
        board: board(),
        target: "refs/heads/rollback".into(),
        base_dir: base.to_path_buf(),
        state,
    })
    .map(drop)
}

fn refused(result: Result<(), String>, why: &str) {
    let error = result.err().unwrap_or_else(|| panic!("refused: {why}"));
    assert!(error.contains(why), "{why} in: {error}");
}

#[test]
fn a_relative_base_directory_is_refused() {
    // Under the ignored target directory, should a regression make it.
    let base = Path::new("target/board-2741-relative-base");
    refused(import(base, None), "not absolute");
    refused(export(base, None), "not absolute");
}

#[test]
fn a_state_directory_ending_in_a_slash_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let state = PathBuf::from(format!("{}/state/", dir.path().display()));
    refused(
        import(dir.path(), Some(state.clone())),
        "must not end in '/'",
    );
    refused(export(dir.path(), Some(state)), "must not end in '/'");
}

#[test]
fn a_symlinked_or_shared_state_directory_is_refused_by_import() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let private = "must be a directory of this user's, mode 0700";
    refused(import(dir.path(), Some(link)), private);
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    refused(import(dir.path(), Some(real.clone())), private);
    assert!(!real.join(super::super::board_server::BOARD_FILE).exists());
}

#[test]
fn import_refuses_while_a_server_holds_the_state_directory() {
    use crate::infrastructure::persistence::file::board_tokens::FileBoardTokens;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let serving = FileBoardTokens::open(&state).unwrap();
    refused(
        import(dir.path(), Some(state.clone())),
        "a board server holds it",
    );
    drop(serving);
    assert!(import(dir.path(), Some(state)).is_ok(), "free once stopped");
}
