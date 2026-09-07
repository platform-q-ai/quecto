//! Swarm admission and settlement adapt existing harness process/UDS lifecycle.
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::domain::error::DomainError;
use serde_json::{Value, json};

pub fn join_current_process(
    context: &SwarmContext,
    socket: Option<&std::path::Path>,
) -> Result<(), DomainError> {
    if !context.database().exists() && std::env::var("QUECTO_SWARM_BOOTSTRAP").as_deref() != Ok("1")
    {
        return Err(DomainError::Tool("swarm coordination store missing; only the container creator may initialize it; do not reset admission".into()));
    }
    std::fs::create_dir_all(context.checkout.join(".quecto"))
        .map_err(|e| DomainError::Tool(format!("swarm storage: {e}")))?;
    let pid = std::process::id();
    let start = process_start(pid)
        .ok_or_else(|| DomainError::Tool("swarm requires Linux procfs process identity".into()))?;
    let summary = context.call(
        "_bootstrap",
        json!([
            pid,
            start,
            socket.map(|s| s.to_string_lossy().to_string()),
            std::env::var("QUECTO_SWARM_RESERVATION").ok()
        ]),
    )?;
    if summary["status"] == "running" {
        if let Some(deadline) = summary["deadline"].as_f64() {
            supervise(context.clone(), deadline);
        }
    }
    Ok(())
}

pub fn reconcile(context: &SwarmContext) -> Result<Value, DomainError> {
    let summary = context.summary()?;
    for member in summary["members"].as_array().into_iter().flatten() {
        if let (Some(pid), Some(start), Some(id)) = (
            member["pid"].as_u64(),
            member["started"].as_str(),
            member["id"].as_str(),
        ) {
            if u32::try_from(pid).is_ok_and(|pid| process_confirmed_dead(pid, start))
                && member["status"] != "dead"
            {
                context.call("_confirmed_dead", json!([id]))?;
            }
        }
    }
    context.summary()
}

/// Runs after a helper execution. Durable messages remain accepted even when
/// the notification hint fails; the caller receives the failure as a warning.
pub async fn notify(context: &SwarmContext) -> Vec<String> {
    let ctx = context.clone();
    let summary = tokio::task::spawn_blocking(move || ctx.summary()).await;
    let Ok(Ok(summary)) = summary else {
        return vec!["swarm notification summary unavailable; inspect durable inbox".into()];
    };
    let mut warnings = Vec::new();
    for member in summary["members"].as_array().into_iter().flatten() {
        let Some(socket) = member["socket"].as_str() else {
            continue;
        };
        if member["id"] == context.member || member["status"] != "live" {
            continue;
        }
        // A hint never claims delivery/consumption/completion. Existing UDS
        // follow-up semantics wake idle agents without a second message daemon.
        let command = json!({"type":"prompt", "message":"Swarm board changed. Inspect your durable inbox and ready work with swarm; acknowledge messages after reading. If no work is ready, yield this turn.", "streamingBehavior":"followUp", "ack":"accept"});
        if super::subagent_registry::send_subagent_uds_command_with_timeout(
            std::path::Path::new(socket),
            &command.to_string(),
            std::time::Duration::from_millis(500),
        )
        .await
        .is_err()
        {
            warnings.push(format!(
                "wake hint failed for {}; durable board is authoritative",
                member["id"]
            ));
        }
    }
    warnings
}

pub async fn settle(context: SwarmContext) -> Result<Value, DomainError> {
    let ctx = context.clone();
    let summary = tokio::task::spawn_blocking(move || ctx.summary())
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))??;
    if summary["status"] == "running" || summary["status"] == "setup" {
        return Ok(summary);
    }
    let mut members = summary["members"].as_array().cloned().unwrap_or_default();
    members.sort_by_key(|member| member["id"] == context.member);
    for member in &members {
        if member["id"] == summary["coordinator"]
            && summary["status"] != "budget-exhausted"
            && summary["status"] != "failed"
        {
            continue;
        }
        if let Some(socket) = member["socket"].as_str() {
            let _ = super::subagent_registry::send_subagent_uds_command_with_timeout(
                std::path::Path::new(socket),
                r#"{"type":"abort"}"#,
                std::time::Duration::from_millis(500),
            )
            .await;
        }
        if member["id"] == summary["coordinator"] && member["socket"].as_str().is_some() {
            continue;
        }
        if let (Some(pid), Some(start)) = (
            member["pid"].as_u64().and_then(|p| u32::try_from(p).ok()),
            member["started"].as_str(),
        ) {
            if process_start(pid).as_deref() == Some(start) {
                tokio::task::spawn_blocking(move || super::swarm::terminate_member(pid))
                    .await
                    .map_err(|e| DomainError::Tool(e.to_string()))?;
            }
        }
    }
    tokio::task::spawn_blocking(move || reconcile(&context))
        .await
        .map_err(|e| DomainError::Tool(e.to_string()))?
}

/// Deadline supervision belongs to the harness, including while all members
/// are idle. No Python process, transaction, or provider request is kept alive.
pub fn supervise(context: SwarmContext, deadline: f64) {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let remaining = deadline
            - std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
        if remaining > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f64(remaining));
        }
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => {
                if let Err(error) = runtime.block_on(settle(context)) {
                    tracing::error!(%error, "swarm deadline settlement failed");
                }
            }
            Err(error) => tracing::error!(%error, "swarm deadline runtime failed"),
        }
    });
}
