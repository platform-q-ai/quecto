//! `agent_cmd kill` as a tool adapter (#1936, #1882): parse the command's
//! arguments, invoke the application's selected termination, present the
//! outcome. No lifecycle decision lives here — which edge is routed, what
//! is observed before the row goes, when a fallback is authorised — the
//! use case owns all of it; this file only translates.
use std::sync::Arc;
use std::{future::Future, pin::Pin};

use crate::application::subagents::dto::{
    KillDelegatedAgentError, KillDelegatedAgentOutcome, KillDelegatedAgentRequest,
};
use crate::application::subagents::use_cases::KillDelegatedAgent;
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Name under which `AgentCmdTool` delegates its `kill` command.
pub const KILL_TOOL_NAME: &str = "agent_cmd:kill";

/// Longest list of removed agents echoed back to the model.
const MAX_REPORTED_AGENTS: usize = 20;

pub struct KillDelegatedAgentTool {
    use_case: Arc<KillDelegatedAgent>,
}

impl KillDelegatedAgentTool {
    pub fn new(use_case: Arc<KillDelegatedAgent>) -> Self {
        Self { use_case }
    }
}

/// The one argument the command takes, validated as a display label or
/// uuid the way every other `agent_cmd` command validates `agent_id`.
pub fn parse_kill_arguments(arguments: &str) -> Result<KillDelegatedAgentRequest, String> {
    let args: serde_json::Value =
        serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {e}"))?;
    let agent_id = args
        .get("agent_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing required field: agent_id".to_string())?;
    // Delivery-specific syntax only (the same shape every `agent_cmd`
    // command accepts); whether the reference names anything is the use
    // case's answer.
    if agent_id.is_empty() || agent_id.len() > 64 {
        return Err("agent_id must be 1-64 characters".to_string());
    }
    if !agent_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err("agent_id must use only [a-zA-Z0-9_-]".to_string());
    }
    Ok(KillDelegatedAgentRequest {
        reference: agent_id.to_owned(),
    })
}

pub fn present_outcome(outcome: &KillDelegatedAgentOutcome) -> ToolResult {
    let killed: Vec<&str> = outcome
        .removed
        .iter()
        .take(MAX_REPORTED_AGENTS)
        .map(|uuid| uuid.as_str())
        .collect();
    let mut body = serde_json::json!({
        "target": outcome.target.uuid.as_str(),
        "result": outcome.result.as_str(),
        "killed": killed,
    });
    if outcome.removed.len() > MAX_REPORTED_AGENTS {
        body["omitted_agents"] = serde_json::json!(outcome.removed.len() - MAX_REPORTED_AGENTS);
    }
    result(body.to_string(), false)
}

pub fn present_error(reference: &str, error: &KillDelegatedAgentError) -> ToolResult {
    match error {
        KillDelegatedAgentError::Failed { detail } => result(
            serde_json::json!({
                "result": "failed",
                "target": reference,
                "error": detail,
            })
            .to_string(),
            true,
        ),
        KillDelegatedAgentError::Unresolved(_) => result(
            format!("agent_cmd error: subagent '{reference}' {error}"),
            true,
        ),
        KillDelegatedAgentError::AlreadyStopping
        | KillDelegatedAgentError::Rejected(_)
        | KillDelegatedAgentError::NotAccepting
        | KillDelegatedAgentError::RouteUnreachable { .. } => {
            result(format!("agent_cmd error: {error}"), true)
        }
    }
}

fn result(content: String, is_error: bool) -> ToolResult {
    ToolResult {
        content,
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

impl Tool for KillDelegatedAgentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: KILL_TOOL_NAME.into(),
            description: "Terminate one delegated agent by uuid or display label; its subtree ends with it.".into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string"}},"required":["agent_id"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let arguments = arguments.to_owned();
        Box::pin(async move {
            let request = match parse_kill_arguments(&arguments) {
                Ok(request) => request,
                Err(error) => return Ok(result(format!("agent_cmd error: {error}"), true)),
            };
            let reference = request.reference.clone();
            Ok(match self.use_case.execute(request).await {
                Ok(outcome) => present_outcome(&outcome),
                Err(error) => present_error(&reference, &error),
            })
        })
    }
}

#[cfg(test)]
#[path = "agent_cmd_kill_tests.rs"]
mod tests;
