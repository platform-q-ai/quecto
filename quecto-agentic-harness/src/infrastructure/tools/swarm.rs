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

/// The snapshot key of a full `summary` answer (#2342): a newer one
/// supersedes it in the member's conversation.
pub const SUMMARY_SNAPSHOT: &str = "swarm.summary";

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
    /// A full `summary` answer is a whole snapshot of the board (#2342):
    /// the op is `summary` and the answer carries the board's `members`,
    /// `tasks` and `event_cursor`. The fast path's `unchanged` answer
    /// carries no board state, and no other op's answer is a snapshot (an
    /// `inbox` answer can hold the only copy of a message).
    fn snapshot_key(&self, arguments: &str, content: &str) -> Option<&'static str> {
        let arguments: serde_json::Value = serde_json::from_str(arguments).ok()?;
        if arguments.get("op").and_then(serde_json::Value::as_str) != Some("summary") {
            return None;
        }
        let answer: serde_json::Value = serde_json::from_str(content).ok()?;
        let whole = answer
            .get("members")
            .is_some_and(serde_json::Value::is_array)
            && answer.get("tasks").is_some_and(serde_json::Value::is_array)
            && answer
                .get("event_cursor")
                .is_some_and(serde_json::Value::is_number);
        whole.then_some(SUMMARY_SNAPSHOT)
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "swarm".into(),
            description: include_str!("swarm_assets/tool_description.txt").into(),
            parameters_schema: include_str!("swarm_assets/tool_schema.json").into(),
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
                Err(e) => return refuse_op(&context, Refused::InvalidJson(e), started).await,
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
                // A call with `code` and no op was the implicit op=run.
                None => {
                    let implicit_run = v.get("code").is_some();
                    refuse_op(&context, Refused::Missing { implicit_run }, started).await
                }
            }
        })
    }
}

/// Why a call names no valid op.
enum Refused<'a> {
    /// A string that is no op the tool has (an internal board method's
    /// name included).
    Unknown(&'a str),
    /// An op that is present but not a string (#2282 review N1).
    NotAString,
    /// No op at all (or arguments that are not an object);
    /// `implicit_run` when the call carries `code`, which once ran Python
    /// as op=run.
    Missing { implicit_run: bool },
    /// Arguments that are not JSON.
    InvalidJson(serde_json::Error),
}

/// The refusal of a call that names no valid op: one `tracing` record on
/// the board's telemetry target carrying the refusal's kind and whether the
/// call is a removed Python workbench op (#2282), never the member's text;
/// the call's `swarm_op` record while the event log is on, always as op
/// `unknown` (never under a name the member chose, an internal board
/// method's included), refused as `calling` (`invalid` for arguments that
/// are not JSON), by the caller's redacted ref; then the guidance the
/// member reads.
async fn refuse_op(
    context: &super::swarm_bridge::SwarmContext,
    refused: Refused<'_>,
    started: std::time::Instant,
) -> Result<ToolResult, DomainError> {
    let (refusal, removed_workbench_op, kind) = match &refused {
        Refused::Unknown(op) => (
            "unknown_op",
            swarm_guidance::REMOVED_OPS.contains(op),
            RefusalKind::Calling,
        ),
        Refused::NotAString => ("op_not_a_string", false, RefusalKind::Calling),
        Refused::Missing { implicit_run } => ("op_required", *implicit_run, RefusalKind::Calling),
        Refused::InvalidJson(_) => ("invalid_json", false, RefusalKind::Invalid),
    };
    tracing::info!(
        target: super::swarm_board_telemetry::TELEMETRY_TARGET,
        refusal,
        removed_workbench_op,
        "swarm op refused"
    );
    debug_assert!(
        super::swarm_board_dispatch::signature(UNKNOWN_OP).is_none(),
        "a refused call is recorded under no board method's name"
    );
    // Text that is no JSON is refused as a whole (#2341).
    let arguments = match &refused {
        Refused::InvalidJson(_) => super::swarm_board_dispatch::unreadable_arguments(None),
        Refused::Unknown(_) | Refused::NotAString | Refused::Missing { .. } => {
            super::swarm_board_dispatch::BindingFaults::NONE
        }
    };
    let ctx = context.clone();
    let elapsed = started.elapsed();
    super::call_work::spawn_blocking_in_call(move || {
        ctx.refused(UNKNOWN_OP, kind, arguments, elapsed)
    })
    .await
    .map_err(|error| DomainError::Tool(error.to_string()))?;
    match refused {
        Refused::Unknown(op) => ok_json(
            json!({"status":"error","message":swarm_guidance::unknown_op(op)}),
            true,
        ),
        Refused::NotAString => ok_json(
            json!({"status":"error","message":swarm_guidance::op_not_a_string()}),
            true,
        ),
        Refused::Missing { .. } => ok_json(
            json!({"status":"error","message":swarm_guidance::op_required()}),
            true,
        ),
        Refused::InvalidJson(error) => tool_err(format!("invalid JSON arguments: {error}")),
    }
}

/// The name a refused call's `swarm_op` is recorded under: no board method
/// has it, so the board records the call as op `unknown`.
const UNKNOWN_OP: &str = "unknown";

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
