@tui @issue-997
Feature: TUI shared list rendering and grouped App state
  As a TUI maintainer
  I want the four list/overlay surfaces to render through one shared row helper
  and the App's rewind, sub-agent and model-registry state grouped by owner
  So that duplicated render code disappears with zero visual or behavior change

  # ── Characterization locks: today's pixels, before and after the refactor ──

  @wip
  Scenario: Slash dropdown windows the commands with an overflow indicator
    Given the editor text is "/"
    Then the slash dropdown windows the commands with the indicator "(1/12)"

  @wip
  Scenario: A stale file list reload keeps the selection marker
    Given a shared-list files popup loaded with a stale workspace file list
    When the shared-list files popup is opened with an at token
    Then a shared-list background reload is requested
    And the selected file row keeps its arrow marker undimmed

  @wip
  Scenario: The loading placeholder is dimmed and cannot be accepted
    Given a shared-list files popup with no loaded files
    When the shared-list files popup is opened with an at token
    Then the only file row is a dimmed loading placeholder
    And accepting the placeholder leaves the file result pending

  @wip
  Scenario: Model selector clamps the selection when the filter narrows
    Given a model selector over the known models
    When the model selection moves down 5 rows
    And the model filter "fireworks" is typed
    Then 2 models match and the selection is clamped to the last match

  @wip
  Scenario: The current model marker does not disturb the provider column
    Given a model selector whose current model has the longest id
    Then the current model row carries the marker after its id
    And the marked row's provider is offset by exactly the marker width

  # ── Shared-renderer contract: RED until the #997 helper exists ──

  @wip
  Scenario: The select list renders through the shared row helper
    Given the four list surfaces hold sample rows
    Then the shared row helper reproduces the select list rows exactly

  @wip
  Scenario: The slash dropdown renders through the shared row helper
    Given the four list surfaces hold sample rows
    Then the shared row helper reproduces the slash dropdown rows exactly

  @wip
  Scenario: The files dropdown renders through the shared row helper
    Given the four list surfaces hold sample rows
    Then the shared row helper reproduces the files dropdown rows exactly

  @wip
  Scenario: The model selector renders through the shared row helper
    Given the four list surfaces hold sample rows
    Then the shared row helper reproduces the model selector rows exactly

  # ── Grouped App state contract: RED until the owner structs exist ──

  @wip
  Scenario: Rewind flow state lives in one owner group
    Given a live TUI render harness
    When a rewind open request is issued by double Escape
    Then the rewind owner group reports request sequence 1

  @wip
  Scenario: The model registry is a named owner struct
    Given a live TUI render harness
    When a list_models response with 2 models arrives
    Then the model registry group holds 2 entries with no pending open

  @wip
  Scenario: Sub-agent UI state lives in one owner group
    Given a live TUI render harness
    When a subagents_changed push registers 1 agent
    Then the sub-agent owner group tracks 1 agent
