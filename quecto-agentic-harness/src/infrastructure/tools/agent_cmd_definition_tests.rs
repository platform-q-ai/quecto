use super::*;
use crate::domain::tool::Tool;

#[test]
fn definition_does_not_expose_await() {
    let def = AgentCmdTool::new(new_registry()).definition();
    assert!(!def.description.contains("await"));
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let commands = schema["properties"]["command"]["enum"].as_array().unwrap();
    assert!(!commands.iter().any(|v| v.as_str() == Some("await")));
    assert!(schema["properties"].get("timeout").is_none());
    assert!(schema["properties"].get("idle_timeout").is_none());
}
