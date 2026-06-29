@tui @pending
Feature: TUI Phase 4 — Slash Commands, Autocomplete, and Model Switching
  Interactive commands, fuzzy autocomplete, and model management.

  Scenario: Fuzzy match finds substring matches
    Given fuzzy matcher items "settings", "model", "clear", "quit"
    When the query is "set"
    Then the results should include "settings"

  Scenario: Fuzzy match handles multi-token queries
    Given fuzzy matcher items "claude-sonnet-4", "gpt-4o", "claude-opus-4"
    When the query is "claude opus"
    Then the results should include "claude-opus-4"
    And the results should not include "gpt-4o"

  Scenario: Select list renders items with selection indicator
    Given a select list with items "Option A", "Option B", "Option C"
    When the select list renders at width 40
    Then the rendered output should contain a selection indicator
    And the rendered output should contain "Option A"

  Scenario: Select list navigates with Up/Down
    Given a select list with items "A", "B", "C"
    When the user presses Down
    Then item "B" should be selected

  Scenario: Autocomplete triggers on slash
    Given an autocomplete provider with commands "model", "clear", "quit"
    When the input text is "/mo"
    Then the autocomplete should suggest "model"

  Scenario: Autocomplete dismisses on Escape
    Given an active autocomplete dropdown
    When the user presses Escape
    Then the autocomplete should be dismissed

  Scenario: Model selector shows available models
    Given a model selector with models "claude-sonnet-4", "gpt-4o"
    When the model selector renders at width 60
    Then the rendered output should contain "claude-sonnet-4"
    And the rendered output should contain "gpt-4o"
