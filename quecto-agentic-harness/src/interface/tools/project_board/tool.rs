//! The agent `board` tool (#2477 S2b): a parent agent's front end on its
//! repository's project board. A call's arguments are the request, the
//! same JSON object the `quecto board` CLI reads, answered through the one
//! shared path ([`site::answer`]) on the repository the agent works in.
//!
//! Unlike the CLI's, its answers are the agent's to read: a listing shows
//! a summary per task within [`MAX_LISTING_BYTES`], and creating a board
//! (`bootstrap`) is left to the user and the CLI. Its work runs off the
//! async workers in the call's panic scope (ADR-0029). A call that is
//! cancelled (its turn interrupted) pushes nothing more to the board; a
//! read already under way may still finish. That leftover work runs on
//! outside the dropped call's panic scope: a panic in it is no call's to
//! answer, so it still aborts the process (ADR-0029's backstop does not
//! reach it).
//!
//! Each call is recorded (epic #2481 T3) in its session's event log while
//! that log is on ([`SessionRecording`]): one `board_op` per call, written
//! on the call's own thread once its answer is summarised, `cancelled`
//! when the call was dropped first. A call whose work panicked writes
//! none.
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value, json};

use super::read;
use super::recording::BoardCall;
use super::request::{Kind, OPS, RequestError};
use super::session_recording::SessionRecording;
use super::site::{self, BoardSite};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::project_board::value_objects::telemetry::{Client, FaultKind, FrontEnd};
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::tools::call_work::off_the_workers;

/// The tool's name.
pub const BOARD_TOOL_NAME: &str = "board";

/// The tool's description: the approved text, with only the changes the
/// ops on master need and the ones the owner approved (#2477 S2b, #2513;
/// listed in its pull request).
pub const BOARD_TOOL_DESCRIPTION: &str = r#"Project board for this repo: the shared list of tasks people and agents plan, claim and finish. Tasks live on the repo's quecto/board branch and are written only through this tool, never by editing the branch. Each op is one call, {"op":"<name>", ...named fields}, answering JSON: {"ok":true,...} or {"ok":false,"error":{"code","field","message"}}. Ids are slugs (a-z, 0-9, -). Statuses: draft, ready, claimed, in_progress, review, blocked, done, archived.
Read: list(status|parent|dependents_of, at most one filter), show(task), list_ready (ready tasks whose dependencies are done or archived), list_takeable (held tasks whose claim expired). Listings show a summary per task; show(task) gives the whole task. Listings are advice; every change is checked against the board when it is written.
Plan: create(task,title,kind,description,status draft|ready,parent,depends_on); update_draft(task, any of title,description,parent,depends_on, expected_updated) edits a draft only; pass the updated you read as expected_updated so a newer edit is never overwritten. Anyone may create, edit a draft, move it to ready, or archive a draft, ready or done task with move(task,to archived). Anyone may move a ready task back to draft.
Work: move(task,to claimed) takes a ready task for 2 hours; renew(task) before it lapses while you work; take_over(task) only once a claim has expired; move(task,to ready) hands it back to ready. Only the claim holder moves a held task: claimed to in_progress|blocked|ready, in_progress to review|blocked|ready, review to in_progress|ready (or done when no pull request is linked), blocked to in_progress|ready. Once its claim has expired, or a task has none, anyone may move a review or blocked task to ready. link_pr(task,pr) and unlink_pr(task,pr) are the holder's. A task with linked pull requests is done only when all of them have merged: link_pr and moving to review check this automatically, and complete(task) asks again (anyone may call it). Never move a task to done by hand while a pull request is linked.
Repair: clear_set_aside(path, object, repeated) removes a file the board could not read (listed under quarantined, with its object); repeated, for a task file that appears twice, maps its path to {"keep":object,"seen":[objects]} or {"drop":[objects]}, the objects its copies list. Use it only when told to.
On "stale" re-read with show and redo your change; on "contended" call again; on "clock_ahead" or "task_ahead" stop and report: this machine's clock is wrong. A "clock" note on an answer means writes ran without an outside time check."#;

/// The most bytes a listing's answer takes.
pub const MAX_LISTING_BYTES: usize = 32 * 1024;

/// The ops whose answers list tasks.
const LISTINGS: &[&str] = &["list", "list_ready", "list_takeable"];

/// The ops the tool takes, an allowlist: every op of [`OPS`] but
/// `bootstrap`, which creates a board (the user's or the CLI's).
const TOOL_OPS: &[&str] = &[
    "list",
    "list_ready",
    "list_takeable",
    "show",
    "create",
    "update_draft",
    "move",
    "renew",
    "take_over",
    "link_pr",
    "unlink_pr",
    "complete",
    "clear_set_aside",
];

