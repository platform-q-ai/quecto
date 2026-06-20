@tui
Feature: TUI context size and usage percentage display
  As a TUI user
  I want to see context usage percentage in the footer
  So that I know how close I am to the context window limit

  Scenario: Token usage displayed after agent response
    Given the agent completes a response using 5000 input tokens
    And the context window is 200k tokens
    When the TurnEnd event includes usage data
    Then the footer should show "2.5%/200k"

  Scenario: Usage updates after each turn
    Given the agent has processed multiple turns
    When each TurnEnd event includes current context usage
    Then the footer should reflect the latest context usage percentage, not cumulative input

  Scenario: Agent result records provider prompt tokens as current context size
    Given a completed non-streaming agent turn has provider prompt-token usage
    When the agent finalizes the response
    Then the result context tokens should equal the provider prompt tokens

  Scenario: Streaming agent result records provider prompt tokens as current context size
    Given a completed streaming agent turn has provider prompt-token usage
    When the agent finalizes the streamed response
    Then the result context tokens should equal the provider prompt tokens

  Scenario: Session stats accumulate provider usage and cost
    Given multiple LLM calls return input, output, cache, and cost usage
    When the TUI requests session stats
    Then the stats response should include non-zero token totals and cost
    And the TUI should display those token totals and cost instead of zeros

  Scenario: High usage shows warning color
    Given context usage exceeds 70%
    When the footer renders
    Then the usage should be displayed in warning color

  Scenario: Critical usage shows error color
    Given context usage exceeds 90%
    When the footer renders
    Then the usage should be displayed in error color
