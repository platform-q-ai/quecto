use super::*;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tool_panic_scope::{ToolScope, scoped};
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::site::{
    BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};
use futures::FutureExt;

/// The approved text (memory: board-tool-description-approved, "S2b
/// text"), with only the changes the PR lists to match the ops on master.
const STORED: &str = r#"Project board for this repo: the shared list of tasks people and agents plan, claim and finish. Tasks live on the repo's quecto/board branch and are written only through this tool, never by editing the branch. Each op is one call, {"op":"<name>", ...named fields}, answering JSON: {"ok":true,...} or {"ok":false,"error":{"code","field","message"}}. Ids are slugs (a-z, 0-9, -). Statuses: draft, ready, claimed, in_progress, review, blocked, done, archived.
Read: list(status|parent|dependents_of, at most one filter), show(task), list_ready (ready tasks whose dependencies are done or archived), list_takeable (held tasks whose claim expired). Listings show a summary per task; show(task) gives the whole task. Listings are advice; every change is checked against the board when it is written.
Plan: create(task,title,kind,description,status draft|ready,parent,depends_on); update_draft(task, any of title,description,parent,depends_on, expected_updated) edits a draft only; pass the updated you read as expected_updated so a newer edit is never overwritten. Anyone may create, edit a draft, move it to ready, or archive a draft, ready or done task with move(task,to archived). Anyone may move a ready task back to draft.
Work: move(task,to claimed) takes a ready task for 2 hours; renew(task) before it lapses while you work; take_over(task) only once a claim has expired; move(task,to ready) hands it back to ready. Only the claim holder moves a held task: claimed to in_progress|blocked|ready, in_progress to review|blocked|ready, review to in_progress|ready (or done when no pull request is linked), blocked to in_progress|ready. Once its claim has expired, or a task has none, anyone may move a review or blocked task to ready. link_pr(task,pr) and unlink_pr(task,pr) are the holder's. A task with linked pull requests is done only when all of them have merged: link_pr and moving to review check this automatically, and complete(task) asks again (anyone may call it). Never move a task to done by hand while a pull request is linked.
Repair: clear_set_aside(path, object, repeated) removes a file the board could not read (listed under quarantined, with its object); repeated, for a task file that appears twice, maps its path to {"keep":object,"seen":[objects]} or {"drop":[objects]}, the objects its copies list. Use it only when told to.
On "stale" re-read with show and redo your change; on "contended" call again; on "clock_ahead" or "task_ahead" stop and report: this machine's clock is wrong. A "clock" note on an answer means writes ran without an outside time check."#;

/// The most bytes a tool's description may take.
const DESCRIPTION_BUDGET_BYTES: usize = 8_192;

#[test]
fn the_description_is_the_stored_text_within_the_budget() {
    let definition = BoardTool::new(site(None)).definition();
    assert_eq!(definition.name, BOARD_TOOL_NAME);
    assert_eq!(definition.description, STORED);
    assert!(STORED.len() <= DESCRIPTION_BUDGET_BYTES, "{}", STORED.len());
}

/// A board in memory, built afresh for each request.
fn memory_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

fn site(
    build: Option<crate::interface::tools::project_board::site::ProjectBoardHandlesBuilder>,
) -> BoardSite {
    BoardSite {
        repo: std::path::PathBuf::from("."),
        remote: "origin".into(),
        build,
    }
}

