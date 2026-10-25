//! #2705 B1: where a board is, resolved once. A front end that names its
//! board's id and its remote's URL (`quecto board --board-id
//! --remote-url`) is composed with one git process, the check that its
//! remote is one the clone configures; each part it names skips only the
//! git call that part would need, and one that names neither is composed
//! exactly as before. A write takes no location on trust. git is counted
//! through a `git` on the composed `PATH` that logs each call, then runs
//! the real one.
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
/// Task `id` on `repo`'s board moved to review, as its run's end will move
/// it (#2705 T4: `move` no longer does), written straight to the store as
/// the committer the clone configures.
pub(super) fn put_in_review(repo: &Path, id: &str) {
    let environment = environment(repo, Access::Write);
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    let committer = Git::configured_committer(&environment.repo, &vars).unwrap();
    let git = Git::new(&environment.repo, vars, committer).unwrap();
    let store = GitBoardStore::new(git, &environment.remote).unwrap();
    let board = store.read().unwrap();
    let task = board
        .tasks
        .iter()
        .find(|task| task.fields().id.as_str() == id);
    let mut fields = task.expect("the task is on the board").fields().clone();
    fields.status = crate::domain::project_board::value_objects::vocabulary::TaskStatus::Review;
    let task = crate::domain::project_board::entities::task::Task::new(fields).unwrap();
    store
        .write_tasks(&board.head, &[task], "board: a run's end")
        .unwrap();
}

