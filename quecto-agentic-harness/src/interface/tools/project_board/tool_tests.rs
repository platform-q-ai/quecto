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
Plan: create(task,title,kind,description,status draft|ready,parent,depends_on); update_draft(task, any of title,description,parent,depends_on, expected_updated) edits a draft only; pass the updated you read as expected_updated so a newer edit is never overwritten. Anyone may create, edit a draft, move it to ready, or archive a draft, ready or done task with move(task,to archived).
Work: move(task,to claimed) takes a ready task for 2 hours; renew(task) before it lapses while you work; take_over(task) only once a claim has expired; move(task,to ready) hands it back to ready. Only the claim holder moves a held task: claimed to in_progress|blocked|ready, in_progress to review|blocked|ready, review to in_progress (or done when no pull request is linked), blocked to in_progress. Once its claim has expired, anyone may move a review or blocked task to ready. link_pr(task,pr) and unlink_pr(task,pr) are the holder's. A task with linked pull requests is done only when all of them have merged: link_pr and moving to review check this automatically, and complete(task) asks again (anyone may call it). Never move a task to done by hand while a pull request is linked.
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
