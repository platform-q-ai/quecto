//! Thin tool adapter for the environment control use case (#1369 slice 2).
//!
//! Decode `get_containers` / `kill_container` / `get_container_configs`
//! arguments, delegate to [`ListEnvironmentsQuery`] / [`KillEnvironment`] /
//! [`ListContainerConfigs`], and encode the result. No environment
//! transaction logic lives here.

use std::sync::Arc;

use crate::application::environments::dto::ContainerConfigInventory;
use crate::application::environments::use_cases::{
    KillEnvironment, KilledEnvironment, ListContainerConfigs, ListEnvironmentsQuery,
};
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentTarget};
use crate::domain::tool::ToolResult;

/// The environment control use cases `agent_cmd` invokes (#1369, #1939):
/// the side-effect-free inventory query and the kill owner, composed once
/// per harness over the session's environment registry.
#[derive(Clone)]
pub struct EnvironmentControl {
    pub list: Arc<ListEnvironmentsQuery>,
    pub kill: Arc<KillEnvironment>,
}

/// Where the composed environment control lives: built empty with the
/// agent-control tools, filled once by composition alongside the
/// termination owners. Empty, `get_containers` and `kill_container` report
/// the capability unavailable. A second install is ignored: one control
/// per harness. Cloning shares the slot.
#[derive(Clone, Default)]
pub struct EnvironmentControlSlot(Arc<std::sync::OnceLock<EnvironmentControl>>);

impl EnvironmentControlSlot {
    /// Install the control; `true` when this call filled the slot.
    pub fn install(&self, control: EnvironmentControl) -> bool {
        self.0.set(control).is_ok()
    }

    pub fn get(&self) -> Option<EnvironmentControl> {
        self.0.get().cloned()
    }
}

pub(super) fn is_container_command(args: &serde_json::Value) -> bool {
    matches!(
        args.get("command").and_then(|v| v.as_str()),
        Some("get_containers") | Some("kill_container") | Some("get_container_configs")
    )
}

pub(super) async fn execute_container_command(
    list_environments: Option<&Arc<ListEnvironmentsQuery>>,
    kill_environment: Option<&Arc<KillEnvironment>>,
    list_container_configs: Option<&Arc<ListContainerConfigs>>,
    args: &serde_json::Value,
) -> ToolResult {
    match args.get("agent_id").and_then(|v| v.as_str()) {
        Some("*") => {}
        _ => return error("container commands require agent_id '*'".to_string()),
    }
    match args.get("command").and_then(|v| v.as_str()) {
        Some("get_containers") => match list_environments {
            Some(query) => encode_listing(query.execute()),
            None => error("environment listing is not available in this session".to_string()),
        },
        Some("get_container_configs") => match list_container_configs {
            Some(query) => match query.execute() {
                Ok(inventory) => encode_config_inventory(&inventory),
                Err(reason) => error(reason),
            },
            None => error("container config listing is not available in this session".to_string()),
        },
        Some("kill_container") => match kill_environment {
            None => error("environment control is not available in this session".to_string()),
            Some(uc) => match decode_target(args) {
                Ok(target) => match uc.kill_container(&target).await {
                    Ok(killed) => ToolResult {
                        content: kill_container_result_json(&killed).to_string(),
                        is_error: false,
                        image_blocks: vec![],
                        delivery_metadata: None,
                    },
                    Err(e) => error(e.to_string()),
                },
                Err(e) => error(e),
            },
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

fn kill_container_result_json(killed: &KilledEnvironment) -> serde_json::Value {
    let mut result = capped_agents_json(&killed.record.members);
    result["killed"] = serde_json::json!(killed.record.environment_ref);
    // How each member was settled before the retained kill ran (#1939).
    result["settled"] = serde_json::json!(
        killed
            .members
            .settled
            .iter()
            .map(|member| {
                serde_json::json!({"agent": member.member, "result": member.result.as_str()})
            })
            .collect::<Vec<_>>()
    );
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
                // Provenance (#2024 S4d): an environment another (or an
                // earlier) session created, reachable here for a join or
                // a kill but never torn down by a joiner's exit.
                "restored": record.origin == crate::domain::environment_registry::EnvironmentOrigin::Restored,
                "session": record.created_by,
                "config": record.script_name,
                "created_at": record.created_at,
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

/// `{"container_configs":[{name, default, source, repository, problem,
/// joinable}], "overlay_withheld": bool, "diagnostics": [..]}` — the
/// default first, `source` is `overlay` (repo-bound) or `global`,
/// `repository` null for a sandbox config, `problem` null unless a launch
/// would refuse the entry, `joinable` when the config supports
/// `{"mode":"existing"}` joins (an `exec` argv).
fn encode_config_inventory(inventory: &ContainerConfigInventory) -> ToolResult {
    let configs: Vec<serde_json::Value> = inventory
        .configs
        .iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.name,
                "default": entry.default,
                "source": entry.layer.as_str(),
                "repository": entry.repository,
                "problem": entry.problem,
                "joinable": entry.joinable,
            })
        })
        .collect();
    ToolResult {
        content: serde_json::json!({
            "container_configs": configs,
            "overlay_withheld": inventory.overlay_withheld,
            "diagnostics": inventory.diagnostics,
        })
        .to_string(),
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
