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
mod tests {
    use super::declare_turn_outcome_definition;
    use serde_json::json;

    #[test]
    fn issue_2708_definition_reason_max_length_is_512() {
        let definition = declare_turn_outcome_definition();
        let schema = serde_json::from_str::<serde_json::Value>(&definition.parameters_schema)
            .expect("tool schema is JSON");

        assert_eq!(schema["properties"]["reason"]["maxLength"], json!(512));
    }

    #[test]
    fn issue_2708_definition_outcome_enum_is_exact() {
        let definition = declare_turn_outcome_definition();
        let schema = serde_json::from_str::<serde_json::Value>(&definition.parameters_schema)
            .expect("tool schema is JSON");

        assert_eq!(
            schema["properties"]["outcome"]["enum"],
            json!(["waiting", "blocked", "asking", "done"])
        );
    }

    #[test]
    fn issue_2708_definition_metadata_is_strict() {
        let definition = declare_turn_outcome_definition();
        let schema = serde_json::from_str::<serde_json::Value>(&definition.parameters_schema)
            .expect("tool schema is JSON");

        assert_eq!(definition.name, "declare_turn_outcome");
        assert_eq!(
            definition.description,
            "Declare how this top-level turn ends. Use waiting only for work already running, blocked when you need the human, asking for a question to the human, or done when the task is complete. Include a one-line reason. This does not launch work."
        );
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["required"], json!(["outcome", "reason"]));
        assert_eq!(schema["properties"]["outcome"]["type"], json!("string"));
        assert_eq!(schema["properties"]["reason"]["type"], json!("string"));
        assert_eq!(schema["properties"]["reason"]["minLength"], json!(1));
    }
}
