use super::{AgentCommand, ToolPolicyApplyModeCommand};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

#[test]
fn set_tool_policy_command_deserializes_scope_and_mode() {
    let json = r#"{"type":"set_tool_policy","id":"p1","mode":"atNextTurnBoundary","mutations":[{"name":"alpha","scope":"child","reason":"test"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::SetToolPolicy {
            id,
            mutations,
            mode,
        } => {
            assert_eq!(id.as_deref(), Some("p1"));
            assert_eq!(mode, ToolPolicyApplyModeCommand::AtNextTurnBoundary);
            assert_eq!(mutations[0].name.as_deref(), Some("alpha"));
            assert_eq!(mutations[0].scope, ProfileAvailabilityScope::Child);
        }
        other => panic!("expected SetToolPolicy, got {other:?}"),
    }
}
