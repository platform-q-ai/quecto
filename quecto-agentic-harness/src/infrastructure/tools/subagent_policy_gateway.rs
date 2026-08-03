use crate::domain::tool::{
    ChildToolPolicyPropagation, ChildToolPolicyPropagationStatus, ToolPolicyApplyMode,
    ToolPolicyMutation, ToolPolicyMutationResult, ToolPolicyReconciliation,
};
use crate::infrastructure::tools::subagent_registry::{
    INSPECTOR_RESPONSE_TIMEOUT, SubagentRegistry, SubagentStatus,
    send_subagent_uds_command_with_timeout,
};

#[derive(Debug, Clone)]
struct ChildPolicyTarget {
    agent_id: String,
    socket_path: std::path::PathBuf,
    status: SubagentStatus,
}

pub async fn propagate_tool_policy_to_children(
    registry: &Option<SubagentRegistry>,
    mutations: &[ToolPolicyMutation],
    mode: ToolPolicyApplyMode,
) -> Vec<ChildToolPolicyPropagation> {
    let targets = snapshot_targets(registry);
    let mut results = Vec::with_capacity(targets.len());
    for target in targets {
        if target.status == SubagentStatus::Exited {
            results.push(ChildToolPolicyPropagation {
                agent_id: target.agent_id,
                status: ChildToolPolicyPropagationStatus::Disconnected,
                reconciliation: None,
                error: Some("child is exited".to_string()),
            });
            continue;
        }
        let child_mode = match (mode, &target.status) {
            (ToolPolicyApplyMode::AtNextTurnBoundary, _) | (_, SubagentStatus::Running) => {
                ToolPolicyApplyMode::AtNextTurnBoundary
            }
            _ => ToolPolicyApplyMode::ImmediateIfIdle,
        };
        let command = serde_json::json!({
            "type": "set_tool_policy",
            "mode": child_mode,
            "mutations": mutations.iter().map(|mutation| serde_json::json!({
                "name": mutation.name,
                "scope": mutation.scope,
                "reason": mutation.reason,
            })).collect::<Vec<_>>(),
        });
        let response = send_subagent_uds_command_with_timeout(
            &target.socket_path,
            &command.to_string(),
            INSPECTOR_RESPONSE_TIMEOUT,
        )
        .await;
        results.push(map_child_response(target.agent_id, response));
    }
    results
}

fn snapshot_targets(registry: &Option<SubagentRegistry>) -> Vec<ChildPolicyTarget> {
    let Some(registry) = registry else {
        return Vec::new();
    };
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .iter()
        .map(|(agent_id, entry)| ChildPolicyTarget {
            agent_id: agent_id.clone(),
            socket_path: entry.socket_path.clone(),
            status: entry.status.clone(),
        })
        .collect()
}

fn map_child_response(
    agent_id: String,
    response: Result<String, crate::domain::error::DomainError>,
) -> ChildToolPolicyPropagation {
    let Ok(response) = response else {
        return ChildToolPolicyPropagation {
            agent_id,
            status: ChildToolPolicyPropagationStatus::Disconnected,
            reconciliation: None,
            error: Some(response.err().unwrap().to_string()),
        };
    };
    let value: serde_json::Value = match serde_json::from_str(&response) {
        Ok(value) => value,
        Err(error) => {
            return ChildToolPolicyPropagation {
                agent_id,
                status: ChildToolPolicyPropagationStatus::Error,
                reconciliation: None,
                error: Some(format!("invalid child response: {error}")),
            };
        }
    };
    let data = value.get("data").cloned().unwrap_or(value);
    let queued = data
        .get("queued")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reconciliation = parse_reconciliation(&data);
    let status = if queued {
        ChildToolPolicyPropagationStatus::Queued
    } else if reconciliation.as_ref().is_some_and(|r| {
        r.results.iter().any(|result| {
            matches!(
                result.status,
                crate::domain::tool::ToolPolicyMutationStatus::BlockedByRestriction
                    | crate::domain::tool::ToolPolicyMutationStatus::UnknownTool
            )
        })
    }) {
        ChildToolPolicyPropagationStatus::BlockedByCeiling
    } else if reconciliation.is_some() {
        ChildToolPolicyPropagationStatus::Applied
    } else {
        ChildToolPolicyPropagationStatus::Error
    };
    ChildToolPolicyPropagation {
        agent_id,
        status,
        reconciliation: reconciliation.map(Box::new),
        error: None,
    }
}

fn parse_reconciliation(value: &serde_json::Value) -> Option<ToolPolicyReconciliation> {
    let mode = match value.get("mode")?.as_str()? {
        "immediateIfIdle" => ToolPolicyApplyMode::ImmediateIfIdle,
        "atNextTurnBoundary" => ToolPolicyApplyMode::AtNextTurnBoundary,
        _ => return None,
    };
    let results = value
        .get("results")?
        .as_array()?
        .iter()
        .map(parse_mutation_result)
        .collect::<Option<Vec<_>>>()?;
    Some(ToolPolicyReconciliation {
        mode,
        results,
        child_propagation: Vec::new(),
    })
}

fn parse_mutation_result(value: &serde_json::Value) -> Option<ToolPolicyMutationResult> {
    Some(ToolPolicyMutationResult {
        name: value.get("name")?.as_str()?.to_string(),
        requested_availability: match value.get("requestedAvailability")?.as_str()? {
            "enabled" => crate::domain::tool_descriptor::ToolAvailability::Enabled,
            "disabled" => crate::domain::tool_descriptor::ToolAvailability::Disabled,
            _ => return None,
        },
        requested_scope: serde_json::from_value(value.get("requestedScope")?.clone()).ok()?,
        status: serde_json::from_value(value.get("status")?.clone()).ok()?,
        before: None,
        after: None,
        reason: value.get("reason")?.as_str()?.to_string(),
    })
}
