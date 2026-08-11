use super::subagent_status::SubagentStatus;

#[test]
fn wire_roundtrip_covers_every_status_and_unknown_defaults_starting() {
    for (status, wire) in [
        (SubagentStatus::Starting, "starting"),
        (SubagentStatus::Idle, "idle"),
        (SubagentStatus::Running, "running"),
        (SubagentStatus::Error, "error"),
        (SubagentStatus::Exited, "exited"),
    ] {
        assert_eq!(status.to_wire_str(), wire);
        assert_eq!(SubagentStatus::from_wire_str(wire), status);
    }
    assert_eq!(
        SubagentStatus::from_wire_str("mystery"),
        SubagentStatus::Starting
    );
}

#[test]
fn display_is_stable_title_case_for_all_statuses() {
    assert_eq!(SubagentStatus::Starting.to_string(), "Starting");
    assert_eq!(SubagentStatus::Idle.to_string(), "Idle");
    assert_eq!(SubagentStatus::Running.to_string(), "Running");
    assert_eq!(SubagentStatus::Error.to_string(), "Error");
    assert_eq!(SubagentStatus::Exited.to_string(), "Exited");
}
