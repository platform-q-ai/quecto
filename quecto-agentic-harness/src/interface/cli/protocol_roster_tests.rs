//! The interface side of the compact roster: the `get_subagents` command and
//! its `since` cursor. The roster itself is built (and tested) beside the
//! registry in `infrastructure/tools/subagent_compact_roster.rs`.
use super::*;

#[test]
fn get_subagents_command_round_trips_since_cursor() {
    let cmd = AgentCommand::GetSubagents {
        id: Some("gs".into()),
        since: Some(42),
    };
    let value = serde_json::to_value(&cmd).unwrap();
    assert_eq!(value["type"], "get_subagents");
    assert_eq!(value["since"], 42);
    let parsed: AgentCommand = serde_json::from_value(value).unwrap();
    match parsed {
        AgentCommand::GetSubagents { id, since } => {
            assert_eq!(id.as_deref(), Some("gs"));
            assert_eq!(since, Some(42));
        }
        _ => panic!("expected get_subagents"),
    }
}
