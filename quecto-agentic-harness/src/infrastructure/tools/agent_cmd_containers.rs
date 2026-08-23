//! Thin tool adapter for the environment control use case (#1369 slice 2).
//!
//! Decode `get_containers` / `kill_container` arguments, delegate to
//! [`EnvironmentControlUseCase`], and encode the result. No environment
//! transaction logic lives here.

use std::sync::Arc;

use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentTarget};
use crate::domain::tool::ToolResult;
use crate::environment_control_app::EnvironmentControlUseCase;

pub(super) fn is_container_command(args: &serde_json::Value) -> bool {
    matches!(
        args.get("command").and_then(|v| v.as_str()),
        Some("get_containers") | Some("kill_container")
    )
}

pub(super) async fn execute_container_command(
    environment_control: Option<&Arc<EnvironmentControlUseCase>>,
    args: &serde_json::Value,
) -> ToolResult {
    let Some(uc) = environment_control else {
        return error("environment control is not available in this session".to_string());
    };
    if args.get("agent_id").and_then(|v| v.as_str()) != Some("*") {
        return error("container commands require agent_id '*'".to_string());
    }
    match args.get("command").and_then(|v| v.as_str()) {
        Some("get_containers") => encode_listing(uc.get_containers()),
        Some("kill_container") => match decode_target(args) {
            Ok(target) => match uc.kill_container(&target).await {
                Ok(record) => ToolResult {
                    content: kill_container_result_json(&record).to_string(),
                    is_error: false,
                    image_blocks: vec![],
                    delivery_metadata: None,
                },
                Err(e) => error(e),
            },
            Err(e) => error(e),
        },
        _ => error("unsupported container command".to_string()),
    }
}

fn decode_target(args: &serde_json::Value) -> Result<EnvironmentTarget, String> {
    let env_ref = optional_str(args, "ref")?;
    let name = optional_str(args, "name")?;
    match (env_ref, name) {
        (Some(env_ref), None) => Ok(EnvironmentTarget::Ref(env_ref)),
        (None, Some(name)) => Ok(EnvironmentTarget::Name(name)),
        _ => Err("kill_container requires exactly one of 'ref' or 'name'".to_string()),
    }
}

fn optional_str(args: &serde_json::Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn capped_agents_json(agent_ids: &[String]) -> serde_json::Value {
    const MAX_REPORTED_AGENTS: usize = 20;
    let shown: Vec<_> = agent_ids
        .iter()
        .take(MAX_REPORTED_AGENTS)
        .cloned()
        .collect();
    let mut result = serde_json::json!({"agents": shown});
    if agent_ids.len() > MAX_REPORTED_AGENTS {
        result["omitted_agents"] = serde_json::json!(agent_ids.len() - MAX_REPORTED_AGENTS);
    }
    result
}

fn kill_container_result_json(record: &EnvironmentRecord) -> serde_json::Value {
    let mut result = capped_agents_json(&record.members);
    result["killed"] = serde_json::json!(record.environment_ref);
    result
}

fn encode_listing(records: Vec<EnvironmentRecord>) -> ToolResult {
    let containers: Vec<serde_json::Value> = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "ref": record.environment_ref,
                "name": record.name,
                "status": record.status_label(),
                "workspace": record.workspace_path.display().to_string(),
                "repository": record.repository,
                "environment_uuid": record.environment_uuid,
                "members": record.members,
                "metadata": record.metadata,
                "last_error": record.last_error,
            })
        })
        .collect();
    ToolResult {
        content: serde_json::json!({"containers": containers}).to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

fn error(message: String) -> ToolResult {
    ToolResult {
        content: format!("agent_cmd error: {message}"),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

#[cfg(test)]
#[path = "agent_cmd_containers_tests.rs"]
mod tests;