fn board_with_a(work: &Path) -> (BoardId, String) {
    let writer = build_project_board_handles(&environment(work, Access::Write)).unwrap();
    let ops = |request: Value| run(&*writer.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    let url = sh(work, &["remote", "get-url", "origin"]);
    (writer.board.expect("a local remote has an id"), url)
}

fn any_call(calls: &[String], spelt: &str) -> bool {
    calls.iter().any(|call| call.contains(spelt))
}

/// Each call logged so far, as its own arguments, past the settings every
/// call carries.
fn own_calls(log: &Path) -> Vec<String> {
    calls(log)
        .iter()
        .map(|call| {
            let (_, own) = call
                .split_once("maintenance.auto=false ")
                .expect("every call carries the runner's settings");
            own.to_owned()
        })
        .collect()
}

#[test]
fn a_list_on_a_given_location_spawns_only_the_remote_check_to_compose() {
    let (dir, work) = repository();
    let (board, url) = board_with_a(&work);
    let (bin, log) = counting_git(dir.path());
    let known = KnownLocation {
        board_id: Some(board.clone()),
        remote_url: Some(url),
    };
    let handles = build_project_board_handles(&counted(&work, Access::Read, known, &bin)).unwrap();
    assert_eq!(
        own_calls(&log),
        ["remote get-url origin"],
        "composition spawns one git: the check that the remote is configured"
    );
    assert_eq!(handles.board, Some(board), "the board id given");
    let listed = run(&*handles.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"][0]["id"], json!("a"), "{listed}");
    assert!(
        own_calls(&log).len() > 1,
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
    assert_eq!(
        own_calls(&log),
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

/// A board on `work` and a copy of it in a bare `decoy` folder in `repo`,
/// where git would read a remote named `decoy` taken for a path; then a
/// read of `repo`'s `decoy` on `known`, which must be refused before git
/// reads any remote at all.
fn refused_without_reading_the_decoy(dir: &Path, work: &Path, repo: &Path, known: KnownLocation) {
    sh(repo, &["init", "-q", "--bare", "decoy"]);
    let decoy = repo.join("decoy");
    let spec = "refs/remotes/origin/quecto/board:refs/heads/quecto/board";
    sh(work, &["push", "-q", decoy.to_str().unwrap(), spec]);
    let (bin, log) = counting_git(dir);
    let mut reading = counted(repo, Access::Read, known, &bin);
    reading.remote = "decoy".into();
    let built = build_project_board_handles(&reading).err();
    assert!(
        matches!(built, Some(BuildError::NoSuchRemote(_))),
        "{built:?}"
    );
    let asked = calls(&log);
    assert!(!any_call(&asked, "ls-remote"), "{asked:?}");
    assert!(!any_call(&asked, "fetch"), "{asked:?}");
    assert_ne!(decoy_branches(&decoy), "", "the decoy holds a board");
}

#[test]
fn a_read_on_a_known_location_never_reads_an_unconfigured_remote_folder() {
    let (dir, work) = repository();
    board_with_a(&work);
    refused_without_reading_the_decoy(dir.path(), &work, &work, given_location());
}

/// Round 2: a `[remote "decoy"]` line inside a backslash-continued value
/// is no remote to git, whatever a reading of the file by hand finds.
#[test]
fn a_remote_header_inside_a_continued_value_is_no_remote() {
    let (dir, work) = repository();
    board_with_a(&work);
    let config = work.join(".git/config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[alias]\n\tx = log \\\n[remote \"decoy\"]\n\turl = decoy\n");
    std::fs::write(&config, text).unwrap();
    refused_without_reading_the_decoy(dir.path(), &work, &work, given_location());
}

/// Round 2: a `.git` directory git does not take for one (no `HEAD`,
/// objects or refs) leaves git in the repository around it, whose remotes
/// are the ones that count.
#[test]
fn a_git_directory_git_does_not_take_for_one_lists_no_remote() {
    let (dir, work) = repository();
    board_with_a(&work);
    let inner = work.join("inner");
    std::fs::create_dir_all(inner.join(".git")).unwrap();
    std::fs::write(
        inner.join(".git/config"),
        "[remote \"decoy\"]\n\turl = /nonexistent/decoy.git\n",
    )
    .unwrap();
    refused_without_reading_the_decoy(dir.path(), &work, &inner, given_location());
}

/// A remote whose `url` was emptied: git prints its name back as its URL,
/// and would read a folder of that name, so it is no remote (allowlist).
#[test]
fn a_remote_whose_url_was_emptied_is_no_remote() {
    let (dir, work) = repository();
    board_with_a(&work);
    sh(&work, &["config", "remote.decoy.url", ""]);
    refused_without_reading_the_decoy(dir.path(), &work, &work, KnownLocation::default());
    let (_kept, emptied) = repository();
    sh(&emptied, &["config", "remote.decoy.url", ""]);
    let mut reading = environment(&emptied, Access::Read);
    reading.remote = "decoy".into();
    reading.known = given_location();
    let built = build_project_board_handles(&reading).err();
    assert!(
        matches!(built, Some(BuildError::NoSuchRemote(_))),
        "{built:?}"
    );
}

/// A write on `origin` told another board id and URL builds on the ones
/// git resolves: its records name the real board, and pull request
/// states are judged by the real remote (a local one: no host), never
/// by the URL given (GitHub).
#[test]
fn a_write_names_the_board_and_remote_git_resolves() {
    let (dir, work) = repository();
    let (board, _) = board_with_a(&work);
    let (bin, _) = counting_git(dir.path());
    // Only the counting git on the PATH: no gh to ask GitHub with.
    let mut writing = counted(&work, Access::Write, given_location(), &bin);
    for (key, value) in &mut writing.vars {
        if key == "PATH" {
            *value = bin.clone().into_os_string();
        }
    }
    let handles = build_project_board_handles(&writing).unwrap();
    assert_eq!(handles.board, Some(board), "the id git resolves");
    assert_ne!(handles.board, given_location().board_id);
    let ops = |request: Value| run(&*handles.operations, &request.to_string());
    let claimed = ops(json!({"op": "move", "task": "a", "to": "claimed"}));
    assert_eq!(claimed["ok"], json!(true), "{claimed}");
    let linked = ops(json!({"op": "link_pr", "task": "a", "pr": 7}));
    assert_eq!(linked["ok"], json!(true), "{linked}");
    put_in_review(&work, "a");
    let complete = ops(json!({"op": "complete", "task": "a"}));
    assert_eq!(
        complete["error"]["code"],
        json!("no_pull_request_host"),
        "{complete}"
    );
    let reason = complete["error"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("not on GitHub"), "{complete}");
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
