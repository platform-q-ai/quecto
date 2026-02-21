Feature: E2E Real LLM REPL
  End-to-end REPL tests against a real OpenAI endpoint.
  These scenarios are gated by @real-llm and excluded from normal CI runs.

  Background:
    Given a real LLM workspace is configured

  @done @real-llm @real-llm-smoke
  Scenario: Real LLM responds in REPL mode
    When I start quecto in REPL mode
    And I type "Reply with exactly REPL_PONG"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_PONG"

  @done @real-llm @real-llm-smoke
  Scenario: Real LLM uses tools from REPL mode
    When I start quecto in REPL mode
    And I type "Create a file named repl-note.txt containing exactly REPL_TOOL_OK"
    And I type "/exit"
    Then the exit code should be 0
    And the file "repl-note.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: Real LLM supports multi-turn memory in one REPL run
    When I start quecto in REPL mode
    And I type "Remember this code word: ember-77. Reply ACK_EMBER"
    And I type "What code word did I give you? Reply with just the code word."
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "ember-77"

  @done @real-llm
  Scenario: Real LLM remembers across named REPL sessions
    When I start quecto in REPL mode with flags "-s realrepl"
    And I type "My favorite token is mango-9081. Reply ACK_MANGO"
    And I type "/exit"
    Then the exit code should be 0
    When I start quecto in REPL mode with flags "-s realrepl"
    And I type "What is my favorite token? Reply with just the token."
    And I type "/exit"
    Then the exit code should be 0
    And stdout should not be empty

  @done @real-llm @real-llm-smoke
  Scenario: Skills influence REPL behavior with real LLM
    Given a workspace skill "repl-format" with frontmatter:
      """
      ---
      name: repl-format
      description: REPL formatting skill
      ---
      Include the token REPL_SKILL_OK in every response.
      """
    When I start quecto in REPL mode
    And I type "Say hello"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_SKILL_OK"
