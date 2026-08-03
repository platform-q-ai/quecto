use crate::domain::tool::{
    ChildToolPolicyPropagation, ChildToolPolicyPropagationStatus, ToolPolicyApplyMode,
    ToolPolicyChildPropagator, ToolPolicyMutation, ToolPolicyMutationResult,
    ToolPolicyReconciliation,
};
use crate::infrastructure::tools::subagent_registry::{
    INSPECTOR_RESPONSE_TIMEOUT, SubagentRegistry, SubagentStatus,
    send_subagent_uds_command_with_timeout,
};

#[derive(Debug, Clone)]
pub struct SubagentPolicyGateway {
    registry: Option<SubagentRegistry>,
}

impl SubagentPolicyGateway {
    pub fn new(registry: Option<SubagentRegistry>) -> Self {
        Self { registry }
    }
}

impl ToolPolicyChildPropagator for SubagentPolicyGateway {
    fn has_children(&self) -> bool {
        self.registry.as_ref().is_some_and(|registry| {
            registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .values()
                .any(|entry| entry.status != SubagentStatus::Exited)
        })
    }

    fn propagate_tool_policy_to_children(
        &self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> Vec<ChildToolPolicyPropagation> {
        run_child_policy_propagation_on_dedicated_thread(
            self.registry.clone(),
            mutations.to_vec(),
            mode,
        )
    }
}

fn run_child_policy_propagation_on_dedicated_thread(
    registry: Option<SubagentRegistry>,
    mutations: Vec<ToolPolicyMutation>,
    mode: ToolPolicyApplyMode,
) -> Vec<ChildToolPolicyPropagation> {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                return vec![ChildToolPolicyPropagation {
                    agent_id: "*".to_string(),
                    status: ChildToolPolicyPropagationStatus::Error,
                    reconciliation: None,
                    error: Some(format!("failed to create propagation runtime: {error}")),
                }];
            }
        };
        runtime.block_on(propagate_tool_policy_to_children(
            &registry, &mutations, mode,
        ))
    })
    .join()
    .unwrap_or_else(|_| {
        vec![ChildToolPolicyPropagation {
            agent_id: "*".to_string(),
            status: ChildToolPolicyPropagationStatus::Error,
            reconciliation: None,
            error: Some("child policy propagation thread panicked".to_string()),
        }]
    })
}

#[derive(Debug, Clone)]
pub(super) struct ChildPolicyTarget {
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
            results.push(exited_target_result(target));
            continue;
        }
        let child_mode = child_apply_mode(mode, &target.status);
        let command = child_policy_command(mutations, child_mode);
        let response = send_subagent_uds_command_with_timeout(
            &target.socket_path,
            &command,
            INSPECTOR_RESPONSE_TIMEOUT,
        )
        .await;
        results.push(map_child_response(target.agent_id, response));
    }
    results
}

pub(super) fn exited_target_result(target: ChildPolicyTarget) -> ChildToolPolicyPropagation {
    ChildToolPolicyPropagation {
        agent_id: target.agent_id,
        status: ChildToolPolicyPropagationStatus::Disconnected,
        reconciliation: None,
        error: Some("child is exited".to_string()),
    }
}

pub(super) fn child_apply_mode(
    parent_mode: ToolPolicyApplyMode,
    child_status: &SubagentStatus,
) -> ToolPolicyApplyMode {
    match (parent_mode, child_status) {
        (ToolPolicyApplyMode::AtNextTurnBoundary, _) | (_, SubagentStatus::Running) => {
            ToolPolicyApplyMode::AtNextTurnBoundary
        }
        _ => ToolPolicyApplyMode::ImmediateIfIdle,
    }
}

pub(super) fn child_policy_command(
    mutations: &[ToolPolicyMutation],
    mode: ToolPolicyApplyMode,
) -> String {
    serde_json::json!({
        "type": "set_tool_policy",
        "mode": mode,
        "propagated": true,
        "mutations": mutations.iter().map(|mutation| serde_json::json!({
            "name": mutation.name,
            "scope": mutation.scope,
            "reason": mutation.reason,
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

pub(super) fn snapshot_targets(registry: &Option<SubagentRegistry>) -> Vec<ChildPolicyTarget> {
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

pub(super) fn map_child_response(
    agent_id: String,
    response: Result<String, crate::domain::error::DomainError>,
) -> ChildToolPolicyPropagation {
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            let message = error.to_string();
            let status = if message.contains("timed out") {
                ChildToolPolicyPropagationStatus::Timeout
            } else {
                ChildToolPolicyPropagationStatus::Disconnected
            };
            return ChildToolPolicyPropagation {
                agent_id,
                status,
                reconciliation: None,
                error: Some(message),
            };
        }
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
            )
        })
    }) {
        ChildToolPolicyPropagationStatus::BlockedByCeiling
    } else if reconciliation.as_ref().is_some_and(|r| {
        r.results.iter().any(|result| {
            matches!(
                result.status,
                crate::domain::tool::ToolPolicyMutationStatus::UnknownTool
            )
        })
    }) {
        ChildToolPolicyPropagationStatus::UnknownTool
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

pub(super) fn parse_reconciliation(value: &serde_json::Value) -> Option<ToolPolicyReconciliation> {
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

pub(super) fn parse_mutation_result(value: &serde_json::Value) -> Option<ToolPolicyMutationResult> {
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
