use super::protocol_commands::AgentCommand;

#[test]
fn test_parse_persist_session_ordinary_exit_barrier() {
    let json = r#"{"type":"persist_session","id":"tab1:persist-exit","restoreReason":"ordinary_tui_exit_stopped"}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id(), Some("tab1:persist-exit"));
    match cmd {
        AgentCommand::PersistSession { restore_reason, .. } => {
            assert_eq!(restore_reason.as_deref(), Some("ordinary_tui_exit_stopped"));
        }
        _ => panic!("expected PersistSession"),
    }
}
