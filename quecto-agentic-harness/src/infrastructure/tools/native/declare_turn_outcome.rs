//! Inactive native JSON boundary; intentionally absent from production registration.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::agent_turn::declaration::TurnDeclarationService;
use crate::application::tools::ports::Tool;
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::domain::turn_declaration::{LaunchAuthority, TurnIdentity};

pub struct DeclareTurnOutcomeTool {
    _service: Arc<TurnDeclarationService>,
    _authority: LaunchAuthority,
    _identity: TurnIdentity,
}

impl DeclareTurnOutcomeTool {
    pub fn new(
        service: Arc<TurnDeclarationService>,
        authority: LaunchAuthority,
        identity: TurnIdentity,
    ) -> Self {
        Self {
            _service: service,
            _authority: authority,
            _identity: identity,
        }
    }
}

impl Tool for DeclareTurnOutcomeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "declare_turn_outcome".into(),
            description: "unsupported declaration adapter scaffold".into(),
            parameters_schema: "{}".into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Err(DomainError::Tool(
                "turn declaration adapter is not implemented".into(),
            ))
        })
    }
}

#[cfg(test)]
#[path = "declare_turn_outcome_tests.rs"]
mod issue_2708_declaration_tests;
