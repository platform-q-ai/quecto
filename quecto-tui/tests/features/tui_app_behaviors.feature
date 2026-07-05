@done @tui
Feature: TUI app event routing and command behaviours
  As a TUI operator
  I want responses, rewind actions, submitted prompts, and sub-agent streams to route to the right visible session
  So that the interface stays consistent with the conversation I am controlling

  Scenario: Successful session stats response updates the footer without a chat notification
    Given a fresh TUI app harness
    When a quiet session stats footer response arrives with cost "$0.1234" and context "42k"
    Then the footer shows cost "$0.1234" and context "42k"
    And the chat transcript does not show a session stats notification

  Scenario: Failed model switch response is shown as an error notification
    Given a fresh TUI app harness
    When a model switch response fails with "model not found"
    Then the app notification includes "Model switch failed: model not found"

  Scenario: Rewind selector opens from history and applies the selected turn
    Given a fresh TUI app harness
    When I request rewind history with two prior user turns
    And I choose the most recent rewind target
    Then a rewind command is sent for the most recent user turn

  Scenario: Successful rewind refreshes the conversation
    Given a fresh TUI app harness
    When I request rewind history with two prior user turns
    And I choose the most recent rewind target
    And the rewind apply response succeeds
    Then the app notification includes "Rewound conversation"
    And a rewind refresh command is sent

  Scenario: Master submit while streaming sends a steer prompt
    Given a fresh TUI app harness
    And the master assistant is currently streaming
    When I submit the master prompt "add more detail"
    Then the master prompt command includes streaming behavior "steer"
    And the master chat shows "add more detail"

  Scenario: Sub-agent live stream updates only the selected sub-agent session
    Given a TUI viewing sub-agent "a1"
    When sub-agent "a1" streams token "child-only-token"
    Then the selected sub-agent session shows "child-only-token"
    When I return to the master
    Then the app master session does not show "child-only-token"

  Scenario: Sub-agent get_state snapshot updates its own footer
    Given a TUI viewing sub-agent "a1"
    When sub-agent "a1" reports model "child-model" and context "12k"
    Then the footer shows the sub-agent model "child-model" and context "12k"
