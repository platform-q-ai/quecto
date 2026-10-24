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

#[test]
fn the_server_serves_the_git_boards_head() {
    let (dir, work) = seeded();
    let remote = dir.path().join("remote.git");
    let handles = build_board_server(&environment(&dir, &work, "origin")).unwrap();
    assert_eq!(handles.uid, this_uid());
    assert!(handles.listen.is_empty(), "no config, no TCP");
    let routes = BoardRoutes::new(handles.serve, handles.access);
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    let head = sh(&remote, &["rev-parse", "quecto/board"]);
    assert_eq!(body["head"], head.trim());
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
