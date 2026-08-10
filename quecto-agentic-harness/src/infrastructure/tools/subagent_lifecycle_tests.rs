use super::{SubagentLifecycleEvent as Event, SubagentLifecycleState as State};
use crate::infrastructure::tools::subagent_registry::SubagentStatus;

#[test]
fn newly_launched_child_reports_starting_until_socket_is_ready() {
    let state = State::default();

    assert_eq!(state, State::Launched);
    assert_eq!(state.status_projection(), SubagentStatus::Starting);
}

#[test]
fn socket_ready_child_still_reports_starting_until_a_run_is_observed() {
    let state = State::default().transition(Event::SocketConnected);

    assert_eq!(state, State::SocketReady);
    assert_eq!(state.status_projection(), SubagentStatus::Starting);
}

#[test]
fn child_exit_before_socket_ready_is_terminal() {
    let state = State::Launched.transition(Event::SocketConnectFailed);

    assert_eq!(state, State::Exited);
    assert_eq!(state.status_projection(), SubagentStatus::Exited);
    assert_eq!(state.transition(Event::SocketConnected), State::Exited);
}

#[test]
fn passive_note_emission_keeps_completed_child_idle() {
    let state = State::Busy.transition(Event::RunEnded);

    assert_eq!(state, State::Idle);
    assert_eq!(state.transition(Event::PassiveNoteEmitted), State::Idle);
}

#[test]
fn passive_note_emission_is_non_terminal_and_does_not_imply_await() {
    let ready = State::SocketReady.transition(Event::PassiveNoteEmitted);
    let busy = State::Busy.transition(Event::PassiveNoteEmitted);
    let failed = State::Failed.transition(Event::PassiveNoteEmitted);

    assert_eq!(ready, State::SocketReady);
    assert_eq!(ready.status_projection(), SubagentStatus::Starting);
    assert_eq!(busy, State::Busy);
    assert_eq!(busy.status_projection(), SubagentStatus::Running);
    assert_eq!(failed, State::Failed);
    assert_eq!(failed.status_projection(), SubagentStatus::Error);
}

#[test]
fn kill_after_passive_completion_remains_cancelled_terminal() {
    let state = State::Busy
        .transition(Event::RunEnded)
        .transition(Event::PassiveNoteEmitted)
        .transition(Event::KillRequested);

    assert_eq!(state, State::Killed);
    assert_eq!(state.status_projection(), SubagentStatus::Exited);
    assert_eq!(state.transition(Event::RunStarted), State::Killed);
    assert_eq!(state.transition(Event::PassiveNoteEmitted), State::Killed);
}

#[test]
fn kill_during_busy_is_terminal_and_projects_to_existing_exited_status() {
    let state = State::Busy.transition(Event::KillRequested);

    assert_eq!(state, State::Killed);
    assert_eq!(state.status_projection(), SubagentStatus::Exited);
    assert_eq!(state.transition(Event::RunEnded), State::Killed);
}

#[test]
fn tool_failure_stays_sticky_until_current_tool_or_run_ends() {
    let state = State::Busy.transition(Event::RunFailed);

    assert_eq!(state, State::Failed);
    assert_eq!(state.status_projection(), SubagentStatus::Error);
    assert_eq!(state.transition(Event::ToolStarted), State::Failed);
}

#[test]
fn recoverable_tool_failure_returns_to_idle_when_run_ends() {
    let state = State::Busy.transition(Event::RunFailed);

    assert_eq!(state.transition(Event::RunEnded), State::Idle);
}

#[test]
fn failed_child_can_recover_when_a_new_agent_run_starts() {
    let state = State::Busy.transition(Event::RunFailed);

    assert_eq!(state.transition(Event::RunStarted), State::Busy);
}

#[test]
fn existing_registry_statuses_are_interpreted_as_equivalent_lifecycle_states() {
    assert_eq!(
        State::from_status(&SubagentStatus::Starting),
        State::Launched
    );
    assert_eq!(State::from_status(&SubagentStatus::Running), State::Busy);
    assert_eq!(State::from_status(&SubagentStatus::Idle), State::Idle);
    assert_eq!(State::from_status(&SubagentStatus::Error), State::Failed);
    assert_eq!(State::from_status(&SubagentStatus::Exited), State::Exited);
}
