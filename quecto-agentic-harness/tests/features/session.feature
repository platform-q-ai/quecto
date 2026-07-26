@done
Feature: Session Management
  As a user
  I want my conversations to be persistent
  So that the agent remembers context across messages

  Scenario: Empty new chats are not saved
    Given a [session] workspace
    And no [session] exists for key "telegram:12345"
    When the [session] store creates a session for key "telegram:12345"
    Then no [session] should exist for key "telegram:12345"
    When the [session] store loads session "telegram:12345"
    Then the [session] should not be found

  Scenario: Existing session is resumed
    Given a [session] workspace
    And a [session] "telegram:12345" with 3 messages in history
    When the [session] store loads session "telegram:12345"
    Then the [session] should be found
    And the conversation history should contain 3 messages

  Scenario: Sessions persist to disk
    Given a [session] workspace
    And a [session] "telegram:12345" with 3 messages in history
    When the [session] is saved to disk
    And the [session] store is recreated from the same directory
    And the [session] store loads session "telegram:12345"
    Then the [session] should be found
    And the conversation history should contain 3 messages

  @session @issue-987
  Scenario: Completed turns preserve previously saved session data
    Given a [session] workspace
    And a [session] "cli:efficient" with 2 messages in history
    When the [session] "cli:efficient" records a completed turn
    Then the [session] "cli:efficient" should reload with 4 messages
    And the [session] "cli:efficient" storage should preserve the previously saved data

  @session @issue-987
  Scenario: Interrupted turns preserve the last completed session
    Given a [session] workspace
    And a [session] "cli:durable" with 2 messages in history
    When the [session] "cli:durable" has an interrupted turn
    Then the [session] "cli:durable" should reload with 2 messages

  @session @issue-987
  Scenario: Replaced histories remain loadable
    Given a [session] workspace
    And a [session] "cli:replace" with 6 messages in history
    When the [session] "cli:replace" keeps only the latest 2 messages
    Then the [session] "cli:replace" should reload with 2 messages
    And the [session] "cli:replace" storage should replace the previous data

  @session @issue-987
  Scenario: Stored messages retain their conversation content after appended turns
    Given a [session] workspace
    And a [session] "cli:faithful" with distinct conversation content
    When the [session] "cli:faithful" records a completed turn
    And the [session] store is recreated from the same directory
    Then the [session] "cli:faithful" should reload with the same conversation content

  @session
  Scenario: Corrupt session files do not block session listing
    Given a [session] workspace
    And a [session] "cli:good" with 2 messages in history
    And a corrupt [session] file "cli_bad.json"
    Then the [session] list should include session "cli:good"

  @session
  Scenario: Sessions with unrecognised message detail fields still appear in the listing (#765)
    # Listing summarises sessions from their first user message and message
    # count; it must tolerate per-message details the full parser would reject,
    # since those details are never needed to build a summary.
    Given a [session] workspace
    And a session "chat-heavy" whose assistant message carries an unrecognised detail field
    Then the [session] list should include session "chat-heavy"
    And the [session] list entry "chat-heavy" should have title "what is the answer"
    And the [session] list entry "chat-heavy" should report 2 messages

  # NOTE: CLI session key scenarios moved to e2e_session.feature
  # because they now require a mock LLM server (cmd_agent is no longer a stub).

  Scenario: CLI agent without message flag rejects with usage error
    When I run quecto with arguments "agent"
    Then the exit code should be 1
    And the stderr should contain "agent: -m is required"

  Scenario: CLI agent with session flag requires config and provider
    Given a quecto base directory at a temporary path
    When I run quecto with arguments "agent -s my-session -m Hello"
    Then the exit code should be 1
    And the stderr should contain "no LLM providers"

  Scenario: Session routing by channel and chat ID
    Given a [session] workspace
    When user "111" sends a [message] on channel "telegram"
    And user "222" sends a [message] on channel "telegram"
    Then user "111" should have [session] key "telegram:111"
    And user "222" should have [session] key "telegram:222"
