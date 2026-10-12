//! The agent `board` tool (#2477 S2b): a parent agent's front end on its
//! repository's project board. A call's arguments are the request, the
//! same JSON object the `quecto board` CLI reads, answered through the one
//! shared path ([`site::answer`]) on the repository the agent works in.
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::request::{Kind, OPS};
use super::site::{self, BoardSite};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};

/// The tool's name.
pub const BOARD_TOOL_NAME: &str = "board";

/// The tool's description: the approved text, with only the changes the
/// ops on master need (#2477 S2b; listed in its pull request).
pub const BOARD_TOOL_DESCRIPTION: &str = r#"Project board for this repo: the shared list of tasks people and agents plan, claim and finish. Tasks live on the repo's quecto/board branch and are written only through this tool, never by editing the branch. Each op is one call, {"op":"<name>", ...named fields}, answering JSON: {"ok":true,...} or {"ok":false,"error":{"code","field","message"}}. Ids are slugs (a-z, 0-9, -). Statuses: draft, ready, claimed, in_progress, review, blocked, done, archived.
Read: list(status|parent|dependents_of, at most one filter), show(task), list_ready (ready tasks whose dependencies are done or archived), list_takeable (held tasks whose claim expired). Listings are advice; every change is checked against the board when it is written.
Plan: create(task,title,kind,description,status draft|ready,parent,depends_on); update_draft(task, any of title,description,parent,depends_on, expected_updated) edits a draft only; pass the updated you read as expected_updated so a newer edit is never overwritten. Anyone may create, edit a draft, or move it to ready.
Work: move(task,to claimed) takes a ready task for 2 hours; renew(task) before it lapses while you work; take_over(task) only once a claim has expired; move(task,to ready) hands it back to ready. Only the claim holder moves a claimed task: move(task,to in_progress|review|blocked|ready). link_pr(task,pr) and unlink_pr(task,pr) are the holder's. A task with linked pull requests is done only when all of them have merged: link_pr and moving to review check this automatically, and complete(task) asks again (anyone may call it). Never move a task to done by hand while a pull request is linked.
Repair: clear_set_aside(path, object, repeated) removes a file the board could not read (listed under quarantined, with its object); repeated names the copy to keep when a task file appears twice. Use it only when told to.
On "stale" re-read with show and redo your change; on "contended" call again; on "clock_ahead" or "task_ahead" stop and report: this machine's clock is wrong. A "clock" note on an answer means writes ran without an outside time check."#;

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

/// The JSON type a field of `kind` takes.
fn field_schema(kind: Kind) -> Value {
    match kind {
        Kind::Slug
        | Kind::Text
        | Kind::Timestamp
        | Kind::Status
        | Kind::NewStatus
        | Kind::TaskKind
        | Kind::Path
        | Kind::ObjectId => json!({"type": "string"}),
        Kind::OptionalSlug => json!({"type": ["string", "null"]}),
        Kind::Slugs => json!({"type": "array", "items": {"type": "string"}}),
        Kind::Pr => json!({"type": "integer", "minimum": 1}),
        Kind::Repeated => json!({"type": "object"}),
    }
}

/// The parameters every op may name, typed, from the one table of ops
/// ([`OPS`]): a name two ops type differently takes both types.
fn parameters_schema() -> String {
    let mut properties = Map::new();
    let ops: Vec<&str> = OPS.iter().map(|(op, _)| *op).collect();
    properties.insert("op".into(), json!({"type": "string", "enum": ops}));
    for field in OPS.iter().flat_map(|(_, fields)| fields.iter()) {
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
            // The board's git and host reads block: off the async workers.
            let answer = tokio::task::spawn_blocking(move || site::answer(&site, &text))
                .await
                .map_err(|error| {
                    DomainError::Tool(format!("the board call did not finish: {error}"))
                })?;
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
