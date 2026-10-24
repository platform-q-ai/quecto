//! #2705 B1: where a board is, resolved once. A front end that names its
//! board's id and its remote's URL (`quecto board --board-id
//! --remote-url`) is composed without a git process; each part it names
//! skips only the git call that part would need, and one that names
//! neither is composed exactly as before. git is counted through a `git`
//! on the composed `PATH` that logs each call, then runs the real one.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::tests::{environment, repository, sh};
use super::*;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::interface::tools::project_board::run;
use crate::interface::tools::project_board::site::KnownLocation;

/// A directory holding a `git` that appends each call's arguments to
/// `log` and then runs the real git: the directory, and the log.
fn counting_git(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .expect("a git on the PATH");
    let bin = dir.join("counting-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.join("git-calls.log");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        log.display(),
        real.display()
    );
    let git = bin.join("git");
    std::fs::write(&git, script).unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

/// Each git call logged so far, as its arguments joined by spaces.
fn calls(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `work`'s board for `access`, told `known`, with the counting git first
/// on its `PATH`.
fn counted(
    work: &Path,
    access: Access,
    known: KnownLocation,
    bin: &Path,
) -> ProjectBoardEnvironment {
    let mut counted = environment(work, access);
    let path = std::env::join_paths(
        std::iter::once(bin.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for (key, value) in &mut counted.vars {
        if key == "PATH" {
            *value = path.clone();
        }
    }
    counted.known = known;
    counted
}

/// A board on `work` with one ready task `a`; its id, as git resolves it,
/// and its remote's URL.
fn board_with_a(work: &Path) -> (BoardId, String) {
    let writer = build_project_board_handles(&environment(work, Access::Write)).unwrap();
    let ops = |request: Value| run(&writer.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    let url = sh(work, &["remote", "get-url", "origin"]);
    (writer.board.expect("a local remote has an id"), url)
}

fn any_call(calls: &[String], spelt: &str) -> bool {
    calls.iter().any(|call| call.contains(spelt))
}

#[test]
fn a_list_on_a_given_location_spawns_no_git_to_compose() {
    let (dir, work) = repository();
    let (board, url) = board_with_a(&work);
    let (bin, log) = counting_git(dir.path());
    let known = KnownLocation {
        board_id: Some(board.clone()),
        remote_url: Some(url),
    };
    let handles = build_project_board_handles(&counted(&work, Access::Read, known, &bin)).unwrap();
    assert_eq!(
        calls(&log),
        Vec::<String>::new(),
        "composition spawns no git"
    );
    assert_eq!(handles.board, Some(board), "the board id given");
    let listed = run(&handles.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"][0]["id"], json!("a"), "{listed}");
    assert!(
        !calls(&log).is_empty(),
        "the listing itself reads the board with git"
    );
}

#[test]
fn a_board_with_no_given_location_asks_git_for_it_as_before() {
    let (dir, work) = repository();
    let (board, _) = board_with_a(&work);
    let (bin, log) = counting_git(dir.path());
    let reading = counted(&work, Access::Read, KnownLocation::default(), &bin);
    let handles = build_project_board_handles(&reading).unwrap();
    // Each call's own arguments, past the settings every call carries.
    let asked: Vec<String> = calls(&log)
        .iter()
        .map(|call| {
            let (_, own) = call
                .split_once("maintenance.auto=false ")
                .expect("every call carries the runner's settings");
            own.to_owned()
        })
        .collect();
    assert_eq!(
        asked,
        [
            "var GIT_COMMITTER_IDENT",
            "remote get-url origin",
            "remote get-url -- origin",
            "rev-parse --show-toplevel",
            "rev-parse --path-format=absolute --git-common-dir",
        ],
        "exactly the calls composition made before #2705 B1, in order"
    );
    assert_eq!(handles.board, Some(board));
}

/// A remote folder beside the clone that the clone does not configure.
fn decoy(work: &Path) -> PathBuf {
    sh(work, &["init", "-q", "--bare", "decoy"]);
    work.join("decoy")
}

/// The decoy's branches, as git lists them.
fn decoy_branches(decoy: &Path) -> String {
    sh(decoy, &["for-each-ref", "--format=%(refname)"])
}

#[test]
fn a_write_takes_no_location_on_trust() {
    let (dir, work) = repository();
    board_with_a(&work);
    let decoy = decoy(&work);
    let (bin, log) = counting_git(dir.path());
    let mut writing = counted(&work, Access::Write, given_location(), &bin);
    writing.remote = "decoy".into();
    assert!(matches!(
        build_project_board_handles(&writing),
        Err(BuildError::NoSuchRemote(_))
    ));
    assert_eq!(decoy_branches(&decoy), "", "nothing written to the decoy");
    let asked = calls(&log);
    assert!(!any_call(&asked, "push"), "{asked:?}");
}

#[test]
fn a_read_on_a_known_location_never_reads_an_unconfigured_remote_folder() {
    let (dir, work) = repository();
    board_with_a(&work);
    let decoy = decoy(&work);
    // Something on the decoy that a read of it would find.
    sh(
        &work,
        &[
            "push",
            "-q",
            "decoy",
            "refs/remotes/origin/quecto/board:refs/heads/quecto/board",
        ],
    );
    let (bin, log) = counting_git(dir.path());
    let mut reading = counted(&work, Access::Read, given_location(), &bin);
    reading.remote = "decoy".into();
    assert!(matches!(
        build_project_board_handles(&reading),
        Err(BuildError::NoSuchRemote(_))
    ));
    let asked = calls(&log);
    assert!(!any_call(&asked, "ls-remote"), "{asked:?}");
    assert!(!any_call(&asked, "fetch"), "{asked:?}");
    assert_ne!(decoy_branches(&decoy), "");
}

/// A whole location, as the pocket relay gives one.
fn given_location() -> KnownLocation {
    KnownLocation {
        board_id: Some(BoardId::of_hex("0123456789abcdef").unwrap()),
        remote_url: Some("https://github.com/acme/widgets.git".into()),
    }
}

#[test]
fn a_known_location_in_a_folder_that_is_no_repository_says_so_for_every_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut reading = environment(dir.path(), Access::Read);
    reading.known = given_location();
    let refused = build_project_board_handles(&reading).err();
    let Some(BuildError::Unavailable(why)) = refused else {
        panic!("{refused:?}");
    };
    assert!(!why.contains("ls-remote"), "{why}");
    let site = crate::interface::tools::project_board::site::BoardSite {
        repo: dir.path().to_path_buf(),
        remote: "origin".into(),
        known: given_location(),
        build: Some(build_project_board_handles),
        launcher: None,
        swarm_session: None,
    };
    use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
    use crate::interface::tools::project_board::recording::BoardCall;
    let answered = |request: &str| {
        let mut call = BoardCall::start(None, FrontEnd::Cli, Client::Cli);
        crate::interface::tools::project_board::site::answer(&site, request, None, &mut call)
    };
    let (show, list) = (
        answered(r#"{"op": "show", "task": "a"}"#),
        answered(r#"{"op": "list"}"#),
    );
    assert_eq!(show["error"]["code"], json!("store_unavailable"), "{show}");
    assert_eq!(
        show["error"]["reason"], list["error"]["reason"],
        "{show} {list}"
    );
}

#[test]
fn each_given_part_skips_only_the_git_call_it_answers() {
    let (dir, work) = repository();
    let (_, url) = board_with_a(&work);
    let (bin, log) = counting_git(dir.path());
    // An id alone: the board is named as given, its top level never asked.
    let named = BoardId::of_hex("0123456789abcdef").unwrap();
    let known = KnownLocation {
        board_id: Some(named.clone()),
        remote_url: None,
    };
    let handles = build_project_board_handles(&counted(&work, Access::Read, known, &bin)).unwrap();
    let asked = calls(&log);
    assert_eq!(handles.board, Some(named));
    assert!(!any_call(&asked, "--show-toplevel"), "{asked:?}");
    assert!(any_call(&asked, "remote get-url"), "{asked:?}");
    std::fs::remove_file(&log).unwrap();
    // A URL alone: the remote is checked but never asked for its URL, and
    // the id is made from the URL given.
    let given = "https://github.com/acme/widgets.git";
    let known = KnownLocation {
        board_id: None,
        remote_url: Some(given.into()),
    };
    let handles = build_project_board_handles(&counted(&work, Access::Read, known, &bin)).unwrap();
    let asked = calls(&log);
    assert!(!any_call(&asked, "remote get-url -- origin"), "{asked:?}");
    assert!(any_call(&asked, "remote get-url origin"), "{asked:?}");
    assert!(any_call(&asked, "--show-toplevel"), "{asked:?}");
    assert_eq!(
        handles.board,
        GitBoardStore::board_id_of(given, &work),
        "the id of the URL given"
    );
    assert_ne!(
        handles.board,
        GitBoardStore::board_id_of(&url, &work),
        "not the id of the clone's own remote"
    );
}

#[test]
fn a_given_location_still_refuses_a_remote_name_quecto_does_not_accept() {
    let (_dir, work) = repository();
    let mut reading = environment(&work, Access::Read);
    reading.remote = "../elsewhere".into();
    reading.known = KnownLocation {
        board_id: Some(BoardId::of_hex("0123456789abcdef").unwrap()),
        remote_url: Some("https://github.com/acme/widgets.git".into()),
    };
    assert!(matches!(
        build_project_board_handles(&reading),
        Err(BuildError::NoSuchRemote(_))
    ));
}
