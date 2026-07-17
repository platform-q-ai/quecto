@done @workflow @cache-safe-prompt
Feature: Workflow cache-safe prompting (#1113)
  As an operator running long workflow sessions
  I want the rendered system prompt to stay byte-identical across every turn,
  including across workflow tool calls
  So that the provider-side cached prefix survives every workflow step

  # PRD AC1: snapshot the system prompt across a select_template + check
  # sequence — every LLM request in the session must carry the same bytes.
  Scenario: Workflow check calls leave the system prompt byte-identical
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns tool calls for workflow select then check then text "done"
    When I start the multi-client UDS agent with workflow enabled and system prompt "You are the cache-safe workflow test agent."
    And client 1 connects
    And client 1 sends prompt "select feature and check step 1"
    And I close all UDS clients
    Then the UDS agent exits with code 0
    And every LLM request of the session should carry a byte-identical system prompt

  # PRD AC3, arming-to-dispatch integration: a model that starts WITHOUT
  # selecting a template must receive the selector through the first UDS idle
  # boundary — as an appended message that triggers a further LLM request —
  # never through injected system-prompt text.
  Scenario: The first idle boundary pushes the template selector to a session that did not select
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM replies "starting without selecting" without selecting, then selects template "feature", then replies "done"
    When I start the multi-client UDS agent with workflow enabled and system prompt "You are the cache-safe workflow test agent."
    And client 1 connects
    And client 1 sends prompt "begin the task"
    And I close all UDS clients
    Then the UDS agent exits with code 0
    And a nudged LLM request should carry the workflow template selector
    And every LLM request of the session should carry a byte-identical system prompt

  # The selector nudge is the sole proactive selection channel, so it must
  # not be gated on workflow.auto_continue: pre-#1113 the system-prompt
  # selector reached the model regardless of that setting.
  Scenario: The idle-boundary selector still fires with workflow auto-continue disabled
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config file disables workflow auto-continue
    And the mock LLM replies "starting without selecting" without selecting, then selects template "feature", then replies "done"
    When I start the multi-client UDS agent with workflow enabled and system prompt "You are the cache-safe workflow test agent."
    And client 1 connects
    And client 1 sends prompt "begin the task"
    And I close all UDS clients
    Then the UDS agent exits with code 0
    And a nudged LLM request should carry the workflow template selector

  # PRD AC5/AC6/AC7: inline workflow.templates configs keep working, and a
  # mocked model can still follow a workflow end-to-end — select a template,
  # advance every step via check, and complete — with a static system prompt.
  Scenario: A mocked model completes an inline-template workflow end-to-end with a static system prompt
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config file defines an inline three-step workflow template "inline"
    And the mock LLM selects template "inline", checks all three steps, then replies "workflow done"
    When I start the multi-client UDS agent with workflow enabled and system prompt "You are the cache-safe workflow test agent."
    And client 1 connects
    And client 1 sends prompt "run the inline workflow"
    And I close all UDS clients
    Then the UDS agent exits with code 0
    And client 1 should have received a workflow_state event with mode "complete"
    And every LLM request of the session should carry a byte-identical system prompt
