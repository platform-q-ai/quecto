@done @tui
Feature: Sub-agent session view + interaction parity, Tab focus model, focus divider (#802)
  As a human operator driving a workflow with sub-agents in the TUI
  I want a selected sub-agent to render its OWN full session (chat + workflow bar +
  footer gauges + spinner), to send prompts/follow-ups, and a NeoVim-ish two-pane focus
  model with a focus-highlighted divider
  So that selecting a sub-agent resumes that agent's own session rather than a
  different chat in the master shell

  # Wired to step definitions in tests/bdd/tui_subagent_parity_steps.rs, which
  # drive the REAL render/key path through the headless render harness
  # (quecto_tui::shell::app::tui_harness) so these scenarios actually
  # execute in the non-real BDD suite (#805). The same behaviour is also
  # covered by the unit tests in
  # quecto-tui/src/shell/app_focus_parity_tests.rs.

  Scenario: Selecting a sub-agent renders its own session chrome
    Given a TUI tracking sub-agent "a1" with its own workflow
    When I select sub-agent "a1"
    Then the active session is "a1"
    And the view shows sub-agent "a1"'s own workflow

  Scenario: Returning to the master restores the master's session
    Given a TUI tracking sub-agent "a1" with its own workflow
    And I have selected sub-agent "a1"
    When I return to the master
    Then the active session is the master
    And the view no longer shows the sub-agent's workflow

  Scenario: Selecting a sub-agent shows its own footer gauges
    Given a TUI tracking sub-agent "a1" with its own model and context usage
    When I select sub-agent "a1"
    Then the footer shows the sub-agent's own model and context usage
    When I return to the master
    Then the footer shows the master's own model and context usage

  Scenario: Tab toggles focus between input and panel
    Given a TUI tracking sub-agent "a1"
    When I press Tab
    Then focus is on the panel
    When I press Tab again
    Then focus is on the input

  Scenario: Tab keeps completing while an autocomplete popup is open
    Given a TUI tracking sub-agent "a1" with an open autocomplete popup
    When I press Tab
    Then focus stays on the input

  Scenario: Panel focus moves the highlight without changing the active session
    Given a TUI tracking sub-agent "a1" with focus on the panel
    When I move the highlight down
    Then the active session is unchanged
    And focus stays on the panel

  Scenario: Digits jump the highlight to a numbered row
    Given a TUI tracking two sub-agents with focus on the panel
    When I press digit "2"
    Then the active session is unchanged
    When I press Enter
    Then the active session is "a1"

  Scenario: Enter commits the highlighted agent and makes its session active
    Given a TUI tracking two sub-agents with focus on the panel
    When I move the highlight down
    And I press Enter
    Then the active session is "a1"
    And focus is on the input

  Scenario: Esc cancels panel focus without changing the selection
    Given a TUI viewing sub-agent "a1" with focus on the panel
    When I move the highlight down
    And I press Esc
    Then focus is on the input
    And the active session is "a1"

  Scenario: Sending while a sub-agent is active targets that sub-agent
    Given a TUI viewing sub-agent "a1"
    When I send the prompt "message for child"
    Then the prompt appears in sub-agent "a1"'s session
    And no prompt is sent to the master

  Scenario: Enter while a running sub-agent is active queues a follow-up without claiming steer
    Given a TUI viewing running sub-agent "a1"
    When I send the prompt "message for running child"
    Then the follow-up is sent to sub-agent "a1"
    And the sub-agent command does not claim steer
    And no prompt is sent to the master

  Scenario: Aborting while a sub-agent is active targets that sub-agent
    Given a TUI viewing sub-agent "a1"
    When I abort
    Then no abort is sent to the master

  Scenario: Focus-highlighted divider between the panel and the body
    Given a TUI tracking sub-agent "a1"
    Then a vertical divider is drawn between the panel and the body
    When I press Tab
    Then the divider styling reflects the focused pane