/// An op goes through the tool to the board and back as its JSON answer;
/// a refused one is an error result the model can act on.
#[tokio::test]
async fn an_op_round_trips_through_the_tool() {
    let tool = BoardTool::new(site(Some(memory_board)));
    let created = tool
        .execute(r#"{"op":"create","task":"a","title":"A","status":"ready"}"#)
        .await
        .unwrap();
    let answer: Value = serde_json::from_str(&created.content).unwrap();
    assert!(!created.is_error, "{answer}");
    assert_eq!(answer["ok"], json!(true));
    assert_eq!(answer["task"]["id"], json!("a"));
    let refused = tool.execute(r#"{"op":"claim","task":"a"}"#).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert!(refused.is_error, "{answer}");
    assert_eq!(answer["error"]["field"], json!("op"));
}

/// Board work that panics fails its call, as the agent loop contains it
/// (ADR-0029): the panic resumes in the call, never on a detached thread
/// the process would abort on.
#[tokio::test]
async fn a_panic_in_board_work_fails_the_call() {
    fn panicking(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
        panic!("board boom")
    }
    let tool = BoardTool::new(site(Some(panicking)));
    let call = std::panic::AssertUnwindSafe(scoped(
        ToolScope::new(BOARD_TOOL_NAME),
        tool.execute(r#"{"op":"list"}"#),
    ))
    .catch_unwind()
    .await;
    let payload = call.expect_err("the panic resumed in the call");
    let message = crate::application::tool_panic_scope::payload_message(payload.as_ref());
    assert!(message.ends_with("board boom"), "{message}");
}

/// A board of `count` ready tasks with long titles, built afresh.
fn crowded_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let handles = memory_board(&environment())?;
    for index in 0..300 {
        let request = json!({"op": "create", "task": format!("t{index}"), "title": "x".repeat(200), "description": "d".repeat(500), "status": "ready"});
        let created =
            crate::interface::tools::project_board::run(&handles.operations, &request.to_string());
        assert_eq!(created["ok"], json!(true), "{created}");
    }
    Ok(handles)
}

fn environment() -> ProjectBoardEnvironment {
    ProjectBoardEnvironment {
        repo: std::path::PathBuf::from("."),
        remote: "origin".into(),
        access: crate::interface::tools::project_board::site::Access::Read,
        vars: vec![],
        cancelled: None,
    }
}

/// A listing shows a summary per task, and its answer stays within the
/// cap, ending with what it left out.
#[tokio::test(flavor = "multi_thread")]
async fn a_listing_shows_summaries_within_the_cap() {
    let tool = BoardTool::new(site(Some(crowded_board)));
    for op in ["list", "list_ready"] {
        let listed = tool.execute(&json!({"op": op}).to_string()).await.unwrap();
        assert!(!listed.is_error, "{}", listed.content);
        assert!(
            listed.content.len() <= MAX_LISTING_BYTES,
            "{}",
            listed.content.len()
        );
        let answer: Value = serde_json::from_str(&listed.content).unwrap();
        let tasks = answer["tasks"].as_array().unwrap();
        let mut keys: Vec<&str> = tasks[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "claim", "id", "kind", "parent", "status", "title", "updated"
            ]
        );
        assert_eq!(answer["truncated"]["shown"], json!(tasks.len()));
        assert_eq!(answer["truncated"]["more"], json!(300 - tasks.len()));
        assert!(tasks.len() > 50, "{}", tasks.len());
    }
}

/// Creating a board is the user's or the CLI's: the tool refuses it.
#[tokio::test]
async fn the_tool_does_not_bootstrap_a_board() {
    let tool = BoardTool::new(site(Some(memory_board)));
    let refused = tool.execute(r#"{"op":"bootstrap"}"#).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert!(refused.is_error, "{answer}");
    assert_eq!(answer["error"]["field"], json!("op"));
}

/// The schema names every op the tool takes, and types and words their
/// fields as the parser reads them.
#[test]
fn the_schema_is_the_parsers() {
    let definition = BoardTool::new(site(None)).definition();
    let schema: Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    let ops: Vec<&str> = OPS
        .iter()
        .map(|(op, _)| *op)
        .filter(|op| *op != "bootstrap")
        .collect();
    assert_eq!(schema["properties"]["op"]["enum"], json!(ops));
    assert_eq!(schema["properties"]["pr"]["type"], json!("integer"));
    assert_eq!(
        schema["properties"]["parent"]["type"],
        json!(["string", "null"])
    );
    let statuses: Vec<&str> = TaskStatus::ALL
        .iter()
        .map(|status| status.as_str())
        .collect();
    assert_eq!(schema["properties"]["to"]["enum"], json!(statuses));
    assert_eq!(schema["properties"]["status"]["enum"], json!(statuses));
    let kinds: Vec<&str> = TaskKind::ALL.iter().map(|kind| kind.as_str()).collect();
    assert_eq!(schema["properties"]["kind"]["enum"], json!(kinds));
}

/// A clock reading one time, with no outside reference.
struct Fixed;

impl crate::application::project_board::ports::BoardClock for Fixed {
    fn now(
        &self,
    ) -> Result<
        crate::domain::project_board::value_objects::timestamp::Timestamp,
        crate::application::project_board::ports::BoardClockError,
    > {
        Ok(
            crate::domain::project_board::value_objects::timestamp::Timestamp::parse(
                "2026-10-12T00:00:00Z",
            )
            .unwrap(),
        )
    }

    fn reference(
        &self,
    ) -> Option<crate::domain::project_board::value_objects::timestamp::Timestamp> {
        None
    }

    fn reference_status(&self) -> crate::application::project_board::ports::ReferenceStatus {
        crate::application::project_board::ports::ReferenceStatus::NoReference(
            "no outside time in this test".into(),
        )
    }

    fn pause(&self, _: usize) {}

    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

/// A board in memory at a fixed time holding one ready task, `a`, built
/// afresh for each request: two such boards answer a request alike.
fn seeded_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(Fixed),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    let created = crate::interface::tools::project_board::run(
        &operations,
        r#"{"op":"create","task":"a","title":"A","status":"ready"}"#,
    );
    assert_eq!(created["ok"], json!(true), "{created}");
    Ok(ProjectBoardHandles { operations })
}

/// A non-listing answer is the shared path's, unchanged: no key added, a
/// refusal and a success alike.
#[tokio::test(flavor = "multi_thread")]
async fn a_non_listing_answer_is_the_shared_paths() {
    let tool = BoardTool::new(site(Some(seeded_board)));
    for text in [
        r#"{"op":"show","task":"nope"}"#,
        r#"{"op":"show","task":"a"}"#,
        r#"{"op":"create","task":"b","title":"B","description":"d","status":"draft"}"#,
    ] {
        let answer: Value =
            serde_json::from_str(&tool.execute(text).await.unwrap().content).unwrap();
        let shared = crate::interface::tools::project_board::run(
            &seeded_board(&environment()).unwrap().operations,
            text,
        );
        assert_eq!(answer, shared, "{text}");
    }
}

/// An op the tool does not take is refused with the ones it does, never
/// named.
#[tokio::test]
async fn an_op_the_tool_does_not_take_is_refused_with_its_ops() {
    let tool = BoardTool::new(site(Some(memory_board)));
    for op in ["nope", "bootstrap"] {
        let refused = tool.execute(&json!({"op": op}).to_string()).await.unwrap();
        let answer: Value = serde_json::from_str(&refused.content).unwrap();
        assert!(refused.is_error, "{answer}");
        assert_eq!(answer["error"]["field"], json!("op"));
        let message = answer["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("list_takeable") && !message.contains("bootstrap"),
            "{message}"
        );
    }
}

/// A board with many files set aside, each with a long path.
fn quarantined_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    for index in 0..300 {
        store.set_aside_stray(
            &format!("tasks/stray-{index}-{}", "p".repeat(200)),
            "a stray",
        );
    }
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

/// The files a listing shows set aside are within the cap too, ending with
/// how many more there are.
#[tokio::test(flavor = "multi_thread")]
async fn a_listing_caps_what_it_shows_set_aside() {
    let tool = BoardTool::new(site(Some(quarantined_board)));
    let listed = tool.execute(r#"{"op":"list"}"#).await.unwrap();
    assert!(
        listed.content.len() <= MAX_LISTING_BYTES,
        "{}",
        listed.content.len()
    );
    let answer: Value = serde_json::from_str(&listed.content).unwrap();
    let shown = answer["quarantined"].as_array().unwrap().len();
    assert!(shown > 10, "{shown}");
    assert_eq!(answer["quarantined_more"], json!(300 - shown));
}

/// A board with a few ready tasks and many long files set aside.
fn mixed_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    for index in 0..300 {
        store.set_aside_stray(
            &format!("tasks/stray-{index}-{}", "p".repeat(200)),
            "a stray",
        );
    }
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    for index in 0..5 {
        let request =
            json!({"op": "create", "task": format!("t{index}"), "title": "T", "status": "ready"});
        let created =
            crate::interface::tools::project_board::run(&operations, &request.to_string());
        assert_eq!(created["ok"], json!(true), "{created}");
    }
    Ok(ProjectBoardHandles { operations })
}

/// Tasks take the room first: files set aside never crowd a listing's
/// tasks out; they get what is left, ending with how many more there are.
#[tokio::test(flavor = "multi_thread")]
async fn a_listing_shows_its_tasks_before_what_was_set_aside() {
    let tool = BoardTool::new(site(Some(mixed_board)));
    for op in ["list", "list_ready"] {
        let listed = tool.execute(&json!({"op": op}).to_string()).await.unwrap();
        assert!(listed.content.len() <= MAX_LISTING_BYTES, "{op}");
        let answer: Value = serde_json::from_str(&listed.content).unwrap();
        assert_eq!(
            answer["tasks"].as_array().unwrap().len(),
            5,
            "{op}: {answer}"
        );
        assert_eq!(answer.get("truncated"), None, "{op}");
        let files = answer
            .get("quarantined")
            .unwrap_or_else(|| panic!("{op} shows what was set aside: {answer}"));
        let shown = files.as_array().unwrap().len();
        assert!(shown > 10, "{op}: {shown}");
        assert_eq!(answer["quarantined_more"], json!(300 - shown), "{op}");
    }
}

static SEEN_CANCEL: std::sync::Mutex<Option<Arc<std::sync::atomic::AtomicBool>>> =
    std::sync::Mutex::new(None);

/// Signalled once the waiting board's work has its flag (a stored permit
/// if nothing waits yet).
static WORK_STARTED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Board work that waits, holding the call's cancel flag, until it is set.
fn waiting_board(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let flag = environment
        .cancelled
        .clone()
        .expect("the tool passes a cancel flag");
    *SEEN_CANCEL.lock().unwrap() = Some(Arc::clone(&flag));
    WORK_STARTED.notify_one();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !flag.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Err(BuildError::Unavailable("cancelled".into()))
}

/// A call dropped while its work runs (its turn interrupted) sets the flag
/// the board's store checks before it pushes.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_call_sets_its_cancel_flag() {
    let tool = BoardTool::new(site(Some(waiting_board)));
    let call = tool.execute(r#"{"op":"list"}"#);
    // Run the call until its work has started, then drop it.
    tokio::select! {
        ended = call => panic!("the call ended before it was dropped: {ended:?}"),
        () = WORK_STARTED.notified() => {}
    }
    let flag = SEEN_CANCEL.lock().unwrap().clone().expect("the work ran");
    assert!(
        flag.load(std::sync::atomic::Ordering::SeqCst),
        "dropping the call set the flag"
    );
}
