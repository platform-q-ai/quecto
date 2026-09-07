//! Swarm run commands, kept separate from Python execution and output handling.
use super::swarm_bridge::SwarmContext;
use crate::domain::error::DomainError;
use serde_json::{Value, json};

pub async fn control(context: SwarmContext, op: &str, input: Value) -> Result<Value, DomainError> {
    let ctx = context.clone();
    let op = op.to_owned();
    let result = tokio::task::spawn_blocking(move || match op.as_str() {
        "create" => {
            std::fs::create_dir_all(ctx.checkout.join(".quecto"))
                .map_err(|e| DomainError::Tool(e.to_string()))?;
            let result = ctx.call(
                "create",
                json!([
                    input["goal"],
                    input["constraints"],
                    input["criteria"],
                    input["member_limit"],
                    input["deadline"]
                ]),
            )?;
            let reservation = result["members"][0]["reservation"].clone();
            ctx.call(
                "_activate",
                json!([
                    ctx.member,
                    reservation,
                    std::process::id(),
                    super::swarm_bridge::process_start(std::process::id()),
                    super::swarm_bridge::process_socket().map(|s| s.to_string_lossy().to_string())
                ]),
            )?;
            if let Some(deadline) = result["deadline"].as_f64() {
                super::swarm_lifecycle::supervise(ctx.clone(), deadline);
            }
            ctx.summary()
        }
        "summary" | "reconcile" => super::swarm_lifecycle::reconcile(&ctx),
        "cancel_run" => {
            ctx.call("stop", json!(["cancelled", "parent/user cancellation"]))?;
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
