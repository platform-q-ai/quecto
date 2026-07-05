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

  @app-response @session-payloads
  Scenario: Interactive session stats response adds a chat summary and footer values
    Given a fresh TUI app harness
    When an interactive session stats response arrives for "cli:demo" with cost "$0.5000" and tokens 123 input 456 output
    Then the footer shows cost "$0.5000" and context "8.0k"
    And the app master session shows "Session: cli:demo"
    And the app master session shows "Tokens: ↑123 ↓456"

  @app-response @session-payloads
  Scenario: Malformed resumed messages payload preserves existing chat and reports the error
    Given a fresh TUI app harness
    And the master chat already contains "keep this visible"
    When a resumed messages response arrives with a non-array messages field
    Then the app master session shows "keep this visible"
    And the app master session shows "Invalid resume payload: messages field is not an array"
    And the app notification includes "Invalid resume payload: messages field is not an array"

  Scenario: Failed model switch response is shown as an error notification
    Given a fresh TUI app harness
    When a model switch response fails with "model not found"
    Then the app notification includes "Model switch failed: model not found"

  @model-selector
  Scenario: Model selector opens from a fresh list and submits the filtered choice
    Given a fresh TUI app harness
    When I request the model selector
    And the model list response contains "openai-api/gpt-5.5" and "anthropic-api/claude-fable-5"
    And I filter the model selector with "fable"
    And I accept the selected model
    Then a set model command is sent for "anthropic-api/claude-fable-5"
    And the footer shows the master model "anthropic-api/claude-fable-5"

  @model-selector
  Scenario: Failed model list still opens the selector with cached models
    Given a fresh TUI app harness
    When I request the model selector
    And the model list response fails with "registry unavailable"
    Then the app notification includes "Could not list models: registry unavailable"
    And the model selector is visible

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

  @workflow-bar
  Scenario: Workflow state renders current step context in the main pane
    Given a fresh TUI app harness at width 120
    When workflow state reports issue 1028 with step 2 "Add BDD coverage" in phase "red" out of 3
    Then the workflow bar shows "Step 2/3"
    And the workflow bar shows "RED"
    And the workflow bar shows "Add BDD coverage"
    And the workflow bar shows "#1028"
    And the bottom stack does not show workflow text "Step 2/3"

  @workflow-bar
  Scenario: Narrow workflow state stays inside the terminal
    Given a fresh TUI app harness at width 60
    When workflow state reports issue 1028 with step 1 "A very long workflow label that must be truncated by the TUI" in phase "green" out of 1
    Then every workflow frame row fits the terminal width
    And the workflow bar preserves left padding after the divider
