Feature: Subagent status bar fixes (#534)
  As a human operator using the TUI
  I want the spinner to render below status bars, live updates to propagate,
  and bars to clear after subagent kill
  So that the TUI provides accurate real-time feedback

  # ── Fix 1: Spinner renders below widgets_above ─────────────────────
  # Verified by unit tests in quecto-tui/src/interface/app.rs and code structure.

  @wip
  Scenario: Render order places widgets_above before spinner
    Given a render bottom section layout
    Then widgets_above should come before spinner in the output order

  # ── Fix 2: Notifications drain during prompt execution ─────────────
  # Verified by the notification_rx integration in run_with_token_drain_broadcast.

  @wip
  Scenario: SubagentStateChanged event includes correct subagent list
    Given a subagent registry with agent "worker" status "running"
    When I build the subagent info list from the registry
    Then the list should contain 1 entry
    And the entry agent_id should be "worker"
    And the entry status should be "running"

  # ── Fix 3: Bars clear when subagents are killed ────────────────────

  @wip
  Scenario: Exited subagent appears in state_changed event
    Given a subagent registry with agent "worker" status "exited"
    When I build the subagent info list from the registry
    Then the list should contain 1 entry
    And the entry status should be "exited"
