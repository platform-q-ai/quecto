@done
Feature: Session Management
  As a user
  I want my conversations to be persistent
  So that the agent remembers context across messages

  Scenario: New session is created for a new chat
    Given a session workspace
    And no session exists for key "telegram:12345"
    When the session store creates a session for key "telegram:12345"
    Then a session should exist for key "telegram:12345"
    When the session store loads session "telegram:12345"
    Then the session should be found
    And the conversation history should contain 0 messages

  Scenario: Existing session is resumed
    Given a session workspace
    And a session "telegram:12345" with 3 messages in history
    When the session store loads session "telegram:12345"
    Then the session should be found
    And the conversation history should contain 3 messages

  Scenario: Sessions persist to disk
    Given a session workspace
    And a session "telegram:12345" with 3 messages in history
    When the session is saved to disk
    And the session store is recreated from the same directory
    And the session store loads session "telegram:12345"
    Then the session should be found
    And the conversation history should contain 3 messages

  Scenario: CLI agent uses default session key
    When I run quecto with arguments "agent"
    Then the output should contain "session: cli:default"

  Scenario: Custom session key via CLI flag
    When I run quecto with arguments "agent -s my-session -m Hello"
    Then the output should contain "session: cli:my-session"

  Scenario: Long-term memory stored in MEMORY.md
    Given a session workspace
    When the agent writes a memory note "User prefers concise answers"
    Then the file "memory/MEMORY.md" should exist in the session workspace
    And the memory file should contain "User prefers concise answers"

  Scenario: Agent identity loaded from workspace
    Given a session workspace
    And the workspace file "IDENTITY.md" contains "You are Quecto, a helpful assistant"
    When the agent loads identity from the workspace
    Then the identity should include "You are Quecto, a helpful assistant"

  Scenario: Session routing by channel and chat ID
    Given a session workspace
    When user "111" sends a message on channel "telegram"
    And user "222" sends a message on channel "telegram"
    Then user "111" should have session key "telegram:111"
    And user "222" should have session key "telegram:222"
