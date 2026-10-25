//! The board server's graph over a real git store (epic #2705 B4): its
//! status gives the remote's board head.
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::*;
use crate::composition::bootstrap::project_board::build_project_board_handles;
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceRequest, Transport, this_uid,
};
use crate::interface::cli::board_server::routes::BoardRoutes;
use crate::interface::cli::controllers::board::{Access, ProjectBoardEnvironment};
use crate::interface::tools::project_board::run;

/// Test-side git: no global or system config.
fn sh(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", "/nonexistent"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn vars(home: &Path) -> Vec<(OsString, OsString)> {
    vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), home.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ]
}

/// A clone of a bare remote whose board holds a task and its plan.
fn seeded() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    sh(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    );
    sh(dir.path(), &["clone", "-q", "remote.git", "work"]);
    let work = dir.path().join("work");
    sh(&work, &["config", "user.name", "Ada"]);
    sh(&work, &["config", "user.email", "ada@example.com"]);
    let writing = ProjectBoardEnvironment {
        repo: work.clone(),
        remote: "origin".into(),
        known: Default::default(),
        access: Access::Write,
        vars: vars(&work),
        cancelled: None,
        launcher: None,
        swarm_session: None,
    };
    let handles = build_project_board_handles(&writing).unwrap();
    let ops = |request: Value| run(&*handles.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    let proposed = ops(
        json!({"op": "propose_plan", "task": "a", "approach": "Build it.",
                              "items": [{"id": "schema", "title": "Schema"}]}),
    );
    assert_eq!(proposed["ok"], json!(true), "{proposed}");
    (dir, work)
}

/// The server's environment for `work`: base dir and config under `dir`.
fn environment(dir: &tempfile::TempDir, work: &Path, remote: &str) -> BoardServerEnvironment {
    BoardServerEnvironment {
        repo: work.to_path_buf(),
        remote: remote.into(),
        vars: vars(work),
        config: dir.path().join("base").join("config.json"),
        state: None,
        base_dir: dir.path().join("base"),
    }
}

fn get(routes: &BoardRoutes, path: &str) -> (u16, Value) {
    let reply = routes.answer(&ServiceRequest {
        method: ServiceMethod::Get,
        path: path.into(),
        body: vec![],
        transport: Transport::Local,
        principal: None,
    });
    (reply.status, serde_json::from_slice(&reply.body).unwrap())
}

/// B7: the server serves its own SQLite board in its state directory, not
/// the clone's git board: none until it is made.
#[test]
fn the_server_serves_its_sqlite_board() {
    let (dir, work) = seeded();
    let handles = build_board_server(&environment(&dir, &work, "origin")).unwrap();
    assert_eq!(handles.uid, this_uid());
    assert!(handles.listen.is_empty(), "no config, no TCP");
    let routes = BoardRoutes::new(handles.serve, handles.access, handles.operations);
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["head"], Value::Null, "no board made yet");
    assert_eq!(body["role"], "primary");
    assert_eq!(body["board"].as_str().map(str::len), Some(16), "{body}");
    assert_eq!(get(&routes, "/v1/time").0, 200);
}

#[test]
fn a_remote_the_clone_does_not_configure_is_refused() {
    let (_dir, work) = seeded();
    let error = build_board_server(&environment(&_dir, &work, "nowhere"))
        .err()
        .unwrap();
    assert!(error.contains("nowhere"), "{error}");
}

fn write_config(dir: &tempfile::TempDir, config: &Value) {
    let base = dir.path().join("base");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("config.json"), config.to_string()).unwrap();
}

/// `board_server.listen` in the global config names the TCP addresses.
#[test]
fn the_config_names_the_tcp_addresses() {
    let (dir, work) = seeded();
    write_config(
        &dir,
        &json!({"board_server": {"listen": ["127.0.0.1:7979", "100.64.1.2:7979"]}}),
    );
    let handles = build_board_server(&environment(&dir, &work, "origin")).unwrap();
    let listen: Vec<String> = handles
        .listen
        .iter()
        .map(|address| address.socket_addr().to_string())
        .collect();
    assert_eq!(listen, ["127.0.0.1:7979", "100.64.1.2:7979"]);
}

/// B5 red: `0.0.0.0` in the config is refused at start, as on the command
/// line.
#[test]
fn a_config_naming_every_interface_is_refused_at_start() {
    let (dir, work) = seeded();
    write_config(&dir, &json!({"board_server": {"listen": ["0.0.0.0:7979"]}}));
    let error = build_board_server(&environment(&dir, &work, "origin"))
        .err()
        .unwrap();
    assert!(error.contains("board_server.listen in"), "{error}");
    assert!(error.contains("0.0.0.0:7979 is refused"), "{error}");
    write_config(&dir, &json!({"board_server": {"listen": [], "bind": "x"}}));
    let error = build_board_server(&environment(&dir, &work, "origin"))
        .err()
        .unwrap();
    assert!(error.contains("config"), "an unknown key: {error}");
}

