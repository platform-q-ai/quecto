use crate::infrastructure::tools::subagent_lifecycle::{
    SubagentLifecycleEvent, SubagentLifecycleState, apply_lifecycle_event,
};
use crate::infrastructure::tools::subagent_status::SubagentStatus;

#[test]
fn lifecycle_terminal_and_failure_transitions_are_stable() {
    assert_eq!(
        SubagentLifecycleState::Exited.transition(SubagentLifecycleEvent::SocketConnected),
        SubagentLifecycleState::Exited
    );
    assert_eq!(
        SubagentLifecycleState::Exited.transition(SubagentLifecycleEvent::KillRequested),
        SubagentLifecycleState::Killed
    );
    assert_eq!(
        SubagentLifecycleState::Failed.transition(SubagentLifecycleEvent::ToolStarted),
        SubagentLifecycleState::Failed
    );
    assert_eq!(
        SubagentLifecycleState::Launched.transition(SubagentLifecycleEvent::RunFailed),
        SubagentLifecycleState::Failed
    );
}

#[test]
fn lifecycle_projection_and_status_reconstruction_cover_all_variants() {
    assert_eq!(
        SubagentLifecycleState::Launched.status_projection(),
        SubagentStatus::Starting
    );
    assert_eq!(
        SubagentLifecycleState::SocketReady.status_projection(),
        SubagentStatus::Starting
    );
    assert_eq!(
        SubagentLifecycleState::Busy.status_projection(),
        SubagentStatus::Running
    );
    assert_eq!(
        SubagentLifecycleState::Idle.status_projection(),
        SubagentStatus::Idle
    );
    assert_eq!(
        SubagentLifecycleState::Failed.status_projection(),
        SubagentStatus::Error
    );
    assert_eq!(
        SubagentLifecycleState::Exited.status_projection(),
        SubagentStatus::Exited
    );
    assert_eq!(
        SubagentLifecycleState::Killed.status_projection(),
        SubagentStatus::Exited
    );

    assert_eq!(
        SubagentLifecycleState::from_status(&SubagentStatus::Starting),
        SubagentLifecycleState::Launched
    );
    assert_eq!(
        SubagentLifecycleState::from_status(&SubagentStatus::Idle),
        SubagentLifecycleState::Idle
    );
    assert_eq!(
        SubagentLifecycleState::from_status(&SubagentStatus::Running),
        SubagentLifecycleState::Busy
    );
    assert_eq!(
        SubagentLifecycleState::from_status(&SubagentStatus::Error),
        SubagentLifecycleState::Failed
    );
    assert_eq!(
        SubagentLifecycleState::from_status(&SubagentStatus::Exited),
        SubagentLifecycleState::Exited
    );
}

#[test]
fn apply_lifecycle_event_updates_state_and_returns_projected_status() {
    let mut state = SubagentLifecycleState::Launched;
    assert_eq!(
        apply_lifecycle_event(&mut state, SubagentLifecycleEvent::SocketConnected),
        SubagentStatus::Starting
    );
    assert_eq!(state, SubagentLifecycleState::SocketReady);
    assert_eq!(
        apply_lifecycle_event(&mut state, SubagentLifecycleEvent::RunStarted),
        SubagentStatus::Running
    );
    assert_eq!(state, SubagentLifecycleState::Busy);
    assert_eq!(
        apply_lifecycle_event(&mut state, SubagentLifecycleEvent::RunEnded),
        SubagentStatus::Idle
    );
    assert_eq!(state, SubagentLifecycleState::Idle);
}

#[test]
fn lifecycle_socket_ready_failure_and_exit_paths_project_terminal_statuses() {
    assert_eq!(
        SubagentLifecycleState::SocketReady.transition(SubagentLifecycleEvent::SocketConnectFailed),
        SubagentLifecycleState::Exited
    );
    assert_eq!(
        SubagentLifecycleState::SocketReady.transition(SubagentLifecycleEvent::ProcessExited),
        SubagentLifecycleState::Exited
    );
}

#[test]
fn lifecycle_idle_restart_and_failure_paths_are_observable() {
    assert_eq!(
        SubagentLifecycleState::Idle.transition(SubagentLifecycleEvent::ToolStarted),
        SubagentLifecycleState::Busy
    );
    assert_eq!(
        SubagentLifecycleState::Idle.transition(SubagentLifecycleEvent::RunFailed),
        SubagentLifecycleState::Failed
    );
}

#[test]
fn lifecycle_busy_kill_and_process_exit_paths_are_terminal() {
    assert_eq!(
        SubagentLifecycleState::Busy.transition(SubagentLifecycleEvent::KillRequested),
        SubagentLifecycleState::Killed
    );
    assert_eq!(
        SubagentLifecycleState::Busy.transition(SubagentLifecycleEvent::ProcessExited),
        SubagentLifecycleState::Exited
    );
}

#[test]
fn lifecycle_killed_state_is_sticky_except_process_exit() {
    assert_eq!(
        SubagentLifecycleState::Killed.transition(SubagentLifecycleEvent::RunStarted),
        SubagentLifecycleState::Killed
    );
    assert_eq!(
        SubagentLifecycleState::Killed.transition(SubagentLifecycleEvent::ProcessExited),
        SubagentLifecycleState::Killed
    );
}

#[test]
fn apply_lifecycle_event_covers_kill_terminal_projection() {
    let mut state = SubagentLifecycleState::Busy;
    assert_eq!(
        apply_lifecycle_event(&mut state, SubagentLifecycleEvent::KillRequested),
        SubagentStatus::Exited
    );
    assert_eq!(state, SubagentLifecycleState::Killed);
}

#[test]
fn lifecycle_launched_duplicate_and_tool_started_paths_are_observable() {
    assert_eq!(
        SubagentLifecycleState::Launched.transition(SubagentLifecycleEvent::SocketConnected),
        SubagentLifecycleState::SocketReady
    );
    assert_eq!(
        SubagentLifecycleState::Launched.transition(SubagentLifecycleEvent::ToolStarted),
        SubagentLifecycleState::Busy
    );
    assert_eq!(
        SubagentLifecycleState::Launched.transition(SubagentLifecycleEvent::PassiveNoteEmitted),
        SubagentLifecycleState::Launched
    );
}
