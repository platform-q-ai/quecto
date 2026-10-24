//! Contract for [`BoardOperations`] (epic #2705 B6): what the board's
//! front ends are handed for a request, as every adapter must answer it.
//! The checks take the operations of one fresh board that is not
//! bootstrapped yet ([`Fixture`]); here they run against the local adapter
//! as composition builds it for `quecto board`, over a clone of a bare git
//! remote. A board server's adapter (B7) runs the same checks.
use quecto::application::project_board::dto::{
    BoardAnswered, BoardExecution, BoardOutcome, BoardRequest, ListSummaries, OpNoting,
    ProjectBoardError, SUMMARIES_LISTINGS_ONLY,
};
use quecto::application::project_board::ports::BoardOperations;
use quecto::application::project_board::task_summary::{RowSummaries, RowSummary};
use quecto::composition::bootstrap::project_board::build_project_board_handles;
use quecto::domain::project_board::value_objects::schema_error::SchemaError;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::infrastructure::time::remote_time::{ReferenceSource, reference_source};
use quecto::interface::cli::controllers::board::{Access, ProjectBoardEnvironment};
use quecto::interface::tools::project_board::request::parse;
use serde_json::{Value, json};

use super::project_board_git_store::{Remote, sh};

/// A fresh board's operations, and the clock note its fixture's clock
/// gives a request that reads it (`None`: the clock has an outside
/// reference).
pub struct Fixture {
    pub operations: Box<dyn BoardOperations>,
    pub clock_note: Option<String>,
}

fn request(value: Value) -> BoardRequest {
    parse(&value).expect("a well-formed request")
}

fn create(id: &str) -> BoardRequest {
    request(json!({"op": "create", "task": id, "title": "A task", "status": "ready"}))
}

