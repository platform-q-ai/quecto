Feature: End-to-End Session Management
  As a user or parent process
  I want to control session selection when running the agent CLI
  So that conversations persist, stay isolated, or run statelessly as needed

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Named sessions ---

  @done
  Scenario: Default session is cli:default
    Given the mock LLM returns a text response "Hello"
    When I run quecto agent -m "Hi"
    Then the exit code should be 0
    And a [session] file should exist for key "cli:default"

  @done
  Scenario: Named session persists conversation history
    Given the mock LLM returns a text response "First response"
    When I run quecto agent -s chat1 -m "First message"
    Then the exit code should be 0
    And a [session] file should exist for key "cli:chat1"
    And the [session] "cli:chat1" should contain 2 conversation messages

  @done
  Scenario: Second message in same session includes history
    Given a pre-existing [session] "cli:chat1" with 2 messages
    And the mock LLM returns a text response "I remember"
    When I run quecto agent -s chat1 -m "Do you remember?"
    Then the exit code should be 0
    And the [session] "cli:chat1" should contain 4 conversation messages

  @done
  Scenario: Different session names are isolated
    Given the mock LLM returns a text response "Response A"
    When I run quecto agent -s session-a -m "Message A"
    Given the mock LLM returns a text response "Response B"
    When I run quecto agent -s session-b -m "Message B"
    Then a [session] file should exist for key "cli:session-a"
    And a [session] file should exist for key "cli:session-b"
    And the [session] "cli:session-a" should not contain text "Message B"
    And the [session] "cli:session-b" should not contain text "Message A"

  # --- Ephemeral sessions ---

  @done
  Scenario: Ephemeral session does not persist to disk
    Given the mock LLM returns a text response "Ephemeral reply"
    When I run quecto agent -s - -m "Throwaway question"
    Then the exit code should be 0
    And stdout should contain "Ephemeral reply"
    And no [session] files should exist

  @done
  Scenario: Ephemeral session does not load prior history
    Given a pre-existing [session] "cli:default" with 5 messages
    And the mock LLM returns a text response "Fresh start"
    When I run quecto agent -s - -m "New question"
    Then the exit code should be 0
    And stdout should contain "Fresh start"

  # --- Session with system prompt ---

  @done
  Scenario: System prompt is used but not persisted in session
    Given the mock LLM returns a text response "Aye aye"
    When I run quecto agent -s pirate --system "You are a pirate" -m "Hello"
    Then the exit code should be 0
    And the [session] "cli:pirate" should contain 2 conversation messages
    And the session "cli:pirate" should not include a system role message
