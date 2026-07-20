@done @tui
Feature: TUI app event routing and command behaviours
  As a TUI operator
  I want responses, rewind actions, submitted prompts, and sub-agent streams to route to the right visible session
  So that the interface stays consistent with the conversation I am controlling

  Scenario: Successful session stats response updates the footer without a chat notification
    Given a fresh TUI app harness
    When a quiet session stats footer response arrives with cost "$0.1234" and context "42k"
    Then the footer shows context "42k" without cost "$0.1234"
    And the chat transcript does not show a session stats notification

  @app-response @session-payloads
  Scenario: Interactive session stats response adds a chat summary and footer values
    Given a fresh TUI app harness
    When an interactive session stats response arrives for "cli:demo" with cost "$0.5000" and tokens 123 input 456 output
    Then the footer shows context "8.0k" without cost "$0.5000"
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

  @model-selector
  Scenario: A selected sub-agent receives the chosen model
    Given a TUI viewing sub-agent "a1"
    And sub-agent "a1" uses model "openai-api/gpt-5.5" with effort "medium"
    When I choose model "anthropic-api/claude-fable-5" from the model selector
    Then sub-agent "a1" receives model "anthropic-api/claude-fable-5"
    And no set model command is sent to the master

  @model-selector
  Scenario: Explicit /model while a sub-agent is focused targets that sub-agent
    Given a TUI viewing sub-agent "a1"
    And sub-agent "a1" uses model "openai-api/gpt-5.5" with effort "medium"
    When I submit the master prompt "/model anthropic-api/claude-fable-5"
    Then sub-agent "a1" receives model "anthropic-api/claude-fable-5"
    And no set model command is sent to the master

  @model-selector
  Scenario: /model without a focused sub-agent still targets the master
    Given a fresh TUI app harness
    When I submit the master prompt "/model openai-api/gpt-5.5"
    Then a set model command is sent for "openai-api/gpt-5.5"
    And the footer shows the master model "openai-api/gpt-5.5"

  @model-selector
  Scenario: Successful sub-agent model switch updates only that session footer
    Given a TUI viewing sub-agent "a1"
    And the master uses model "openai-api/gpt-5.5"
    And sub-agent "a1" uses model "anthropic-api/claude-sonnet-4-6" with effort "high"
    And I have submitted the master prompt "/model anthropic-api/claude-fable-5"
    When sub-agent "a1" acknowledges and reports model "anthropic-api/claude-fable-5"
    Then the footer shows the sub-agent model "anthropic-api/claude-fable-5"
    And the master session still shows model "openai-api/gpt-5.5"

  @model-selector
  Scenario: Late master model switch does not clobber a focused sub-agent footer
    Given a TUI viewing sub-agent "a1"
    And the master uses model "openai-api/gpt-5.5"
    And sub-agent "a1" uses model "anthropic-api/claude-sonnet-4-6" with effort "high"
    When a master model switch succeeds for "openai-api/gpt-5.4"
    Then the footer shows the sub-agent model "anthropic-api/claude-sonnet-4-6"
    And the app notification does not include "Model switched"

  @model-selector
  Scenario: Model change is refused when the focused sub-agent connection is not ready
    Given a TUI viewing sub-agent "a1" without a ready connection
    When I submit the master prompt "/model anthropic-api/claude-fable-5" expecting no agent command
    Then the app notification includes "Selected sub-agent is not ready for model changes yet"
    And no set model command is sent

  @effort
  Scenario: Footer shows the active effort level from agent state
    Given a fresh TUI app harness
    When a get_state response arrives with model "openai-api/gpt-5.5" and effort "medium"
    Then the footer shows effort level "medium"

  @effort
  Scenario: Footer shows the effective default effort when the agent reports none set
    Given a fresh TUI app harness
    When a get_state response arrives with model "openai-api/gpt-5.5" and a null effort
    Then the footer shows the effective default effort

  @effort
  Scenario: Explicit /effort level sends set_effort
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    When I submit the master prompt "/effort high"
    Then a set effort command is sent for "high"

  @effort
  Scenario: Successful effort switch updates the footer
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    And I have submitted the master prompt "/effort high"
    When the set effort response succeeds with effort "high"
    Then the footer shows effort level "high"

  @effort
  Scenario: Invalid /effort level is rejected and the previous setting stays
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    When I submit the master prompt "/effort turbo" expecting no agent command
    Then the app reports an invalid effort level listing "none, low, medium, high, xhigh"
    And no set effort command is sent
    And the footer shows effort level "medium"

  @effort
  Scenario: An effort level from another provider's vocabulary is rejected
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    When I submit the master prompt "/effort max" expecting no agent command
    Then the app reports an invalid effort level listing "none, low, medium, high, xhigh"
    And no set effort command is sent
    And the footer shows effort level "medium"

  @effort @effort-selector
  Scenario: /effort opens a selector with the OpenAI effort vocabulary
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    When I open the effort selector via the /effort prompt
    Then the effort selector is visible
    And the effort selector lists exactly "none, low, medium, high, xhigh"

  @effort @effort-selector
  Scenario: /effort selector for an Anthropic model lists the Anthropic vocabulary
    Given a fresh TUI app harness
    And the agent reports model "anthropic-api/claude-fable-5" with effort "high"
    When I open the effort selector via the /effort prompt
    Then the effort selector is visible
    And the effort selector lists exactly "low, medium, high, max"

  @effort @effort-selector
  Scenario: Accepting an effort selector entry sends set_effort
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    And I have opened the effort selector via the /effort prompt
    When I filter the effort selector with "xh"
    And I accept the selected effort
    Then a set effort command is sent for "xhigh"

  @effort @effort-selector
  Scenario: A selected sub-agent receives the chosen effort level
    Given a TUI viewing sub-agent "a1"
    And sub-agent "a1" uses model "openai-api/gpt-5.5" with effort "medium"
    When I choose effort "high" from the effort selector
    Then sub-agent "a1" receives effort "high"
    And no set effort command is sent to the master

  @effort @effort-selector
  Scenario: An effort unsupported by the selected sub-agent is rejected
    Given a TUI viewing sub-agent "a1"
    And sub-agent "a1" uses model "anthropic-api/claude-fable-5" with effort "medium"
    When I request effort "xhigh" for the selected sub-agent
    Then the app reports invalid effort "xhigh" with supported levels "low, medium, high, max"
    And no set effort command is sent

  @effort
  Scenario: Failed effort switch is notified and the footer keeps the previous level
    Given a fresh TUI app harness
    And the agent reports model "openai-api/gpt-5.5" with effort "medium"
    And I have submitted the master prompt "/effort high"
    When the set effort response fails with "agent busy"
    Then the app notification includes "Effort switch failed: agent busy"
    And the footer shows effort level "medium"

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

  Scenario: Master submit while streaming queues a follow-up
    Given a fresh TUI app harness
    And the master assistant is currently streaming
    When I submit the master prompt "add more detail"
    Then the master follow-up command is sent with message "add more detail"
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

  # ── #1050: main chat history on --socket attach ───────────────────
  # Sub-agent panes already backfill on connect-on-select (#828). The master
  # chat must do the same when the TUI attaches to a running agent: prior
  # durable history appears without waiting for new events, live tokens that
  # race ahead of the backfill are preserved (prepend, never wholesale
  # replace), empty payloads do not latch the guard, and re-delivery does not
  # duplicate. Resume/rewind keep their existing get_messages paths.

  @attach-backfill
  Scenario: Attaching to a running agent shows prior master history
    Given a TUI attached to a running agent
    When the master backfill history "prior question" then "prior answer" arrives
    Then the app master session shows "prior question"
    And the app master session shows "prior answer"
    And "prior question" appears above "prior answer" in the master session
    And the app master session does not show "Session resumed"

  @attach-backfill
  Scenario: Master history backfill preserves live content already streamed
    Given a TUI attached to a running agent
    And the master has already streamed the live token "LIVENOW"
    When the master backfill history "earlier question" then "earlier answer" arrives
    Then the app master session shows "earlier question"
    And the app master session shows "earlier answer"
    And the app master session still shows "LIVENOW"
    And "earlier answer" appears above "LIVENOW" in the master session
    And the app master session does not show "Session resumed"

  @attach-backfill
  Scenario: Re-delivered master history does not duplicate chat entries
    Given a TUI attached to a running agent
    And the master backfill history "the question" then "the answer" has already arrived
    When the same master backfill history arrives again
    Then the app master session shows "the answer" exactly once
    And the app master session does not show "Session resumed"

  @attach-backfill
  Scenario: Empty master history does not suppress a later populated history
    Given a TUI attached to a running agent
    And an empty master backfill history has already arrived
    When the master backfill history "real question" then "real answer" arrives
    Then the app master session shows "real question"
    And the app master session shows "real answer"
    And the app master session does not show "Session resumed"
