//! Injected native boundary; intentionally absent from production registration until C4.
use std::{future::Future, pin::Pin, sync::Arc};

use crate::application::{agent_turn::ports::DeclareTurnOutcome, tools::ports::Tool};
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::domain::turn_declaration::{LaunchAuthority, TurnIdentity, TurnIntent, VALIDATION};
use serde::Deserialize;
use serde_json::json;

pub struct DeclareTurnOutcomeTool {
    service: Arc<dyn DeclareTurnOutcome>,
    authority: LaunchAuthority,
    identity: TurnIdentity,
}

// This typed field allowlist rejects duplicates, extra fields and non-string values.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    outcome: String,
    reason: String,
}

impl DeclareTurnOutcomeTool {
    pub fn new(
        service: Arc<dyn DeclareTurnOutcome>,
        authority: LaunchAuthority,
        identity: TurnIdentity,
    ) -> Self {
        Self {
            service,
            authority,
            identity,
        }
    }

    fn record(&self, arguments: &str) -> Result<ToolResult, DomainError> {
        self.authority.admit()?;
        let input: Arguments =
            serde_json::from_str(arguments).map_err(|_| DomainError::Tool(VALIDATION.into()))?;
        let intent = match input.outcome.as_str() {
            "waiting" => TurnIntent::Waiting,
            "blocked" => TurnIntent::Blocked,
            "asking" => TurnIntent::Asking,
            "done" => TurnIntent::Done,
            _ => return Err(DomainError::Tool(VALIDATION.into())),
        };
        let receipt =
            self.service
                .declare(self.authority, &self.identity, intent, &input.reason)?;
        Ok(ToolResult {
            content: json!({"message": "Turn outcome recorded.", "intent": input.outcome,
            "reason": receipt.reason, "sessionKey": receipt.identity.session_key,
            "outcomeEpoch": receipt.identity.outcome_epoch, "turnId": receipt.identity.turn_id})
            .to_string(),
            is_error: false,
            image_blocks: Vec::new(),
            delivery_metadata: None,
        })
    }
}

impl Tool for DeclareTurnOutcomeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "declare_turn_outcome".into(),
            description: "Declare how this top-level turn ends. Use waiting only for work already running, blocked when you need the human, asking for a question to the human, or done when the task is complete. Include a one-line reason. This does not launch work.".into(),
            parameters_schema: json!({"type":"object", "additionalProperties":false, "required":["outcome","reason"], "properties":{
                "outcome":{"type":"string","enum":["waiting","blocked","asking","done"]},
                "reason":{"type":"string","minLength":1,"maxLength":512}}}).to_string().into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        // Complete synchronously: no lock crosses an await and the future borrows no arguments.
        let result = match self.record(arguments) {
            Err(DomainError::Tool(content)) => Ok(ToolResult {
                content,
                is_error: true,
                image_blocks: Vec::new(),
                delivery_metadata: None,
            }),
            other => other,
        };
        Box::pin(async move { result })
    }
}

#[cfg(test)]
#[path = "declare_turn_outcome_tests.rs"]
mod issue_2708_declaration_tests;
