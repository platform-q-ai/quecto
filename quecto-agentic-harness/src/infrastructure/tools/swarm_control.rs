//! Swarm run commands, kept separate from Python execution and output handling.
use super::swarm_bridge::SwarmContext;
use crate::domain::error::DomainError;
use crate::domain::swarm::ProcessIdentity;
use serde_json::Value;

/// The swarm tool's entry: the composition's shared participation handle and
/// this process's own workflow engagement.
pub async fn control_for(
    context: SwarmContext,
    op: &str,
    input: Value,
    participation: super::swarm_bridge::Participation,
) -> Result<Value, DomainError> {
    control_with_workflow(
        context,
        op,
        input,
        participation,
        super::swarm_bridge::workflow_engaged(),
    )
    .await
}

/// Convenience for callers outside a tool composition: no shared participation
/// handle, this process's own workflow engagement.
pub async fn control(context: SwarmContext, op: &str, input: Value) -> Result<Value, DomainError> {
    control_with_workflow(
        context,
        op,
        input,
        super::swarm_bridge::Participation::none(),
        super::swarm_bridge::workflow_engaged(),
    )
    .await
}

/// `workflow_engaged` is this process's own workflow state (guards, bound
/// spec or selected template): creating a run makes the process a
/// coordinator, which cannot be running a workflow (#1715).
pub async fn control_with_workflow(
    context: SwarmContext,
    op: &str,
    input: Value,
    participation: super::swarm_bridge::Participation,
    workflow_engaged: bool,
) -> Result<Value, DomainError> {
    if op == "create" {
        crate::domain::swarm::validate_swarm_creation(workflow_engaged)?;
    }
    let settles = matches!(
        op,
        "create" | "reconcile" | "pause" | "resume" | "cancel_run" | "usage_budget"
    );
    let ctx = context.clone();
    let op = op.to_owned();
    let result = tokio::task::spawn_blocking(move || match op.as_str() {
        "create" => {
            std::fs::create_dir_all(ctx.checkout.join(".quecto"))
                .map_err(|e| DomainError::Tool(e.to_string()))?;
            let process = ProcessIdentity {
                pid: std::process::id(),
                started: super::swarm_bridge::process_start(std::process::id())
                    .ok_or_else(|| DomainError::Tool("process identity unavailable".into()))?,
            };
            let snapshot = ctx.create_run(
                &input,
                &process,
                super::swarm_bridge::process_socket().and_then(|s| s.to_str()),
            )?;
            // From here on this process and its local children are swarm agents.
            participation.set(true);
            super::swarm_lifecycle::supervise(ctx.clone(), snapshot);
            ctx.summary()
        }
        "usage" => ctx.usage_report(),
        "usage_budget" => {
            let strict = match input.get("strict_unknown") {
                None => true,
                Some(value) => value
                    .as_bool()
                    .ok_or_else(|| DomainError::Tool("strict_unknown must be boolean".into()))?,
            };
            if input.get("token_limit").is_some() {
                ctx.usage_budget(optional_cursor(&input, "token_limit")?, strict)
            } else {
                Err(DomainError::Tool(
                    "token_limit is required (null disables the budget)".into(),
                ))
            }
        }
        "summary" => ctx.summary_since(optional_cursor(&input, "since")?),
        "reconcile" => super::swarm_lifecycle::reconcile(&ctx),
        "pause" => ctx.pause(
            input["reason"]
                .as_str()
                .unwrap_or("operator requested pause"),
        ),
        "resume" => ctx.resume(),
        "events" => {
            let limit = optional_cursor(&input, "limit")?.unwrap_or(25);
            if (1..=100).contains(&limit) {
                ctx.events(optional_cursor(&input, "after")?.unwrap_or(0), limit as u32)
            } else {
                Err(DomainError::Tool("event limit must be 1..100".into()))
            }
        }
        "cancel_run" => {
            ctx.cancel_run()?;
            ctx.summary()
        }
        _ => Err(DomainError::Tool(format!("unknown swarm operation {op}"))),
    })
    .await
    .map_err(|e| DomainError::Tool(e.to_string()))??;
    if settles
        && matches!(
            result["status"].as_str(),
            Some("paused" | "cancelled" | "failed" | "succeeded" | "blocked" | "budget-exhausted")
        )
    {
        super::swarm_lifecycle::settle(context).await
    } else {
        Ok(result)
    }
}

pub async fn execution_state(context: SwarmContext) -> Result<Value, DomainError> {
    tokio::task::spawn_blocking(move || context.summary())
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))?
}

/// Shared foreground/background post-execution lifecycle. Read-only programs
/// produce no wake hints, avoiding notification-driven idle loops.
pub async fn after_execution(
    context: SwarmContext,
    before: &Value,
) -> Result<Vec<String>, DomainError> {
    let after = execution_state(context.clone()).await?;
    if after["status"] != "running" {
        super::swarm_lifecycle::settle(context).await?;
        return Ok(Vec::new());
    }
    if after["event_cursor"] != before["event_cursor"] {
        return Ok(super::swarm_lifecycle::notify(&context).await);
    }
    Ok(Vec::new())
}

fn optional_cursor(input: &Value, field: &str) -> Result<Option<u64>, DomainError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| DomainError::Tool(format!("{field} must be a nonnegative integer"))),
    }
}

#[cfg(test)]
#[path = "swarm_control_tests.rs"]
mod tests;

impl super::swarm::SwarmTool {
    /// The composition's shared participation handle (#1715).
    pub fn with_participation(mut self, participation: super::swarm_bridge::Participation) -> Self {
        self.participation = participation;
        self
    }
}
