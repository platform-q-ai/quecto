@tui @done
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
    When each TurnEnd event includes current active conversation size
    Then the footer should reflect the latest active conversation size, not cumulative provider input

  Scenario: Provider token usage drives the context gauge
    Given a completed agent turn reports provider input usage above the configured context window
    And the active pruned conversation estimate remains below the configured context window
    When the agent emits TurnEnd and session stats for the TUI
    Then contextTokens should equal the provider-reported context occupancy
    And maxContextTokens should equal the configured context window
    And the provider token usage should drive both the context gauge and usage totals

  Scenario: Streaming provider token usage drives the context gauge
    Given a completed streamed agent turn reports provider input usage above the configured context window
    And the active pruned conversation estimate remains below the configured context window
    When the agent emits TurnEnd and session stats for the TUI
    Then contextTokens should equal the provider-reported context occupancy
    And maxContextTokens should equal the configured context window
    And the provider token usage should drive both the context gauge and usage totals

  Scenario: Session stats accumulate provider token usage and cost
    Given multiple LLM calls return input, output, cache, and cost usage
    When the TUI requests session stats
    Then the stats response should include non-zero token totals and normalized cost
    And the TUI should display those token totals with normalized cost

  Scenario: High usage shows warning color
    Given context usage exceeds 70%
    When the footer renders
    Then the usage should be displayed in warning color

  Scenario: Critical usage shows error color
    Given context usage exceeds 90%
    When the footer renders
    Then the usage should be displayed in error color