/// The tokens live in `<base_dir>/board-server/<board id>` (0700), or in
/// `--state`; a token added there is read back by the next server.
#[test]
fn the_tokens_live_in_the_state_directory() {
    let (dir, work) = seeded();
    let handles = build_board_server(&environment(&dir, &work, "origin")).unwrap();
    let board = handles.serve.board().as_str().to_string();
    let issued = handles.access.add("laptop").unwrap();
    let file = dir
        .path()
        .join("base")
        .join("board-server")
        .join(&board)
        .join("tokens.json");
    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(
        raw.contains("laptop") && !raw.contains(&issued.secret),
        "{raw}"
    );
    // Review round 1: one server per state directory.
    let error = build_board_server(&environment(&dir, &work, "origin"))
        .err()
        .unwrap();
    assert!(error.contains("another board server holds it"), "{error}");
    drop(handles);
    let again = build_board_server(&environment(&dir, &work, "origin")).unwrap();
    assert!(again.access.authenticate(Some(&issued.secret)).is_ok());
    let state = dir.path().join("elsewhere");
    let elsewhere = build_board_server(&BoardServerEnvironment {
        state: Some(state.clone()),
        ..environment(&dir, &work, "origin")
    })
    .unwrap();
    assert!(elsewhere.access.list().unwrap().is_empty());
    elsewhere.access.add("phone").unwrap();
    assert!(state.join("tokens.json").exists());
}

