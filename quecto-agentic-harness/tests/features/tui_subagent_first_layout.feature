@wip @tui
Feature: Sub-agent-first default layout (#820)
  As a human operator driving workflows in the TUI
  I want the left sub-agent panel always on (Master included, even solo), agent
  rows colour-coded by status, and the selected agent's workflow shown as a boxed
  one-line bar at the top of the main pane
  So that the TUI is sub-agent-first by default and the duplicated bottom
  sub-agent/workflow bars are gone

  # Wired to step definitions in tests/bdd/tui_subagent_first_layout_steps.rs,
  # which drive the REAL render path through the headless render harness
  # (quecto_tui::interface::app::tui_harness). Also covered by the unit tests in
  # quecto-tui/src/interface/app_subagent_first_tests.rs.

  Scenario: The panel is always visible with only the master
    Given a sub-agent-first TUI with no sub-agents
    Then the left panel shows the master row

  Scenario: A selected agent's workflow shows as a full-width status bar aligned to the tool/message content column in the main pane
    Given a sub-agent-first TUI tracking sub-agent "a1" with its own workflow
    When I select sub-agent "a1"
    Then the main pane shows a workflow status bar aligned to the tool/message content column
    And the bottom stack no longer shows the workflow bar

  Scenario: The bottom stack no longer shows the sub-agent bar
    Given a sub-agent-first TUI tracking sub-agent "a1" with its own workflow
    Then the bottom stack no longer shows the sub-agent bar
