@wip
Feature: Auto-notify parent LLM when subagents complete or error
  As a parent agent
  I want automatic follow-up messages when child agents finish or fail
  So that I don't have to manually poll subagent status

  # --- SubagentNotification enum ---

  Scenario: Completed notification includes agent_id and summary
    Given a Completed notification for agent "researcher" with summary "All tests pass"
    Then the notification [message] should contain "researcher"
    And the notification [message] should contain "completed"
    And the notification [message] should contain "All tests pass"

  Scenario: Errored notification includes agent_id and error
    Given an Errored notification for agent "linter" with error "rate limit exceeded"
    Then the notification [message] should contain "linter"
    And the notification [message] should contain "errored"
    And the notification [message] should contain "rate limit exceeded"

  Scenario: Exited notification includes agent_id
    Given an Exited notification for agent "formatter"
    Then the notification [message] should contain "formatter"
    And the notification [message] should contain "exited"

  # --- Notification formatting ---

  Scenario: Completed notification format is bracketed subagent event
    Given a Completed notification for agent "worker-1" with summary "Done building"
    Then the notification message should start with "[subagent]"

  Scenario: Errored notification format is bracketed subagent event
    Given an Errored notification for agent "worker-1" with error "timeout"
    Then the notification message should start with "[subagent]"

  Scenario: Exited notification format is bracketed subagent event
    Given an Exited notification for agent "worker-1"
    Then the notification message should start with "[subagent]"

  # --- Summary extraction ---

  Scenario: Extract summary from agent_end messages array
    Given an agent_end event with messages containing assistant text "The analysis is complete"
    When I extract the summary
    Then the extracted summary should be "The analysis is complete"

  Scenario: Extract summary truncates long text to 200 chars
    Given an agent_end event with assistant text of 300 characters
    When I extract the summary
    Then the extracted summary should be at most 203 characters

  Scenario: Extract summary from empty messages returns default
    Given an agent_end event with empty messages array
    When I extract the summary
    Then the extracted summary should be "(no output)"

  Scenario: Extract summary from messages with no assistant text returns default
    Given an agent_end event with only tool messages
    When I extract the summary
    Then the extracted summary should be "(no output)"

  # --- Channel behavior ---

  Scenario: Notification channel is bounded
    Given a SubagentNotification channel with capacity 64
    Then sending 64 notifications should succeed
    And the channel should not block on bounded sends

  Scenario: Notification receiver drains all pending
    Given a SubagentNotification channel with 3 pending notifications
    When I drain all notifications
    Then I should receive 3 notifications

  # --- Monitor sends notifications ---

  Scenario: Monitor sends Completed on agent_end
    Given a monitor with notification sender
    When the monitor processes an agent_end event with messages
    Then a Completed notification should be sent

  Scenario: Monitor sends Errored on tool_execution_end with is_error
    Given a monitor with notification sender
    When the monitor processes a tool_execution_end event with is_error true
    Then an Errored notification should be sent

  Scenario: Monitor sends Exited on connection close
    Given a monitor with notification sender
    When the monitor detects connection closed for agent "crashed-bot"
    Then an Exited notification should be sent for "crashed-bot"

  Scenario: Monitor does not send notification on agent_start
    Given a monitor with notification sender
    When the monitor processes an agent_start event
    Then no notification should be sent
