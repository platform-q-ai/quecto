@e2e
Feature: Initialization-directory AGENTS.md instructions
  Agent startup loads project instructions once from the exact directory where
  the agent is initialized, and composes them before explicit CLI instructions.

  @done
  Scenario: One-shot agent receives initialization instructions before the explicit system prompt
    Given a temp base directory
    And a mock LLM that captures requests and returns text "done"
    And a file "AGENTS.md" in the e2e workspace with content "ONE_SHOT_AGENTS_MARKER"
    And a file "../AGENTS.md" in the e2e workspace with content "PARENT_AGENTS_MARKER"
    When I run quecto agent --system "EXPLICIT_SYSTEM_MARKER" -m "hello"
    Then the exit code should be 0
    And the first LLM system message should contain "ONE_SHOT_AGENTS_MARKER" before "EXPLICIT_SYSTEM_MARKER"
    And the LLM should not have received a system message containing "PARENT_AGENTS_MARKER"
