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
use super::site::BoardSite;
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};

/// The tool's name.
pub const BOARD_TOOL_NAME: &str = "board";

/// The tool's description, as approved.
pub const BOARD_TOOL_DESCRIPTION: &str = "";

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
            let _ = (site, text);
            Err(DomainError::Tool("the board tool is not built yet".into()))
        })
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
