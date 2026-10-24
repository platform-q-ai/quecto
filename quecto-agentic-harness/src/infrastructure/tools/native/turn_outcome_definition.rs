use crate::domain::tool_policy::value_objects::tool::ToolDefinition;

/// Returns reusable metadata for the staged, unregistered declaration input contract.
///
/// A future Native tool must reuse this factory in its `definition` method.
/// The raw schema limit is stricter than the service's trim-first scalar limit.
pub fn declare_turn_outcome_definition() -> ToolDefinition {
    ToolDefinition {
        name: "declare_turn_outcome".into(),
        description: "Declare how this top-level turn ends. Use waiting only for work already running, blocked when you need the human, asking for a question to the human, or done when the task is complete. Include a one-line reason. This does not launch work.".into(),
        parameters_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["outcome", "reason"],
            "properties": {
                "outcome": {
                    "type": "string",
                    "enum": ["waiting", "blocked", "asking", "done"]
                },
                "reason": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 512
                }
            }
        })
        .to_string()
        .into(),
    }
}

#[cfg(test)]
#[path = "turn_outcome_definition_tests.rs"]
mod issue_2708_definition_tests;