/// Board holding task `a`, ready, linking pull request 7.
fn linking_seven() -> crate::infrastructure::test_support::memory_board_store::MemoryBoardStore {
    use crate::domain::project_board::entities::task::{Task, TaskFields};
    use crate::domain::project_board::value_objects::slug::Slug;
    use crate::domain::project_board::value_objects::timestamp::Timestamp;
    use crate::domain::project_board::value_objects::vocabulary::TaskKind;
    let store =
        crate::infrastructure::test_support::memory_board_store::MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    let task = Task::new(TaskFields {
        id: Slug::parse("a").unwrap(),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description_form: Default::default(),
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![7],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
        reason: None,
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write_tasks(&head, &[task], "seed").unwrap();
    store
}

#[test]
fn done_without_a_run_has_its_pull_requests_read_before_the_write() {
    use crate::application::project_board::ports::PullRequestState;
    use crate::domain::project_board::value_objects::slug::Slug;
    use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;
    let store = linking_seven();
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    let request = BoardRequest::DoneWithoutRun {
        task: Slug::parse("a").unwrap(),
        reason: "Answered in chat".into(),
    };
    let stated = StatedPullRequests::read(&host, Some(&request), &store, Duration::from_secs(30));
    assert_eq!(host.asked(), vec![7], "read before the write");
    assert_eq!(stated.state(7).unwrap(), PullRequestState::Merged);
}

/// #2741: one resolver names the board's file, in the server's state
/// directory keyed by the board id; `--state` replaces that directory.
#[test]
fn one_resolver_names_the_board_file() {
    use crate::domain::project_board::value_objects::telemetry::BoardId;
    let board = BoardId::of_hex("0123456789abcdef").unwrap();
    let base = Path::new("/scratch/base");
    assert_eq!(
        board_database(base, None, &board),
        base.join("board-server/0123456789abcdef/board.sqlite3")
    );
    assert_eq!(
        board_database(base, Some(Path::new("/scratch/state")), &board),
        Path::new("/scratch/state/board.sqlite3")
    );
}

/// #2741: export reads the server's board file and never makes one.
#[test]
fn export_without_the_servers_board_file_is_refused() {
    use crate::composition::bootstrap::project_board_history::build_history_cli;
    use crate::domain::project_board::value_objects::telemetry::BoardId;
    use crate::interface::cli::board_server::history::HistoryCliEnvironment;
    let dir = tempfile::tempdir().unwrap();
    let board = BoardId::of_hex("0123456789abcdef").unwrap();
    let base = dir.path().join("base");
    let refused = build_history_cli(HistoryCliEnvironment {
        repo: dir.path().to_path_buf(),
        board: board.clone(),
        target: "refs/heads/rollback".into(),
        base_dir: base.clone(),
        state: None,
    })
    .err()
    .unwrap();
    let database = board_database(&base, None, &board);
    assert!(
        refused.contains(&database.display().to_string()),
        "{refused}"
    );
    assert!(!database.exists() && !base.exists(), "nothing made");
}

/// #2741: import writes the board where serve serves it, and export reads
/// it from there: one board file for one board.
#[test]
fn serve_import_and_export_use_one_board_file() {
    use crate::composition::bootstrap::project_board_history::{
        build_history_cli, build_history_import_cli,
    };
    use crate::interface::cli::board_server::history::{
        HistoryCliEnvironment, ImportCliEnvironment,
    };
    let (dir, work) = seeded();
    let base = dir.path().join("base");
    let board = {
        let handles = build_board_server(&environment(&dir, &work, "origin")).unwrap();
        let board = handles.serve.board().clone();
        let routes = BoardRoutes::new(handles.serve, handles.access, handles.operations);
        assert_eq!(
            get(&routes, "/v1/status").1["head"],
            Value::Null,
            "none yet"
        );
        board
    };
    // Import reads the clone's local board branch.
    sh(
        &work,
        &["fetch", "-q", "origin", "quecto/board:quecto/board"],
    );
    let imported = build_history_import_cli(ImportCliEnvironment {
        repo: work.clone(),
        board: board.clone(),
        remote: "origin".into(),
        base_dir: base.clone(),
        state: None,
    })
    .unwrap()
    .execute()
    .unwrap();
    assert!(imported.revisions > 0, "the seeded board has history");
    // The head was null before import (above) and is set after (below).
    let handles = build_board_server(&environment(&dir, &work, "origin")).unwrap();
    let routes = BoardRoutes::new(handles.serve, handles.access, handles.operations);
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    assert_ne!(body["head"], Value::Null, "serve reads the imported board");
    drop(routes);
    let exported = build_history_cli(HistoryCliEnvironment {
        repo: work.clone(),
        board,
        target: "refs/heads/rollback".into(),
        base_dir: base,
        state: None,
    })
    .unwrap();
    let report = exported
        .export
        .execute(true, exported.expected.as_deref())
        .unwrap();
    assert_eq!(report.revisions, imported.revisions, "export reads it too");
}

/// #2741 review M1: a `--state` directory names no board, so a file
/// imported for another board is refused at start, naming both ids.
#[test]
fn serve_refuses_a_board_file_imported_for_another_board() {
    use crate::composition::bootstrap::project_board_history::build_history_import_cli;
    use crate::domain::project_board::value_objects::telemetry::BoardId;
    use crate::interface::cli::board_server::history::ImportCliEnvironment;
    let (dir, work) = seeded();
    let state = dir.path().join("state");
    let mut serving = environment(&dir, &work, "origin");
    serving.state = Some(state.clone());
    let board = build_board_server(&serving).unwrap().serve.board().clone();
    let other = BoardId::of_hex("fedcba9876543210").unwrap();
    assert_ne!(board, other);
    sh(
        &work,
        &["fetch", "-q", "origin", "quecto/board:quecto/board"],
    );
    let imported = build_history_import_cli(ImportCliEnvironment {
        repo: work.clone(),
        board: other.clone(),
        remote: "origin".into(),
        base_dir: dir.path().join("base"),
        state: Some(state.clone()),
    })
    .unwrap()
    .execute();
    assert!(imported.is_ok(), "{:?}", imported.err());
    let refused = build_board_server(&serving).err().unwrap();
    for named in [
        board.as_str(),
        other.as_str(),
        &state.join(BOARD_FILE).display().to_string(),
    ] {
        assert!(refused.contains(named), "{named} in: {refused}");
    }
}

/// #2741 review round 2: serve binds an unbound board file to its board at
/// start, so a second clone's server on the same `--state` is refused, and
/// export takes only the server's board id for it.
#[test]
fn serve_binds_its_board_file_to_its_board() {
    use crate::composition::bootstrap::project_board_history::build_history_cli;
    use crate::domain::project_board::value_objects::telemetry::BoardId;
    use crate::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
    use crate::interface::cli::board_server::history::HistoryCliEnvironment;
    use std::os::unix::fs::DirBuilderExt;
    let (dir, work) = seeded();
    // Another repository's clone: a board id is its remote's.
    sh(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", "other.git"],
    );
    sh(dir.path(), &["clone", "-q", "other.git", "second"]);
    let second = dir.path().join("second");
    let state = dir.path().join("state");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&state)
        .unwrap();
    SqliteProjectBoardStore::open(&state.join(BOARD_FILE), server_identity())
        .unwrap()
        .bootstrap()
        .unwrap();
    let mut serving = environment(&dir, &work, "origin");
    serving.state = Some(state.clone());
    let board = build_board_server(&serving).unwrap().serve.board().clone();
    let mut other = environment(&dir, &second, "origin");
    other.state = Some(state.clone());
    let refused = build_board_server(&other).err().unwrap();
    assert!(refused.contains(board.as_str()), "{refused}");
    let export = |board: BoardId| {
        let handles = build_history_cli(HistoryCliEnvironment {
            repo: work.clone(),
            board,
            target: "refs/heads/rollback".into(),
            base_dir: dir.path().join("base"),
            state: Some(state.clone()),
        })?;
        let report = handles.export.execute(true, handles.expected.as_deref());
        report.map_err(|error| error.to_string())
    };
    let wrong = export(BoardId::of_hex("fedcba9876543210").unwrap());
    assert!(wrong.is_err(), "another board id is refused");
    assert_eq!(
        export(board).unwrap().revisions,
        1,
        "the server's id exports"
    );
}
