//! The agent `board` tool (#2477 S2b): one tool taking an `op`, served in
//! the harness's own process through the same request/answer path as
//! `quecto board` ([`serve`]). The board is the one on the agent's working
//! repository's board remote, found as the CLI finds it. Building the
//! board and running the op block (git, the network, the clock's outside
//! reference), so each call runs on Tokio's blocking pool.
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde_json::{Map, Value, json};

use super::answer::{ErrorCode, reason_refused, request_refused};
use super::request::{Kind, RequestError, fields_of};
use super::{BoardLocation, DEFAULT_REMOTE, ProjectBoardHandlesBuilder, json, serve};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::domain::tool::{ToolDefinition, ToolResult};

/// The tool's name.
pub const BOARD_TOOL_NAME: &str = "board";

/// The tool's description, as the owner approved it: change it only with
/// their approval (its fixture test pins it byte for byte).
pub const BOARD_TOOL_DESCRIPTION: &str = include_str!("tool_description.txt");

/// Every op the tool takes: the shared table's, less `bootstrap`, which
/// sets a board up and only the CLI runs.
pub const TOOL_OPS: &[&str] = &[
    "list",
    "show",
    "list_ready",
    "list_takeable",
    "create",
    "update_draft",
    "claim",
    "renew",
    "take_over",
    "release",
    "move",
    "block",
    "archive",
    "link_pr",
    "unlink_pr",
    "complete",
    "clear_set_aside",
];

/// The `board` tool over the board of the repository at `repo`.
pub struct BoardTool {
    build: ProjectBoardHandlesBuilder,
    repo: PathBuf,
}

impl BoardTool {
    /// The tool for the agent working in `repo`, its board built by
    /// `build` on the default board remote.
    pub fn new(build: ProjectBoardHandlesBuilder, repo: PathBuf) -> Self {
        Self { build, repo }
    }
}

impl Tool for BoardTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: BOARD_TOOL_NAME.into(),
            description: BOARD_TOOL_DESCRIPTION.into(),
            parameters_schema: schema().to_string().into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let build = self.build;
        let location = BoardLocation {
            repo: self.repo.clone(),
            remote: DEFAULT_REMOTE.into(),
        };
        let arguments = arguments.to_owned();
        Box::pin(async move {
            let blocking = tokio::task::spawn_blocking(move || answer(build, location, &arguments));
            let answer = match blocking.await {
                Ok(answer) => answer,
                Err(error) => reason_refused(
                    None,
                    ErrorCode::StoreUnavailable,
                    &format!("the board call failed before it answered: {error}"),
                ),
            };
            Ok(result(&answer))
        })
    }
}

/// The answer to one call's `arguments`: JSON naming one of [`TOOL_OPS`],
/// served as the CLI serves it.
fn answer(build: ProjectBoardHandlesBuilder, location: BoardLocation, arguments: &str) -> Value {
    let refused = |field: &str, problem: String| {
        request_refused(
            None,
            &RequestError {
                field: field.into(),
                problem,
            },
        )
    };
    let value = match json::strict(arguments) {
        Ok(value) => value,
        Err(error) => return refused("arguments", format!("must be JSON: {error}")),
    };
    match value.get("op").and_then(Value::as_str) {
        Some(op) if TOOL_OPS.contains(&op) => serve(Some(build), location, &value),
        Some(_) | None => refused("op", format!("must be one of {}", TOOL_OPS.join(", "))),
    }
}

/// `answer` as the call's result: an error unless it is `"ok": true`.
fn result(answer: &Value) -> ToolResult {
    let ok = matches!(answer.get("ok"), Some(Value::Bool(true)));
    ToolResult {
        content: answer.to_string(),
        is_error: !ok,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

/// The parameters: `op`, and every field of every op the tool takes, each
/// typed as the shared table reads it.
fn schema() -> Value {
    let mut kinds: Vec<(&'static str, Kind)> = Vec::new();
    for op in TOOL_OPS {
        let fields = fields_of(op).unwrap_or_else(|| panic!("tool op {op} is not in OPS"));
        for field in fields {
            match kinds.iter_mut().find(|(name, _)| *name == field.name) {
                Some((name, kind)) => *kind = wider(name, *kind, field.kind),
                None => kinds.push((field.name, field.kind)),
            }
        }
    }
    let mut properties = Map::new();
    properties.insert("op".into(), json!({"type": "string", "enum": TOOL_OPS}));
    for (name, kind) in kinds {
        properties.insert(name.into(), kind_schema(kind));
    }
    json!({"type": "object", "properties": properties, "required": ["op"]})
}

/// The one kind a field named in more than one op takes in the schema:
/// the wider of the two. Only these pairs share a name.
fn wider(name: &str, a: Kind, b: Kind) -> Kind {
    match (a, b) {
        (a, b) if a == b => a,
        (Kind::Slug, Kind::OptionalSlug) | (Kind::OptionalSlug, Kind::Slug) => Kind::OptionalSlug,
        (Kind::Status, Kind::NewStatus) | (Kind::NewStatus, Kind::Status) => Kind::Status,
        (a, b) => panic!("field {name} is {a:?} in one op and {b:?} in another"),
    }
}

fn kind_schema(kind: Kind) -> Value {
    let statuses: Vec<&str> = TaskStatus::ALL
        .iter()
        .map(|status| status.as_str())
        .collect();
    let kinds: Vec<&str> = TaskKind::ALL.iter().map(|kind| kind.as_str()).collect();
    match kind {
        Kind::Slug | Kind::Text | Kind::Timestamp | Kind::Path | Kind::ObjectId => {
            json!({"type": "string"})
        }
        Kind::OptionalSlug => json!({"type": ["string", "null"]}),
        Kind::Slugs => json!({"type": "array", "items": {"type": "string"}}),
        Kind::Status => json!({"type": "string", "enum": statuses}),
        Kind::NewStatus => json!({"type": "string", "enum": ["draft", "ready"]}),
        Kind::TaskKind => json!({"type": "string", "enum": kinds}),
        Kind::Pr => json!({"type": "integer", "minimum": 1}),
        Kind::Plan => json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "criteria": {"type": "array", "items": {"type": "string"}},
                },
                "required": ["title"],
            },
        }),
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
