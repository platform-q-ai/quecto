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
//! read already under way may still finish.
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value, json};

use super::answer::request_refused;
use super::request::{Kind, OPS, RequestError};
use super::site::{self, BoardSite};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
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
Plan: create(task,title,kind,description,status draft|ready,parent,depends_on); update_draft(task, any of title,description,parent,depends_on, expected_updated) edits a draft only; pass the updated you read as expected_updated so a newer edit is never overwritten. Anyone may create, edit a draft, move it to ready, or archive a draft, ready or done task with move(task,to archived).
Work: move(task,to claimed) takes a ready task for 2 hours; renew(task) before it lapses while you work; take_over(task) only once a claim has expired; move(task,to ready) hands it back to ready. Only the claim holder moves a held task: claimed to in_progress|blocked|ready, in_progress to review|blocked|ready, review to in_progress (or done when no pull request is linked), blocked to in_progress. Once its claim has expired, anyone may move a review or blocked task to ready. link_pr(task,pr) and unlink_pr(task,pr) are the holder's. A task with linked pull requests is done only when all of them have merged: link_pr and moving to review check this automatically, and complete(task) asks again (anyone may call it). Never move a task to done by hand while a pull request is linked.
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
}

impl BoardTool {
    pub fn new(site: BoardSite) -> Self {
        Self {
            site: Arc::new(site),
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

/// `answer`, a listing's tasks as summaries and as many as fit in
/// [`MAX_LISTING_BYTES`], ending with what was left out; any other answer
/// as it is.
fn summarised(mut answer: Value) -> Value {
    let listing = answer["ok"] == json!(true)
        && answer["op"]
            .as_str()
            .is_some_and(|op| LISTINGS.contains(&op));
    let tasks = match (listing, answer["tasks"].take()) {
        (true, Value::Array(tasks)) => tasks,
        (_, tasks) => {
            answer["tasks"] = tasks;
            return answer;
        }
    };
    // Room for the tasks within the cap, the marker of what was left out
    // set aside.
    let marker = json!({"truncated": {"shown": tasks.len(), "more": tasks.len()}});
    answer["tasks"] = json!([]);
    let mut room =
        MAX_LISTING_BYTES.saturating_sub(answer.to_string().len() + marker.to_string().len());
    let mut shown = vec![];
    for task in &tasks {
        let summary = summary(task);
        let bytes = summary.to_string().len() + 1;
        match bytes <= room {
            true => {
                room -= bytes;
                shown.push(summary);
            }
            false => break,
        }
    }
    let (count, more) = (shown.len(), tasks.len() - shown.len());
    answer["tasks"] = Value::Array(shown);
    if more > 0 {
        answer["truncated"] = json!({"shown": count, "more": more});
    }
    debug_assert!(answer.to_string().len() <= MAX_LISTING_BYTES || count == 0);
    answer
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
        Box::pin(async move {
            let named = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| value["op"].as_str().map(str::to_owned));
            let answer = match named.as_deref() {
                // The tool's ops are the schema's; creating a board is not one.
                Some("bootstrap") => {
                    let ops: Vec<&str> = tool_ops().map(|(op, _)| *op).collect();
                    let error = RequestError {
                        field: "op".into(),
                        problem: format!("must be one of {}", ops.join(", ")),
                    };
                    request_refused(None, &error)
                }
                _ => {
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let mut guard = CancelOnDrop(Some(Arc::clone(&cancelled)));
                    // The board's git and host reads block: off the async
                    // workers, in this call's panic scope.
                    let work = off_the_workers(move || site::answer(&site, &text, Some(cancelled)));
                    let answer = work.await.map_err(|error| {
                        DomainError::Tool(format!("the board call did not finish: {error}"))
                    })?;
                    // Came back: nothing to cancel.
                    guard.0 = None;
                    summarised(answer)
                }
            };
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
