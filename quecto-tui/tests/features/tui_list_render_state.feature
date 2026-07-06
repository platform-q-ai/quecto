@tui @issue-997
Feature: TUI shared list rendering and grouped App state
  As a TUI maintainer
  I want the four list/overlay surfaces to render through one shared row helper
  and the App's rewind, sub-agent and model-registry state grouped by owner
  So that duplicated render code disappears with zero visual or behavior change

  # ── Characterization locks: today's pixels, before and after the refactor ──
  # Helper-vs-component render equivalence is pinned by the unit tests in
  # list_rows.rs and list_render_characterization_tests.rs; these scenarios
  # lock the user-visible behavior through the real components.

  @done
  Scenario: Slash dropdown windows the commands with an overflow indicator
    Given the editor text is "/"
    When the interface renders a frame
    Then the slash dropdown draws exactly the first 8 commands with the indicator "(1/12)"

  @done
  Scenario: A stale file list reload keeps the selection marker
    Given a files popup loaded with a stale workspace file list
    When the user types an at token
    Then a background reload is requested
    And the selected file row keeps its arrow marker undimmed

  @done
  Scenario: The loading placeholder is dimmed
    Given a files popup with no loaded files
    When the user types an at token
    Then the only file row is a dimmed loading placeholder

  @done
  Scenario: The loading placeholder cannot be accepted
    Given a files popup showing the loading placeholder
    When the user accepts the highlighted row
    Then no file is inserted and the popup stays open

  @done
  Scenario: Model selector clamps the selection when the filter narrows
    Given a model selector over the known models
    And the model selection rests on the 6th model
    When the model filter "fireworks" is typed
    Then 2 models match and the selection is clamped to the last match

  @done
  Scenario: The current model marker does not disturb the provider column
    Given a model selector whose current model has the longest id
    When the model selector renders
    Then the current model row carries the marker after its id
    And the marked row's provider is offset by exactly the marker width

  # ── Grouped App state: observed through the owner-group probes ──

  @done
  Scenario: Rewind flow state lives in one owner group
    Given a live TUI render harness
    When a rewind open request is issued by double Escape
    Then a rewind-open command is emitted
    And the rewind owner group reports request sequence 1

  @done
  Scenario: The model registry is a named owner struct
    Given a live TUI render harness
    And a model selector open has been requested
    When a list_models response with 2 models arrives
    Then the model registry group holds 2 entries and the pending open is cleared

  @done
  Scenario: Sub-agent UI state lives in one owner group
    Given a live TUI render harness
    When a subagents_changed push registers 1 agent
    Then the sub-agent owner group tracks 1 agent