/// The ops of [`OPS`] the tool takes, with their fields.
fn tool_ops() -> impl Iterator<Item = &'static (&'static str, &'static [super::request::Field])> {
    OPS.iter().filter(|(op, _)| TOOL_OPS.contains(op))
}

/// The agent `board` tool over one repository's board.
pub struct BoardTool {
    site: Arc<BoardSite>,
    recordings: Arc<SessionRecording>,
}

impl BoardTool {
    /// The tool, recording its calls in the process's session
    /// ([`SessionRecording::process`]).
    pub fn new(site: BoardSite) -> Self {
        Self::recorded_in(site, SessionRecording::process())
    }

    /// The tool, recording its calls as `recordings` is bound.
    pub fn recorded_in(site: BoardSite, recordings: Arc<SessionRecording>) -> Self {
        Self {
            site: Arc::new(site),
            recordings,
        }
    }
}

/// The JSON type, and words, a field of `kind` takes.
fn field_schema(kind: Kind) -> Value {
    let statuses: Vec<&str> = TaskStatus::ALL
        .iter()
        .map(|status| status.as_str())
        .collect();
    let kinds: Vec<&str> = TaskKind::ALL.iter().map(|kind| kind.as_str()).collect();
    match kind {
        Kind::Slug | Kind::Text | Kind::Timestamp | Kind::Path | Kind::ObjectId => {
            json!({"type": "string"})
        }
        // A listing's status is any; a new task's is draft or ready, which
        // the parser checks: one name, one list of words.
        Kind::Status | Kind::NewStatus => json!({"type": "string", "enum": statuses}),
        Kind::TaskKind => json!({"type": "string", "enum": kinds}),
        Kind::OptionalSlug => json!({"type": ["string", "null"]}),
        Kind::Slugs => json!({"type": "array", "items": {"type": "string"}}),
        Kind::Pr => json!({"type": "integer", "minimum": 1}),
        // A path to {"keep", "seen"} or {"drop"}: the portable schema
        // subset types no values under names it does not know, so the
        // description gives the shape.
        Kind::Repeated => json!({"type": "object"}),
    }
}

/// The parameters every op the tool takes may name, typed, from the one
/// table of ops ([`OPS`]): a name two ops type differently takes both
/// types.
fn parameters_schema() -> String {
    let mut properties = Map::new();
    let ops: Vec<&str> = tool_ops().map(|(op, _)| *op).collect();
    properties.insert("op".into(), json!({"type": "string", "enum": ops}));
    for field in tool_ops().flat_map(|(_, fields)| fields.iter()) {
        let schema = field_schema(field.kind);
        match properties.get(field.name) {
            Some(known) if *known != schema => {
                let types = [known, &schema].map(|schema| schema["type"].clone());
                let merged: Vec<Value> = types
                    .iter()
                    .flat_map(|types| match types {
                        Value::Array(types) => types.clone(),
                        single => vec![single.clone()],
                    })
                    .fold(vec![], |mut merged, kind| {
                        if !merged.contains(&kind) {
                            merged.push(kind);
                        }
                        merged
                    });
                properties.insert(field.name.into(), json!({"type": merged}));
            }
            Some(_) => {}
            None => {
                properties.insert(field.name.into(), schema);
            }
        }
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["op"],
        "additionalProperties": false,
    })
    .to_string()
}

/// What a listing shows of a task: who and what it is, where it stands
/// and who holds it until when; `show` gives the whole task.
fn summary(task: &Value) -> Value {
    let claim = task["claim"]
        .as_object()
        .map(|claim| json!({"holder": claim["holder"], "expires": claim["expires"]}));
    json!({
        "id": task["id"],
        "title": task["title"],
        "kind": task["kind"],
        "status": task["status"],
        "parent": task["parent"],
        "claim": claim,
        "updated": task["updated"],
    })
}

