@tui @pending
Feature: TUI model selector overlay — scrollable list with fuzzy search
  Issue #474: /model currently requires typing the full model name.
  Add a scrollable overlay with fuzzy search, opened by /model (no args)
  or Ctrl+L.

  # ---------------------------------------------------------------------------
  # Model selector component
  # ---------------------------------------------------------------------------

  Scenario: Model selector renders list of models
    Given a model selector with known models
    When rendered at width 60
    Then the output should contain at least one model name

  Scenario: Model selector includes latest Anthropic models
    Given a model selector with known models
    When rendered at width 80
    Then the output should contain "anthropic/claude-fable-5"
    And the output should contain "anthropic/claude-opus-4-8"
    And the output should contain "anthropic/claude-opus-4-7"

  Scenario: Model selector shows selection indicator
    Given a model selector with known models
    When rendered at width 60
    Then the first item should have a selection indicator

  Scenario: Model selector navigates with Up/Down
    Given a model selector with known models
    When Down is pressed
    Then the second item should be selected

  Scenario: Model selector wraps navigation
    Given a model selector with known models at the last item
    When Down is pressed
    Then the first item should be selected

  Scenario: Model selector selects on Enter
    Given a model selector with known models
    When Enter is pressed
    Then the result should be Selected with the first model

  Scenario: Model selector cancels on Escape
    Given a model selector with known models
    When Escape is pressed
    Then the result should be Cancelled

  # ---------------------------------------------------------------------------
  # Fuzzy search integration
  # ---------------------------------------------------------------------------

  Scenario: Typing filters models by fuzzy match
    Given a model selector with known models
    When the user types "son"
    Then only models matching "son" should be visible

  Scenario: Empty query shows all models
    Given a model selector with known models
    When the query is empty
    Then all models should be visible

  Scenario: Query with no matches shows empty state
    Given a model selector with known models
    When the user types "zzzznonexistent"
    Then the output should show "No matching models"

  # ---------------------------------------------------------------------------
  # Current model highlighting
  # ---------------------------------------------------------------------------

  Scenario: Current model is marked in the list
    Given a model selector with current model "claude-sonnet-4-6"
    When rendered
    Then the current model should have a visual indicator

  # ---------------------------------------------------------------------------
  # Width compliance
  # ---------------------------------------------------------------------------

  Scenario: Model selector respects terminal width
    Given a model selector with known models
    When rendered at width 40
    Then no rendered line should exceed 40 visible characters
