use super::*;
use crate::application::agent_turn::agent_loop::tool_panic_scope::{ToolScope, scoped};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::QueryBoard;
use crate::composition::bootstrap::project_board_operations::fixtures::{QueryListings, assembled};
use crate::domain::project_board::value_objects::telemetry::OpOutcome;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::recording::tests::{
    Records, outside_text, recording_into,
};
use crate::interface::tools::project_board::session_recording::SessionRecording;
use crate::interface::tools::project_board::site::{
    BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};
use futures::FutureExt;

/// The approved text (memory: board-tool-description-approved, "S2b
/// text"), with only the changes the PR lists to match the ops on master.
const STORED: &str = r#"Project board for this repo: the shared list of tasks people and agents plan, claim and finish. Tasks live on the repo's quecto/board branch and are written only through this tool, never by editing the branch. Each op is one call, {"op":"<name>", ...named fields}, answering JSON: {"ok":true,...} or {"ok":false,"error":{"code","field","message"}}. Ids are slugs (a-z, 0-9, -). Statuses: draft, ready, claimed, in_progress, review, blocked, done, archived.
Read: list(status|parent|dependents_of, at most one filter), show(task), list_ready (ready tasks whose dependencies are done or archived), list_takeable (held tasks whose claim expired). Listings show a summary per task; show(task) gives the whole task, its accepted plan, each plan version in brief and its runs (the latest readable one in full); "may_be_incomplete" means a file that may hold one of its plans or runs is set aside. What does not fit in 64 KiB beside the task (which is never cut, and may take more alone) is cut, with counts of what was left out and approach_next/items_next to read on with show(task, approach_from, items_from). Listings are advice; every change is checked against the board when it is written.
Plan: create(task,title,kind,goal,why,done_when,links,status draft|ready,parent,depends_on); update_draft(task, any of title,goal,why,done_when,links,parent,depends_on, expected_updated) edits a draft only; pass the updated you read as expected_updated so a newer edit is never overwritten. A title is the outcome in at most 70 characters; goal is 1 to 3 sentences, why optional, done_when 1 to 8 outcomes you would check (not steps), links optional, each [text](https://…) or an https:// address; together at most 1,200 characters (move details into the plan or a handoff); update_draft replaces all four together. Anyone may create, edit a draft, move it to ready, or archive a draft, ready or done task, or a review task nobody holds now, with move(task,to archived) (not while a child is held: "children_active"); a task is claimed only once its dependencies are done ("dependency_not_done"). Anyone may move a ready task back to draft. Task and plan text is shared and lasting: describe the work and how it is checked, never an instruction for this session alone (such as not to start yet).
Plans: show(task, version) gives that version of the task's plan whole, a draft included, in place of the accepted one, and pages it with approach_from/items_from as it pages the accepted plan. propose_plan(task, approach, items, team) adds a draft version of a task's plan; items are [{"id","title","criteria":[lines],"depends_on":[item ids]}] and team is null or {"roles":[{"name","model","effort","members","turns"}],"deadline_seconds","image"}, as show gives them. Every page names the plan's version and updated: if either differs from your first read, read again from 0. To propose from a plan you read, read all of its approach and items first (show again with approach_from at approach_next and items_from at items_next until none is given): never propose an approach or items show cut. edit_plan(task, version, any of approach,items,team, expected_updated) edits a draft: a field you leave out stays as it is and one you give replaces it whole (team null removes the team), so items change without resending the approach. Anyone may propose a plan or edit a draft of a task that is not done or archived; pass the draft's updated as expected_updated so a newer edit is never overwritten, and on "plan_changed" re-read it with show(task, version) and redo your edit. accept_plan(task, version, expected_updated, expected_accepted) makes a draft the task's plan and supersedes the plan accepted before (accepting an older draft over it is a rollback; a superseded plan is never accepted again: propose it anew); both pins are required: expected_updated is the updated show(task, version) gave you, and expected_accepted the task's accepted_plan as you read it (null for none), so what you accept is what you read and you never supersede an acceptance you did not see (on "accepted_plan_changed" re-read with show(task, version) and decide again). discard_plan(task, version, expected_updated optional) discards a draft (it is kept, as discarded). On a held task only the claim holder accepts or discards, on an unclaimed one anyone; nothing is accepted while a run of the task is going ("run_open"), only a draft is accepted or discarded ("plan_not_draft"), and on "plans_inconsistent" stop and report: the task's plans contradict each other.
Start: start(task, plan_version) hands a ready task, or one you hold, to a swarm: it claims the task if it is ready, moves it to in_progress, begins its next run on the accepted plan and launches the run's coordinator in a new container with the plan's brief. plan_version is the accepted plan you confirmed from show(task); a task that accepts another by then is refused ("accepted_plan_changed"), so only the plan you confirmed runs. It needs a session that may spawn ("no_launcher"); a plan a swarm cannot take is refused ("brief"). If the coordinator does not launch ("launch_failed"), the start is undone as far as it can be: the run is folded back as failed and a claim the start took is released (with run_state "open" the fold-back failed: end the run with end_run). If launch_failed names still_running, nothing is undone: kill that coordinator, then end_run failed. The answer names the run, the coordinator (agent, environment_ref), the worker roles whose models were not applied (not_applied) and whether the claim was taken (claim: fresh) or yours already (held). The task stays yours while the swarm runs; end it as Runs says.
Work: move(task,to claimed) takes a ready task for 2 hours; renew(task) before it lapses while you work; take_over(task) only once a claim has expired; move(task,to ready) hands it back to ready. Only the claim holder moves a held task: claimed to in_progress|blocked|ready, in_progress to review|blocked|ready, review to in_progress|ready (or done when no pull request is linked), blocked to in_progress|ready. Once its claim has expired, or a task has none, anyone may move a review or blocked task to ready. link_pr(task,pr) and unlink_pr(task,pr) are the holder's. A task with linked pull requests is done only when all of them have merged: link_pr and moving to review check this automatically, and complete(task) asks again (anyone may call it). Never move a task to done by hand while a pull request is linked.
Runs: end_run(task, run, outcome, expected_updated optional) records how a started run ended, in this order: when the run is paused holding an outcome, close it (agent_cmd swarm_control close on the coordinator; a cancelled run needs no close); stop the coordinator with agent_cmd kill (end_run answers "run_live", naming the agent, until it has stopped; it sees only this session's subagents with the run's label); then end_run with "succeeded", "stopped" or "failed". It records the run's task items, owners, evidence and cost from its swarm's board as the coordinator stopped; "succeeded" is refused ("items_not_done") while a task item is not done there. Renew the claim before it lapses (2 hours, not renewed while the swarm runs); if it has expired, renew it if it is yours, or take_over if it is another's, then end_run. A start that answered "launch_failed" with run_state "open" leaves a run to end the same way. When nobody can account for a run, end it as "lost" (the holder, or anyone when the task has no claim). The task stays held: move it on afterwards.
Repair: clear_set_aside(path, object, repeated) removes a file the board could not read (listed under quarantined, with its object); repeated, for a file that appears twice, maps its path to {"keep":object,"seen":[objects]} or {"drop":[objects]}, the objects its copies list. Use it only when told to.
On "stale" re-read with show and redo your change; on "contended" call again; on "clock_ahead" or "task_ahead" stop and report: this machine's clock is wrong. A "clock" note on an answer means writes ran without an outside time check."#;

/// The most bytes a tool's description may take.
const DESCRIPTION_BUDGET_BYTES: usize = 8_192;

#[test]
fn the_description_is_the_stored_text_within_the_budget() {
    let definition = tool(site(None)).definition();
    assert_eq!(definition.name, BOARD_TOOL_NAME);
    assert_eq!(definition.description, STORED);
    assert!(STORED.len() <= DESCRIPTION_BUDGET_BYTES, "{}", STORED.len());
}

/// The tool over `site`, recording into a slot of its own (bound to no
/// session), never the process's.
pub(super) fn tool(site: BoardSite) -> BoardTool {
    BoardTool::recorded_in(site, Arc::default())
}

/// The tool over `site`, recording into `records`.
pub(super) fn recorded_tool(site: BoardSite, records: &Arc<Records>) -> BoardTool {
    let recordings = Arc::new(SessionRecording::default());
    recordings.bind(Some(recording_into(records)));
    BoardTool::recorded_in(site, recordings)
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
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

pub(super) fn site(
    build: Option<crate::interface::tools::project_board::site::ProjectBoardHandlesBuilder>,
) -> BoardSite {
    BoardSite {
        repo: std::path::PathBuf::from("."),
        remote: "origin".into(),
        known: Default::default(),
        build,
        launcher: None,
        swarm_session: None,
    }
}

/// An op goes through the tool to the board and back as its JSON answer;
/// a refused one is an error result the model can act on.
#[tokio::test]
async fn an_op_round_trips_through_the_tool() {
    let tool = tool(site(Some(memory_board)));
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
    let records = Arc::new(Records::default());
    let tool = recorded_tool(site(Some(panicking)), &records);
    let call = std::panic::AssertUnwindSafe(scoped(
        ToolScope::new(BOARD_TOOL_NAME),
        tool.execute(r#"{"op":"list"}"#),
    ))
    .catch_unwind()
    .await;
    let payload = call.expect_err("the panic resumed in the call");
    let message = crate::application::agent_turn::agent_loop::tool_panic_scope::payload_message(
        payload.as_ref(),
    );
    assert!(message.ends_with("board boom"), "{message}");
    // Review round 1: the call is recorded as it unwinds, as `internal`.
    let recorded: Vec<(&str, Option<&str>)> = records
        .taken()
        .iter()
        .map(|record| match &record.outcome {
            OpOutcome::Refused { code, .. } => (record.op.as_str(), Some(code.as_str())),
            OpOutcome::Ok => (record.op.as_str(), None),
        })
        .collect();
    assert_eq!(recorded, [("list", Some("internal"))]);
}

/// A board of `count` ready tasks with long titles, built afresh.
fn crowded_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let handles = memory_board(&environment())?;
    for index in 0..300 {
        let request = json!({"op": "create", "task": format!("t{index}"), "title": "x".repeat(70), "goal": "d".repeat(500), "done_when": ["done"], "status": "ready"});
        let created =
            crate::interface::tools::project_board::run(&*handles.operations, &request.to_string());
        assert_eq!(created["ok"], json!(true), "{created}");
    }
    Ok(handles)
}

fn environment() -> ProjectBoardEnvironment {
    ProjectBoardEnvironment {
        repo: std::path::PathBuf::from("."),
        remote: "origin".into(),
        known: Default::default(),
        access: crate::interface::tools::project_board::site::Access::Read,
        vars: vec![],
        cancelled: None,
        launcher: None,
        swarm_session: None,
    }
}

/// A listing shows a summary per task, and its answer stays within the
/// cap, ending with what it left out.
#[tokio::test(flavor = "multi_thread")]
async fn a_listing_shows_summaries_within_the_cap() {
    let tool = tool(site(Some(crowded_board)));
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
    let tool = tool(site(Some(memory_board)));
    let refused = tool.execute(r#"{"op":"bootstrap"}"#).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert!(refused.is_error, "{answer}");
    assert_eq!(answer["error"]["field"], json!("op"));
}

/// The schema names every op the tool takes, and types and words their
/// fields as the parser reads them.
#[test]
fn the_schema_is_the_parsers() {
    let definition = tool(site(None)).definition();
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
    // A plan's nested fields are typed as `show` gives them (#2481 M4b).
    let item = &schema["properties"]["items"]["items"];
    assert_eq!(item["required"], json!(["id", "title"]));
    assert_eq!(item["additionalProperties"], json!(false));
    let team = &schema["properties"]["team"];
    assert_eq!(team["type"], json!(["object", "null"]));
    // A team sets no token budget, and its deadline is seconds from Start
    // (#2568 review round 2).
    assert_eq!(
        team["properties"]["deadline_seconds"],
        json!({"type": ["integer", "null"], "minimum": 600, "maximum": 604_800})
    );
    let keys: Vec<&String> = team["properties"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["roles", "deadline_seconds", "image"]);
    let role = &team["properties"]["roles"]["items"];
    assert_eq!(
        role["required"],
        json!(["name", "model", "effort", "members"])
    );
    assert_eq!(
        role["properties"]["effort"]["enum"],
        json!(["low", "medium", "high", "max"])
    );
    let version = &schema["properties"]["version"];
    assert_eq!(
        (&version["minimum"], &version["maximum"]),
        (&json!(1), &json!(999))
    );
}

/// #2481 M4b: the tool takes the plan ops through to the board (a board
/// built afresh per call holds no task to plan, so the board answers).
#[tokio::test]
async fn the_tool_takes_the_plan_ops() {
    let tool = tool(site(Some(memory_board)));
    for request in [
        json!({"op": "propose_plan", "task": "a", "approach": "A", "items": []}),
        json!({"op": "edit_plan", "task": "a", "version": 1, "team": null}),
    ] {
        let result = tool.execute(&request.to_string()).await.unwrap();
        let answer: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            (&answer["op"], &answer["error"]["code"]),
            (&request["op"], &json!("not_found")),
            "{answer}"
        );
    }
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
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

/// A non-listing answer is the shared path's, unchanged: no key added, a
/// refusal and a success alike.
#[tokio::test(flavor = "multi_thread")]
async fn a_non_listing_answer_is_the_shared_paths() {
    let tool = tool(site(Some(seeded_board)));
    for text in [
        r#"{"op":"show","task":"nope"}"#,
        r#"{"op":"show","task":"a"}"#,
        r#"{"op":"create","task":"b","title":"B","description":"d","status":"draft"}"#,
    ] {
        let answer: Value =
            serde_json::from_str(&tool.execute(text).await.unwrap().content).unwrap();
        let shared = crate::interface::tools::project_board::run(
            &*seeded_board(&environment()).unwrap().operations,
            text,
        );
        assert_eq!(answer, shared, "{text}");
    }
}

/// An op the tool does not take is refused with the ones it does, never
/// named.
#[tokio::test]
async fn an_op_the_tool_does_not_take_is_refused_with_its_ops() {
    let tool = tool(site(Some(memory_board)));
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
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

/// The files a listing shows set aside are within the cap too, ending with
/// how many more there are.
#[tokio::test(flavor = "multi_thread")]
async fn a_listing_caps_what_it_shows_set_aside() {
    let tool = tool(site(Some(quarantined_board)));
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
    // After the creates: a write drops the strays set aside before it.
    for index in 0..300 {
        store.set_aside_stray(
            &format!("tasks/stray-{index}-{}", "p".repeat(200)),
            "a stray",
        );
    }
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

/// Tasks take the room first: files set aside never crowd a listing's
/// tasks out; they get what is left, ending with how many more there are.
#[tokio::test(flavor = "multi_thread")]
async fn a_listing_shows_its_tasks_before_what_was_set_aside() {
    let tool = tool(site(Some(mixed_board)));
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
/// the board's store checks before it pushes; its work still records it,
/// `cancelled` (epic #2481 T3).
#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_call_sets_its_cancel_flag() {
    use crate::infrastructure::observability::audit_log::AuditLog;
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "dropped").unwrap();
    let recordings = Arc::new(SessionRecording::default());
    let build = crate::composition::bootstrap::project_board::session_board_recording_decided;
    let recording = build(true, &log);
    assert!(recording.is_some(), "on, with a log");
    recordings.bind(recording);
    let tool = BoardTool::recorded_in(site(Some(waiting_board)), recordings);
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
    let file = AuditLog::file_path(base.path(), "dropped");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let record = loop {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let found = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|line| line["event"] == "board_op");
        match found {
            Some(record) => break record,
            None if std::time::Instant::now() < until => {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            None => panic!("the dropped call's work recorded nothing"),
        }
    };
    assert_eq!(
        (&record["op"], &record["code"], &record["cancelled"]),
        (&json!("list"), &json!("store_unavailable"), &json!(true)),
        "{record}"
    );
    assert!(record["build_us"].is_u64(), "{record}");
}

/// Review round 1: a request the tool does not take records its op, and
/// a request that is no object records `request`, as the shared path
/// does; the answers are unchanged.
#[tokio::test]
async fn a_request_the_tool_refuses_itself_is_recorded_as_the_shared_path_would() {
    let records = Arc::new(Records::default());
    let tool = recorded_tool(site(None), &records);
    let cases = [
        (
            r#"{"op": "bootstrap"}"#,
            ("bootstrap", "write", "op", "invalid_value"),
        ),
        (r#"{"task": "a"}"#, ("unknown", "read", "op", "missing")),
        (r#"{"op": 3}"#, ("unknown", "read", "op", "wrong_type")),
        (r#"["op"]"#, ("unknown", "read", "request", "wrong_type")),
    ];
    // Review round 3 (#2567): a start the strict read refuses records its
    // op, as its answer names it.
    let duplicate = r#"{"op": "start", "task": "a", "task": "b", "plan_version": 1}"#;
    let answer: Value =
        serde_json::from_str(&tool.execute(duplicate).await.unwrap().content).unwrap();
    assert_eq!(answer["op"], json!("start"), "{answer}");
    let line = serde_json::to_value(records.taken().remove(0)).unwrap();
    assert_eq!(
        (&line["op"], &line["access"], &line["field"], &line["fault"]),
        (
            &json!("start"),
            &json!("write"),
            &json!("request"),
            &json!("duplicate_key")
        ),
        "{line}"
    );
    for (text, (op, access, field, fault)) in cases {
        let answer: Value =
            serde_json::from_str(&tool.execute(text).await.unwrap().content).unwrap();
        assert_eq!(
            answer["error"]["field"],
            json!("op"),
            "{text}: answered as ever"
        );
        let record = records.taken().remove(0);
        let line = serde_json::to_value(&record).unwrap();
        assert_eq!(
            (
                &line["op"],
                &line["access"],
                &line["code"],
                &line["field"],
                &line["fault"]
            ),
            (
                &json!(op),
                &json!(access),
                &json!("invalid_request"),
                &json!(field),
                &json!(fault)
            ),
            "{text}"
        );
    }
}

/// Review round 1, the property test at the tool's entry point: random
/// titles, descriptions and keys never appear in a record, whichever way
/// the tool answers.
#[tokio::test(flavor = "multi_thread")]
async fn no_request_text_reaches_a_record_through_the_tool() {
    use rand::{Rng, SeedableRng};
    let seed: u64 = rand::thread_rng().r#gen();
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let records = Arc::new(Records::default());
    let tool = recorded_tool(site(Some(memory_board)), &records);
    for case in 0..16 {
        let (title, key) = (outside_text(&mut rng), outside_text(&mut rng));
        let requests = [
            json!({"op": "create", "task": format!("t-{case}"), "title": title, "description": key}),
            json!({"op": "list", key.clone(): title}),
            json!({"op": key.clone(), "title": title}),
            json!({"op": "bootstrap", "title": title}),
            json!([title, key]),
            json!({key.clone(): title}),
        ];
        for request in requests {
            tool.execute(&request.to_string()).await.unwrap();
            let lines: Vec<String> = records
                .taken()
                .iter()
                .map(|record| serde_json::to_string(record).unwrap())
                .collect();
            assert_eq!(lines.len(), 1, "one record per call: {request}");
            for outside in [&title, &key] {
                assert!(
                    !lines[0].contains(outside.as_str()),
                    "seed {seed}: {outside:?} reached {}",
                    lines[0]
                );
            }
        }
    }
}

/// Review round 2: a call dropped before it was ever polled reached no
/// board, and writes no record (nor any from the async worker).
#[test]
fn a_call_dropped_before_it_runs_writes_no_record() {
    let records = Arc::new(Records::default());
    let tool = recorded_tool(site(Some(memory_board)), &records);
    drop(tool.execute(r#"{"op": "list"}"#));
    assert!(records.taken().is_empty());
}

/// Review round 3: a call's duration runs from its arrival, its queueing
/// before its work started included.
#[tokio::test(flavor = "multi_thread")]
async fn a_calls_duration_covers_its_wait_before_its_work_ran() {
    let records = Arc::new(Records::default());
    let tool = recorded_tool(site(None), &records);
    let call = tool.execute(r#"{"op": "list"}"#);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    call.await.unwrap();
    let record = records.taken().remove(0);
    assert!(record.duration_us >= 50_000, "{}", record.duration_us);
}

/// The board ops the parent playbook calls, `op(…)` in backticks (#2481
/// Q11): exactly these, a contract on the playbook's op names, and every
/// one an op this tool takes, so a renamed op cannot leave the playbook
/// calling one the tool refuses.
#[test]
fn the_parent_playbook_calls_only_the_tools_ops() {
    let called: std::collections::BTreeSet<&str> =
        include_str!("../../../../../PARENT_PLAYBOOK.md")
            .split('`')
            .skip(1)
            .step_by(2)
            .filter_map(|code| code.split_once('(').map(|(name, _)| name))
            .filter(|name| {
                name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                    && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            })
            .collect();
    let expected = [
        "accept_plan",
        "complete",
        "edit_plan",
        "end_run",
        "link_pr",
        "move",
        "propose_plan",
        "renew",
        "show",
        "start",
        "take_over",
    ];
    assert_eq!(called, expected.into_iter().collect());
    for op in &called {
        assert!(TOOL_OPS.contains(op), "{op} is an op of the tool");
    }
}