fn slug(id: &str) -> Slug {
    Slug::parse(id).expect("a slug")
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

fn refusal(operations: &dyn BoardOperations, request: &BoardRequest) -> ProjectBoardError {
    match operations
        .execute(request, ListSummaries::Omitted, OpNoting::Unnoted)
        .result
    {
        Err(error) => error,
        Ok(answered) => panic!("answered: {answered:?}"),
    }
}

fn bootstrap(operations: &dyn BoardOperations) {
    run(operations, &request(json!({"op": "bootstrap"})));
}

fn listed(outcome: &BoardOutcome) -> (Vec<String>, &RowSummaries) {
    match outcome {
        BoardOutcome::Listing(listed) => (
            listed
                .entries()
                .map(|(task, _)| task.fields().id.as_str().to_owned())
                .collect(),
            listed.summaries(),
        ),
        other => panic!("not a listing: {other:?}"),
    }
}

/// The board is created once: a bootstrap after it finds it, at the head
/// it was created at.
pub fn check_bootstrap(fixture: Fixture) {
    let operations = &*fixture.operations;
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
pub fn check_create_show_and_list(fixture: Fixture) {
    let operations = &*fixture.operations;
    bootstrap(operations);
    let BoardOutcome::Changed { task, completion } = run(operations, &create("a")).outcome else {
        panic!("changed")
    };
    assert_eq!((task.fields().id.as_str(), completion), ("a", None));
    let show = request(json!({"op": "show", "task": "a"}));
    let BoardOutcome::Shown(shown, _) = run(operations, &show).outcome else {
        panic!("shown")
    };
    assert_eq!(shown.task, task);
    let listing = request(json!({"op": "list"}));
    assert_eq!(listed(&run(operations, &listing).outcome).0, ["a"]);
}

/// A listing carries summaries only when they are asked for: one per task
/// it lists, or why none was read; asked for none, it carries none. Asked
/// of a request that lists nothing, summaries are refused as invalid.
pub fn check_summaries(fixture: Fixture) {
    let operations = &*fixture.operations;
    bootstrap(operations);
    run(operations, &create("a"));
    let listing = request(json!({"op": "list"}));
    let omitted = run(operations, &listing);
    assert_eq!(listed(&omitted.outcome).1, &RowSummaries::Omitted);
    let wanted = operations.execute(&listing, ListSummaries::Wanted, OpNoting::Unnoted);
    let wanted = answered(wanted);
    let (ids, summaries) = listed(&wanted.outcome);
    assert_eq!(ids, ["a"], "a listing is never refused for its summaries");
    match summaries {
        RowSummaries::Given(rows) => {
            assert_eq!(rows.len(), 1, "one summary per listed task");
            assert!(matches!(rows[0], RowSummary::Summary(_)), "{rows:?}");
        }
        RowSummaries::Skipped(why) => {
            panic!("a quiet one-writer board is summarised, not skipped: {why:?}")
        }
        RowSummaries::Omitted => panic!("summaries were asked for"),
    }
    let show = request(json!({"op": "show", "task": "a"}));
    let refused = operations.execute(&show, ListSummaries::Wanted, OpNoting::Unnoted);
    assert_eq!(
        refused.result,
        Err(ProjectBoardError::Invalid(SchemaError::new(
            "summary",
            SUMMARIES_LISTINGS_ONLY
        )))
    );
}

/// The board's refusal reaches the front end typed, as the use cases
/// refuse: before the board exists, of a task it does not hold, and of a
/// task created twice.
pub fn check_refusals(fixture: Fixture) {
    let operations = &*fixture.operations;
    let show = request(json!({"op": "show", "task": "missing"}));
    let before = refusal(operations, &show);
    assert!(matches!(before, ProjectBoardError::Store(_)), "{before:?}");
    bootstrap(operations);
    assert_eq!(
        refusal(operations, &show),
        ProjectBoardError::NotFound(slug("missing"))
    );
    run(operations, &create("a"));
    assert_eq!(
        refusal(operations, &create("a")),
        ProjectBoardError::Exists(slug("a"))
    );
}

/// What a request's use cases saw comes back exactly when it is noted:
/// every noted request, answered or refused, gives it back.
pub fn check_noting(fixture: Fixture) {
    let operations = &*fixture.operations;
    let show = request(json!({"op": "show", "task": "a"}));
    let listing = request(json!({"op": "list"}));
    let bootstrap = request(json!({"op": "bootstrap"}));
    let unnoted = operations.execute(&bootstrap, ListSummaries::Omitted, OpNoting::Unnoted);
    assert_eq!(unnoted.facts, None);
    let noted = |request: &BoardRequest, summaries| {
        let execution = operations.execute(request, summaries, OpNoting::Noted);
        assert!(execution.facts.is_some(), "noted: {request:?}");
        execution
    };
    // Refused before the board exists, then answered.
    assert!(noted(&show, ListSummaries::Omitted).result.is_err());
    assert!(noted(&bootstrap, ListSummaries::Omitted).result.is_ok());
    let created = noted(&create("a"), ListSummaries::Omitted);
    assert!(created.result.is_ok(), "{created:?}");
    assert_eq!(created.facts.and_then(|facts| facts.attempts), Some(1));
    assert!(noted(&create("a"), ListSummaries::Omitted).result.is_err());
    assert!(noted(&show, ListSummaries::Omitted).result.is_ok());
    assert!(noted(&listing, ListSummaries::Wanted).result.is_ok());
    assert!(noted(&show, ListSummaries::Wanted).result.is_err());
}

/// A request that reads the clock carries the note the fixture's clock
/// gives; one that does not carries none.
pub fn check_clock_notes(fixture: Fixture) {
    let operations = &*fixture.operations;
    for value in [json!({"op": "bootstrap"}), json!({"op": "list"})] {
        assert_eq!(
            run(operations, &request(value.clone())).clock_note,
            None,
            "{value}"
        );
    }
    assert_eq!(
        run(operations, &create("a")).clock_note,
        fixture.clock_note,
        "a create reads the clock"
    );
}

/// A front end that launches nothing owes no ends as its session ends.
pub fn check_no_left_runs(fixture: Fixture) {
    assert_eq!(fixture.operations.left_runs(), Ok(vec![]));
}

/// The local adapter over a fresh clone of `remote`, writing as `Ada`, as
/// composition builds it for `quecto board` (no launcher). A local remote
/// gives no outside time, for the reason [`reference_source`] says.
fn local(remote: &Remote) -> Fixture {
    let repo = remote.clone_dir(0).to_path_buf();
    let url = sh(&repo, &["remote", "get-url", "origin"], b"");
    let ReferenceSource::Unavailable(why) = reference_source(&url) else {
        panic!("a local remote gives no time: {url}")
    };
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
        known: Default::default(),
    };
    match build_project_board_handles(&environment) {
        Ok(handles) => Fixture {
            operations: handles.operations,
            clock_note: Some(why),
        },
        Err(error) => panic!("the local board is built: {error:?}"),
    }
}

fn each(check: fn(Fixture)) {
    let remote = Remote::new();
    check(local(&remote));
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
fn local_board_operations_summaries() {
    each(check_summaries);
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
