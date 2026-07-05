Feature: Subagent status bar fixes (#534)
  As a human operator using the TUI
  I want the spinner to render below status bars, live updates to propagate,
  and bars to clear after subagent kill
  So that the TUI provides accurate real-time feedback

  # ── Fix 1: sub-agent bar relocated to the always-on left panel (#820) ──
  # The sub-agent bar (widgets_above) no longer renders in the bottom section;
  # it moved to the always-on left panel, so the bottom keeps only the spinner.

  @done
  Scenario: The sub-agent bar no longer renders in the bottom section
    Given a render bottom section layout
    Then the bottom section no longer renders the sub-agent bar

  # ── Fix 2: Notifications drain during prompt execution ─────────────
  # Verified by the notification_rx integration in run_with_token_drain_broadcast.

  @done
  Scenario: SubagentStateChanged event includes correct subagent list
    Given a subagent registry with agent "worker" status "running"
    When I build the subagent info list from the registry
    Then the list should contain 1 entry
    And the entry agent_id should be "worker"
    And the entry status should be "running"

  # ── Fix 3: Bars clear when subagents are killed ────────────────────

  @done
  Scenario: Exited subagent appears in state_changed event
    Given a subagent registry with agent "worker" status "exited"
    When I build the subagent info list from the registry
    Then the list should contain 1 entry
    And the entry status should be "exited"
