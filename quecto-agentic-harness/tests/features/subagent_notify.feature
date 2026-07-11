@done
Feature: Auto-notify a parent when a sub-agent ends a turn
  As a parent agent
  I want turn-end notifications that do not imply successful task completion
  So that I inspect the child output before treating its work as complete

  # --- SubagentNotification enum ---

  Scenario: A turn-end notification distinguishes the event from task success
    Given a Completed notification for agent "researcher" with summary "All tests pass"
    Then the notification [message] should contain "researcher"
    And the notification [message] should contain "ended a turn"
    And the notification [message] should contain "status: idle"
    And the notification [message] should contain "get_messages"
    And the notification [message] should contain "before treating its work as complete"

  Scenario: Errored notification includes agent_id and error
    Given an Errored notification for agent "linter" with error "rate limit exceeded"
    Then the notification [message] should contain "linter"
    And the notification [message] should contain "failed"
    And the notification [message] should contain "rate limit exceeded"

  Scenario: Exited notification includes agent_id
    Given an Exited notification for agent "formatter"
    Then the notification [message] should contain "formatter"
    And the notification [message] should contain "exited"

  # --- Notification formatting ---

  Scenario: Completed notification format names the agent
    Given a Completed notification for agent "worker-1" with summary "Done building"
    Then the notification message should start with "Sub-agent 'worker-1'"

  Scenario: Errored notification format names the agent
    Given an Errored notification for agent "worker-1" with error "timeout"
    Then the notification message should start with "Agent 'worker-1'"

  Scenario: Exited notification format names the agent
    Given an Exited notification for agent "worker-1"
    Then the notification message should start with "Agent 'worker-1'"

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

  # --- #816: auto-await — completion notes surface only at the parent's idle boundary ---

  Scenario: A completed child auto-delivers one idle note with no manual await
    Given a parent session with no pending notes
    When subagent "researcher" reports completion with note "researcher complete: all tests pass"
    Then the parent should have 1 pending subagent note
    And the parent's next idle note should be delivered on the operator channel
    And the parent's next idle note should be a single line
    And the parent's next idle note should contain "researcher complete"

  Scenario: A note arriving while the parent is busy waits until the parent is idle
    Given a parent session with no pending notes
    And the parent is busy processing a turn
    When subagent "researcher" reports completion with note "researcher complete"
    Then the busy parent should not have consumed the note yet
    And the parent's next idle note should contain "researcher complete"

  Scenario: The same completion reported twice is delivered only once
    Given a parent session with no pending notes
    When subagent "researcher" reports completion with note "done"
    And subagent "researcher" reports the same completion again
    Then the second report should be ignored
    And the parent should have 1 pending subagent note

  Scenario: A newer completion from one child replaces its earlier pending note
    Given a parent session with no pending notes
    When subagent "worker" reports completion with note "first"
    And subagent "worker" reports a newer completion with note "worker complete: latest"
    Then the parent should have 1 pending subagent note
    And the parent's next idle note should contain "latest"

  Scenario: An errored child still produces a one-line failure note
    Given a parent session with no pending notes
    When subagent "linter" reports completion with note "linter failed: rate limit exceeded"
    Then the parent should have 1 pending subagent note
    And the parent's next idle note should contain "failed"

  Scenario: A spawned child's turn end reaches the parent with no manual await
    Given a parent session with no pending notes
    And a monitor with notification sender
    When the monitor processes an agent_end event with messages
    And the parent drains its subagent notifications
    Then the parent should have 1 pending subagent note
    And the parent's next idle note should be delivered on the operator channel
    And the parent's next idle note should contain "ended a turn"
