//! `declare_turn_outcome` (#2708): how a trusted top-level parent says its
//! turn ends. The adapter parses the strict input, presents its authority
//! from trusted construction and the identity of the turn its binding has
//! admitted; the application decides.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Deserialize;

use crate::application::agent_turn::ports::BoundTurnDeclaration;
use crate::application::tools::ports::Tool;
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::domain::turn_declaration::{LaunchAuthority, TurnIdentity, TurnIntent};

/// The tool's name, as the Core factory defines it.
pub const DECLARE_TURN_OUTCOME_TOOL: &str = "declare_turn_outcome";
/// The provider the tool registers under.
pub const DECLARE_TURN_OUTCOME_PROVIDER_ID: &str = "quecto:turn-outcome";

const RECEIPT: &str = "Turn outcome recorded.";
const VALIDATION: &str = "Choose waiting, blocked, asking, or done and provide a nonempty one-line reason of at most 512 characters.";
const AUTHORITY: &str = "Turn outcomes may be declared only by the active top-level parent turn.";
const STALE: &str = "This declaration does not belong to the active turn.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    outcome: Intent,
    reason: String,
}

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Intent {
    Waiting,
    Blocked,
    Asking,
    Done,
}

impl Intent {
    fn typed(self) -> TurnIntent {
        match self {
            Self::Waiting => TurnIntent::Waiting,
            Self::Blocked => TurnIntent::Blocked,
            Self::Asking => TurnIntent::Asking,
            Self::Done => TurnIntent::Done,
        }
    }
}

pub struct DeclareTurnOutcomeTool {
    binding: Arc<dyn BoundTurnDeclaration>,
    authority: LaunchAuthority,
}

impl DeclareTurnOutcomeTool {
    /// A tool for a launch of `authority`, over `binding`.
    pub fn new(binding: Arc<dyn BoundTurnDeclaration>, authority: LaunchAuthority) -> Self {
        Self { binding, authority }
    }

    /// Authority first (a forged argument never reaches the parser), then
    /// the strict input, then the turn the binding admitted, then the
    /// application's own validation. Every refusal is the model's to act on.
    fn declare(&self, arguments: &str) -> Result<ToolResult, DomainError> {
        let LaunchAuthority::TopLevelParent = self.authority else {
            return Ok(refusal(AUTHORITY));
        };
        let Ok(Arguments { outcome, reason }) = serde_json::from_str::<Arguments>(arguments) else {
            return Ok(refusal(VALIDATION));
        };
        let Some(identity) = self.binding.capture() else {
            return Ok(refusal(STALE));
        };
        match self
            .binding
            .declare(self.authority, &identity, outcome.typed(), &reason)
        {
            Ok(declared) => Ok(receipt(&identity, outcome, &declared.reason)),
            Err(DomainError::Tool(message)) => Ok(refusal(&message)),
            Err(other) => Err(other),
        }
    }
}

fn refusal(message: &str) -> ToolResult {
    ToolResult {
        content: message.to_owned(),
        is_error: true,
        image_blocks: Vec::new(),
        delivery_metadata: None,
    }
}

fn receipt(identity: &TurnIdentity, intent: Intent, reason: &str) -> ToolResult {
    ToolResult {
        content: serde_json::json!({
            "message": RECEIPT,
            "intent": intent,
            "reason": reason,
            "sessionKey": identity.session_key,
            "outcomeEpoch": identity.outcome_epoch,
            "turnId": identity.turn_id,
        })
        .to_string(),
        is_error: false,
        image_blocks: Vec::new(),
        delivery_metadata: None,
    }
}

impl Tool for DeclareTurnOutcomeTool {
    /// The Core factory's definition, projected onto the schema subset
    /// every provider accepts: `reason` keeps only its type, and the
    /// service enforces its bound (trimmed, 1..=512 scalars, one line).
    fn definition(&self) -> ToolDefinition {
        let mut definition = super::turn_outcome_definition::declare_turn_outcome_definition();
        let mut schema: serde_json::Value = serde_json::from_str(&definition.parameters_schema)
            .expect("the factory's schema is JSON");
        let reason = schema.pointer_mut("/properties/reason");
        if let Some(reason) = reason.and_then(serde_json::Value::as_object_mut) {
            reason.retain(|keyword, _| keyword == "type");
        }
        definition.parameters_schema = schema.to_string().into();
        definition
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let result = self.declare(arguments);
        Box::pin(async move { result })
    }
}

#[cfg(test)]
#[path = "declare_turn_outcome_tests.rs"]
mod issue_2708_declaration_native_tests;
