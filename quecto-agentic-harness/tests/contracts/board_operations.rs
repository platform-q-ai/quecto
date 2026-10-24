//! Contract for [`BoardOperations`] (epic #2705 B6): what the board's
//! front ends are handed for a request, as every adapter must answer it.
//! The checks take the operations of one fresh board that is not
//! bootstrapped yet; here they run against the local adapter as
//! composition builds it for `quecto board`, over a clone of a bare git
//! remote. A board server's adapter (B7) runs the same checks.
use quecto::application::project_board::dto::ProjectBoardError;
use quecto::application::project_board::operations::{BoardOutcome, BoardRequest, ListSummaries};
use quecto::application::project_board::ports::{
    BoardAnswered, BoardExecution, BoardOperations, OpNoting,
};
use quecto::composition::bootstrap::project_board::build_project_board_handles;
use quecto::interface::cli::controllers::board::{Access, ProjectBoardEnvironment};
use quecto::interface::tools::project_board::request::parse;
use serde_json::{Value, json};

use super::project_board_git_store::Remote;

fn request(value: Value) -> BoardRequest {
    parse(&value).expect("a well-formed request")
}

fn create(id: &str) -> BoardRequest {
    request(json!({"op": "create", "task": id, "title": "A task", "status": "ready"}))
}

fn answered(execution: BoardExecution) -> BoardAnswered {
    match execution.result {
        Ok(answered) => answered,
        Err(error) => panic!("refused: {error:?}"),
    }
}

fn run(operations: &dyn BoardOperations, request: &BoardRequest) -> BoardAnswered {
    answered(operations.execute(request, ListSummaries::Omitted, OpNoting::Unnoted))
}

fn listed_ids(outcome: &BoardOutcome) -> Vec<String> {
    match outcome {
        BoardOutcome::Listing(listed) => listed
            .entries()
            .map(|(task, _)| task.fields().id.as_str().to_owned())
            .collect(),
        other => panic!("not a listing: {other:?}"),
    }
}

/// The board is created once: a bootstrap after it finds it, at the head
/// it was created at.
pub fn check_bootstrap(operations: &dyn BoardOperations) {
    let bootstrap = request(json!({"op": "bootstrap"}));
    let BoardOutcome::Bootstrapped {
        head,
        created: true,
    } = run(operations, &bootstrap).outcome
    else {
        panic!("created")
    };
    assert_eq!(
        run(operations, &bootstrap).outcome,
        BoardOutcome::Bootstrapped {
            head,
            created: false
        }
    );
}

/// A task created is shown and listed as written.
pub fn check_create_show_and_list(operations: &dyn BoardOperations) {
    run(operations, &request(json!({"op": "bootstrap"})));
    let BoardOutcome::Changed { task, completion } = run(operations, &create("a")).outcome else {
        panic!("changed")
    };
    assert_eq!((task.fields().id.as_str(), completion), ("a", None));
    let BoardOutcome::Shown(shown, _) =
        run(operations, &request(json!({"op": "show", "task": "a"}))).outcome
    else {
        panic!("shown")
    };
    assert_eq!(shown.task, task);
    let listing = request(json!({"op": "list"}));
    assert_eq!(listed_ids(&run(operations, &listing).outcome), ["a"]);
    // A listing is never refused for its summaries.
    let summarised = operations.execute(&listing, ListSummaries::Wanted, OpNoting::Unnoted);
    assert_eq!(listed_ids(&answered(summarised).outcome), ["a"]);
}

/// The board's refusal reaches the front end typed, as the use cases
/// refuse: before the board exists, and of a task it does not hold.
pub fn check_refusals(operations: &dyn BoardOperations) {
    let show = request(json!({"op": "show", "task": "missing"}));
    let before = operations.execute(&show, ListSummaries::Omitted, OpNoting::Unnoted);
    assert!(
        matches!(before.result, Err(ProjectBoardError::Store(_))),
        "{before:?}"
    );
    run(operations, &request(json!({"op": "bootstrap"})));
    let missing = operations.execute(&show, ListSummaries::Omitted, OpNoting::Unnoted);
    let Err(error) = missing.result else {
        panic!("refused: {missing:?}")
    };
    assert!(!error.to_string().is_empty(), "a refusal says why");
}

/// What a request's use cases saw comes back only when it is noted, a
/// refusal's included.
pub fn check_noting(operations: &dyn BoardOperations) {
    let bootstrap = request(json!({"op": "bootstrap"}));
    let unnoted = operations.execute(&bootstrap, ListSummaries::Omitted, OpNoting::Unnoted);
    assert_eq!(unnoted.facts, None);
    let noted = operations.execute(&create("a"), ListSummaries::Omitted, OpNoting::Noted);
    assert!(noted.result.is_ok(), "{noted:?}");
    let facts = noted.facts.expect("a noted request gives back its facts");
    assert_eq!(facts.attempts, Some(1), "one write, at once");
    let refused = operations.execute(&create("a"), ListSummaries::Omitted, OpNoting::Noted);
    assert!(refused.result.is_err(), "a task is created once");
    assert!(refused.facts.is_some(), "a refusal is noted too");
}

/// A request that does not read the clock carries no clock note.
pub fn check_clock_notes(operations: &dyn BoardOperations) {
    for value in [json!({"op": "bootstrap"}), json!({"op": "list"})] {
        assert_eq!(
            run(operations, &request(value.clone())).clock_note,
            None,
            "{value}"
        );
    }
}

/// A front end that launches nothing owes no ends as its session ends.
pub fn check_no_left_runs(operations: &dyn BoardOperations) {
    assert_eq!(operations.left_runs(), Ok(vec![]));
}

/// The local adapter over a fresh clone of a bare remote, writing as
/// `Ada`, as composition builds it for `quecto board` (no launcher).
fn local(remote: &Remote) -> Box<dyn BoardOperations> {
    let repo = remote.clone_dir(0).to_path_buf();
    let vars = vec![
        ("PATH".into(), std::env::var_os("PATH").expect("PATH")),
        ("HOME".into(), repo.clone().into_os_string()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        (
            "QUECTO_BOARD_COMMITTER".into(),
            "Ada <ada@example.com>".into(),
        ),
    ];
    let environment = ProjectBoardEnvironment {
        repo,
        remote: "origin".into(),
        access: Access::Write,
        vars,
        cancelled: None,
        launcher: None,
        swarm_session: None,
    };
    match build_project_board_handles(&environment) {
        Ok(handles) => handles.operations,
        Err(error) => panic!("the local board is built: {error:?}"),
    }
}

fn each(check: fn(&dyn BoardOperations)) {
    let remote = Remote::new();
    check(&*local(&remote));
}

#[test]
fn local_board_operations_bootstrap() {
    each(check_bootstrap);
}

#[test]
fn local_board_operations_create_show_and_list() {
    each(check_create_show_and_list);
}

#[test]
fn local_board_operations_refusals() {
    each(check_refusals);
}

#[test]
fn local_board_operations_noting() {
    each(check_noting);
}

#[test]
fn local_board_operations_clock_notes() {
    each(check_clock_notes);
}

#[test]
fn local_board_operations_no_left_runs() {
    each(check_no_left_runs);
}
