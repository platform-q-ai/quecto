use super::swarm_output::{ok_json, tool_err};
use std::future::Future;
use std::pin::Pin;

use serde_json::json;

#[path = "swarm_guidance.rs"]
pub(super) mod swarm_guidance;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
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
            match v.get("op").and_then(|x| x.as_str()) {
                // The harness's own ops; every other valid op is a structured board
                // op, answered above.
                Some(op) if super::swarm_board_ops::HARNESS_OPS.contains(&op) => {
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
                refused => refuse_op(refused),
            }
        })
    }
}

/// The refusal of a call that names no valid op: one `tracing` record on
/// the board's telemetry target carrying the refusal's kind and whether the
/// op is a removed Python workbench op (#2282), never the member's text;
/// then the guidance the member reads.
fn refuse_op(op: Option<&str>) -> Result<ToolResult, DomainError> {
    let (refusal, message) = match op {
        Some(op) => ("unknown_op", swarm_guidance::unknown_op(op)),
        None => ("op_required", swarm_guidance::op_required()),
    };
    let removed_workbench_op = op.is_some_and(|op| swarm_guidance::REMOVED_OPS.contains(&op));
    tracing::info!(
        target: super::swarm_board_telemetry::TELEMETRY_TARGET,
        refusal,
        removed_workbench_op,
        "swarm op refused"
    );
    ok_json(json!({"status":"error","message":message}), true)
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
