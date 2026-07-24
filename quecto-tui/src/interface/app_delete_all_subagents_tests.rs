use super::tui_harness::TuiHarness;
use crate::infrastructure::client::{Event, SubagentInfoEvent};

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn subagent(id: &str) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_id: id.into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 1,
        socket_path: None,
        parent_id: None,
        read_only: false,
        workflow: None,
    }
}

#[tokio::test]
async fn handle_submit_delete_all_subagents_sends_command_and_clears_ui() {
    let mut h = harness().await;
    h.app_mut().handle_event(Event::SubagentStateChanged {
        subagents: vec![subagent("worker")],
    });

    h.app_mut().handle_submit("/delete-all-subagents");

    let cmds = h.drain_commands().await;
    assert_eq!(cmds.len(), 1, "expected one agent command: {cmds:?}");
    assert!(
        cmds[0].contains("\"type\":\"delete_all_subagents\""),
        "slash command should send delete_all_subagents: {cmds:?}"
    );
    assert_eq!(
        h.subagent_group_tracked(),
        0,
        "subagent panel should be cleared optimistically"
    );
}

#[tokio::test]
async fn handle_submit_delete_all_subagents_preserves_ui_when_command_send_fails() {
    let mut h = harness().await;
    h.app_mut().handle_event(Event::SubagentStateChanged {
        subagents: vec![subagent("worker")],
    });
    h.disconnect_master_commands();

    h.app_mut().handle_submit("/delete-all-subagents");

    let failure = h
        .app_mut()
        .command_send_failure_rx
        .recv()
        .await
        .expect("delete-all-subagents send failure should be reported");
    h.app_mut().handle_command_send_failure(failure);
    assert_eq!(
        h.subagent_group_tracked(),
        1,
        "subagent panel must not be cleared when delete command was not enqueued"
    );
}
