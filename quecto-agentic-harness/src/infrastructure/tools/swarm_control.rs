//! Swarm run commands, kept separate from Python execution and output handling.
use super::swarm_bridge::SwarmContext;
use crate::domain::error::DomainError;
use crate::domain::swarm::ProcessIdentity;
use serde_json::Value;

pub async fn control(context: SwarmContext, op: &str, input: Value) -> Result<Value, DomainError> {
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
            super::swarm_lifecycle::supervise(ctx.clone(), snapshot);
            ctx.summary()
        }
        "summary" | "reconcile" => super::swarm_lifecycle::reconcile(&ctx),
        "cancel_run" => {
            ctx.cancel_run()?;
            ctx.summary()
        }
        _ => Err(DomainError::Tool(format!("unknown swarm operation {op}"))),
    })
    .await
    .map_err(|e| DomainError::Tool(e.to_string()))??;
    if result["status"] != "running" {
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
    if after["events"].as_array().and_then(|e| e.last())
        != before["events"].as_array().and_then(|e| e.last())
    {
        return Ok(super::swarm_lifecycle::notify(&context).await);
    }
    Ok(Vec::new())
}
