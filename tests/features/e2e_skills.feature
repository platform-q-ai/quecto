@done
Feature: E2E Skill Loading
  As a user
  I want skills installed on disk to influence the agent's behavior
  So that I can extend and customize what the agent knows and does

  Skills use YAML frontmatter in SKILL.md files. The body (after the
  closing ---) is injected into the agent's system prompt.

  Background:
    Given a temp base directory

  # --- CLI skill management ---

  Scenario: Skills list shows installed workspace skills
    Given a workspace skill "weather" with frontmatter:
      """
      ---
      name: weather
      description: Fetch weather forecasts for any city
      ---
      Fetch weather forecasts for any city
      """
    When I run quecto with arguments "skills list"
    Then the exit code should be 0
    And the output should contain "weather"
    And the output should contain "Fetch weather forecasts for any city"

  Scenario: Skills list shows no skills when workspace is empty
    When I run quecto with arguments "skills list"
    Then the exit code should be 0
    And the output should contain "No skills installed"

  Scenario: Skills remove deletes a workspace skill
    Given a workspace skill "weather" with frontmatter:
      """
      ---
      name: weather
      description: Weather forecasts
      ---
      Weather skill body
      """
    When I run quecto with arguments "skills remove weather"
    Then the exit code should be 0
    And the output should contain "removed successfully"
    When I run quecto with arguments "skills list"
    Then the output should contain "No skills installed"

  Scenario: Skills remove nonexistent skill returns error
    When I run quecto with arguments "skills remove ghost"
    Then the exit code should be 1
    And the stderr should contain "not found"

  # --- Skill content injection into agent ---

  Scenario: Skill body is prepended to agent system prompt
    Given a workspace skill "code-review" with frontmatter:
      """
      ---
      name: code-review
      description: Review code for quality
      ---
      You are a code review expert. Always suggest improvements.
      """
    And a mock LLM that captures requests and returns text "I will review your code"
    When I run quecto agent -s - -m "Review my code"
    Then the exit code should be 0
    And the output should contain "I will review your code"
    And the LLM should have received a system message containing "code review expert"

  Scenario: Multiple skills are concatenated into system prompt
    Given a workspace skill "code-review" with frontmatter:
      """
      ---
      name: code-review
      description: Review code for quality
      ---
      You are a code review expert.
      """
    And a workspace skill "testing" with frontmatter:
      """
      ---
      name: testing
      description: Testing specialist
      ---
      You are a testing specialist.
      """
    And a mock LLM that captures requests and returns text "I can help with both"
    When I run quecto agent -s - -m "Help me"
    Then the exit code should be 0
    And the LLM should have received a system message containing "code review expert"
    And the LLM should have received a system message containing "testing specialist"


  Scenario: Agent works normally with no skills installed
    Given a mock LLM that captures requests and returns text "Hello from the agent"
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 0
    And the output should contain "Hello from the agent"
    And the LLM system message should only contain the datetime preamble


  Scenario: Skill without valid frontmatter does not add system prompt
    Given a workspace skill directory "bad-skill" with raw content "Just plain text"
    And a mock LLM that captures requests and returns text "Still works"
    When I run quecto agent -s - -m "Hello"
    Then the exit code should be 0
    And the output should contain "Still works"
    And the LLM system message should only contain the datetime preamble

  # Issue #104: The quecto datetime preamble is intentionally richer than
  # any provider-injected "Current date:" metadata. It includes day-of-week,
  # full time with seconds, and timezone — critical for cron scheduling
  # and time-aware tasks. Duplication with provider-side date is expected.
  Scenario: Datetime preamble includes day-of-week, time, and timezone
    Given a mock LLM that captures requests and returns text "I know the time"
    When I run quecto agent -s - -m "What time is it?"
    Then the exit code should be 0
    And the LLM system message datetime preamble should include day-of-week, time, and timezone