/// `answer`, a listing's tasks as summaries and the files it shows set
/// aside, as many of each as fit in [`MAX_LISTING_BYTES`] (its tasks
/// first, the files in what is left), each list ending with what was left
/// out; any other answer exactly as it is.
fn summarised(mut answer: Value) -> Value {
    let listing = answer["ok"] == json!(true)
        && answer["op"]
            .as_str()
            .is_some_and(|op| LISTINGS.contains(&op))
        && answer.get("tasks").is_some_and(Value::is_array);
    if !listing {
        return answer;
    }
    let tasks: Vec<Value> = match answer["tasks"].take() {
        Value::Array(tasks) => tasks.iter().map(summary).collect(),
        other => unreachable!("a listing's tasks were checked an array: {other}"),
    };
    // The files set aside, when the listing shows them (always a list).
    let quarantined = match answer.get_mut("quarantined") {
        Some(Value::Array(files)) => Some(std::mem::take(files)),
        Some(_) | None => None,
    };
    // Room for both lists within the cap, the markers of what was left out
    // set aside at their widest.
    let (all_tasks, all_files) = (tasks.len(), quarantined.as_ref().map_or(0, Vec::len));
    let markers = json!({
        "truncated": {"shown": all_tasks, "more": all_tasks},
        "quarantined_more": all_files,
    });
    answer["tasks"] = json!([]);
    if quarantined.is_some() {
        answer["quarantined"] = json!([]);
    }
    let mut room =
        MAX_LISTING_BYTES.saturating_sub(answer.to_string().len() + markers.to_string().len());
    // The tasks first, what the listing is for; the files set aside in
    // what is left, so they never crowd a task out.
    let shown = fitted(tasks, &mut room);
    let (count, more) = (shown.len(), all_tasks - shown.len());
    answer["tasks"] = Value::Array(shown);
    if more > 0 {
        answer["truncated"] = json!({"shown": count, "more": more});
    }
    if let Some(files) = quarantined {
        let shown = fitted(files, &mut room);
        let more = all_files - shown.len();
        answer["quarantined"] = Value::Array(shown);
        if more > 0 {
            answer["quarantined_more"] = json!(more);
        }
    }
    assert!(
        answer.to_string().len() <= MAX_LISTING_BYTES,
        "a listing's answer is within its cap"
    );
    answer
}

/// The leading `items` that fit in `room` bytes (each with its comma),
/// taking their bytes from it.
fn fitted(items: Vec<Value>, room: &mut usize) -> Vec<Value> {
    let mut shown = vec![];
    for item in items {
        let bytes = item.to_string().len() + 1;
        match bytes <= *room {
            true => {
                *room -= bytes;
                shown.push(item);
            }
            false => break,
        }
    }
    shown
}

/// The answer to the call `text` on `site`, noted on `call`: only the
/// tool's ops run, the schema's; creating a board is not one, and an op it
/// does not take is refused naming the ones it does.
fn answered(
    site: &BoardSite,
    text: &str,
    cancelled: Arc<AtomicBool>,
    call: &mut BoardCall,
) -> Value {
    let request = match read(text) {
        Ok(request) => request,
        Err(error) => return call.refuse_request(None, &error),
    };
    let fault = match request.get("op") {
        Some(Value::String(op)) if TOOL_OPS.contains(&op.as_str()) => {
            return site::answer(site, text, Some(cancelled), call);
        }
        Some(Value::String(op)) => {
            call.named(Some(op));
            FaultKind::InvalidValue
        }
        Some(_) => FaultKind::WrongType,
        None => FaultKind::Missing,
    };
    let ops: Vec<&str> = tool_ops().map(|(op, _)| *op).collect();
    let problem = format!("must be one of {}", ops.join(", "));
    call.refuse_request(None, &RequestError::of("op", problem, fault))
}

/// Sets the call's cancel flag when the call is dropped before its work
/// came back: the board then pushes nothing more.
struct CancelOnDrop(Option<Arc<AtomicBool>>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancelled) = self.0.take() {
            cancelled.store(true, Ordering::SeqCst);
        }
    }
}

impl Tool for BoardTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: BOARD_TOOL_NAME.into(),
            description: BOARD_TOOL_DESCRIPTION.into(),
            parameters_schema: Cow::Owned(parameters_schema()),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let (site, text) = (Arc::clone(&self.site), arguments.to_owned());
        // The call's clock starts as it arrives.
        let mut call = BoardCall::start(self.recordings.current(), FrontEnd::Tool, Client::Agent);
        Box::pin(async move {
            let cancelled = Arc::new(AtomicBool::new(false));
            let mut guard = CancelOnDrop(Some(Arc::clone(&cancelled)));
            // The board's git and host reads block: off the async workers,
            // in this call's panic scope. The record is written there too,
            // once the answer is summarised, even when the call was dropped.
            let work = off_the_workers(move || {
                let answer = summarised(answered(&site, &text, Arc::clone(&cancelled), &mut call));
                call.finish(&answer, cancelled.load(Ordering::SeqCst));
                answer
            });
            let answer = work.await.map_err(|error| {
                DomainError::Tool(format!("the board call did not finish: {error}"))
            })?;
            // Came back: nothing to cancel.
            guard.0 = None;
            // A refusal is the model's to act on (re-read, call again, stop).
            let ok = answer.get("ok") == Some(&Value::Bool(true));
            Ok(ToolResult {
                content: answer.to_string(),
                is_error: !ok,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
