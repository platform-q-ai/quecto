use super::swarm_output::{ok_json, tool_err};
use std::future::Future;
use std::pin::Pin;

use serde_json::json;

#[path = "swarm_guidance.rs"]
pub(super) mod swarm_guidance;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::swarm::RefusalKind;
use crate::domain::tool::{ToolDefinition, ToolResult};

/// The container-only coordination tool: the harness's own run ops
/// (`create`, `summary`, `cancel_run`, …) and the structured board ops
/// (#2279). It spawns and signals nothing: the board runs in-process
/// (ADR-0030) and members use `bash` for computation (#2282).
pub struct SwarmTool {
    context: Option<super::swarm_bridge::SwarmContext>,
    /// Shared with the spawn and workflow tools (#1715); `create` flips it.
    participation: super::swarm_bridge::Participation,
    /// The composition's workflow engine (#1715); `create` refuses while engaged.
    workflow_engine: super::swarm_bridge::WorkflowEngineSlot,
}

impl SwarmTool {
    pub fn new() -> Self {
        Self {
            context: None,
            participation: super::swarm_bridge::Participation::none(),
            workflow_engine: Default::default(),
        }
    }

    pub fn with_participation(mut self, participation: super::swarm_bridge::Participation) -> Self {
        self.participation = participation;
        self
    }

    pub fn with_workflow_engine(mut self, slot: super::swarm_bridge::WorkflowEngineSlot) -> Self {
        self.workflow_engine = slot;
        self
    }

    pub fn with_context(mut self, context: Option<super::swarm_bridge::SwarmContext>) -> Self {
        self.context = context;
        self
    }
}

impl Default for SwarmTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for SwarmTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "swarm".into(),
            description: include_str!("swarm_helpers/tool_description.txt").into(),
            parameters_schema: include_str!("swarm_helpers/tool_schema.json").into(),
        }
    }
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let workflow_engine = self.workflow_engine.clone();
        let participation = self.participation.clone();
        let context = self.context.clone();
        // A structured board op reads the member's text as `json.loads`
        // does (#2279); every other op keeps serde's reading.
        let board_op = self
            .context
            .as_ref()
            .and_then(|context| super::swarm_board_ops::requested(arguments, context.wire()));
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        Box::pin(async move {
            let started = std::time::Instant::now();
            let Some(context) = context else {
                return tool_err("swarm is container-only: use spawn with a registered isolated container, then create a bounded run inside it".into());
            };
            if let Some(request) = board_op {
                return super::swarm_board_ops::board_op(context, request).await;
            }
            let v = match parsed {
                Ok(v) => v,
                Err(e) => return tool_err(format!("invalid JSON arguments: {e}")),
            };
            match v.get("op") {
                // The harness's own ops; every other valid op is a structured board
                // op, answered above.
                Some(serde_json::Value::String(op))
                    if super::swarm_board_ops::HARNESS_OPS.contains(&op.as_str()) =>
                {
                    match super::swarm_control::control_with_workflow(
                        context,
                        op,
                        v.clone(),
                        participation,
                        workflow_engine,
                    )
                    .await
                    {
                        Ok(value) => ok_json(value, false),
                        Err(error) => tool_err(error.to_string()),
                    }
                }
                Some(serde_json::Value::String(op)) => {
                    refuse_op(&context, Refused::Unknown(op), started).await
                }
                Some(_) => refuse_op(&context, Refused::NotAString, started).await,
                None => refuse_op(&context, Refused::Missing, started).await,
            }
        })
    }
}

/// Why a call names no valid op.
enum Refused<'a> {
    /// A string that is no op the tool has.
    Unknown(&'a str),
    /// An op that is present but not a string (#2282 review N1).
    NotAString,
    /// No op at all (or arguments that are not an object).
    Missing,
}

/// The refusal of a call that names no valid op: one `tracing` record on
/// the board's telemetry target carrying the refusal's kind and whether the
/// op is a removed Python workbench op (#2282), never the member's text;
/// the op's `swarm_op` record while the event log is on (a name that is no
/// board method is recorded as `unknown`, refused as `calling`, by the
/// caller's redacted ref); then the guidance the member reads.
async fn refuse_op(
    context: &super::swarm_bridge::SwarmContext,
    refused: Refused<'_>,
    started: std::time::Instant,
) -> Result<ToolResult, DomainError> {
    let (refusal, message, op) = match refused {
        Refused::Unknown(op) => ("unknown_op", swarm_guidance::unknown_op(op), op),
        Refused::NotAString => ("op_not_a_string", swarm_guidance::op_not_a_string(), ""),
        Refused::Missing => ("op_required", swarm_guidance::op_required(), ""),
    };
    let removed_workbench_op = swarm_guidance::REMOVED_OPS.contains(&op);
    tracing::info!(
        target: super::swarm_board_telemetry::TELEMETRY_TARGET,
        refusal,
        removed_workbench_op,
        "swarm op refused"
    );
    let ctx = context.clone();
    let method = op.to_owned();
    let elapsed = started.elapsed();
    super::call_work::spawn_blocking_in_call(move || {
        ctx.refused(&method, RefusalKind::Calling, elapsed)
    })
    .await
    .map_err(|error| DomainError::Tool(error.to_string()))?;
    ok_json(json!({"status":"error","message":message}), true)
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
