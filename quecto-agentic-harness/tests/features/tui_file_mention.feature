@tui
Feature: TUI @files mention autocomplete
  Typing `@` in the editor opens a popup of workspace files, fuzzy-filtered by
  the text after `@`; selecting one inserts the relative path.

  Background:
    Given a workspace with files "src/main.rs, src/lib.rs, README.md"

  Scenario: @ opens the file mention popup with all files
    When the user types "@" in the editor
    Then the file mention popup is active
    And the file mention popup lists "@src/main.rs"

  Scenario: typing after @ fuzzy-filters the files
    When the user types "@main" in the editor
    Then the file mention popup is active
    And the file mention popup lists "@src/main.rs"

  Scenario: a space ends the @token and dismisses the popup
    When the user types "@src done" in the editor
    Then the file mention popup is not active

  Scenario: plain text without @ does not open the popup
    When the user types "hello world" in the editor
    Then the file mention popup is not active

  Scenario: accepting a suggestion selects the file path
    When the user types "@main" in the editor
    And the user accepts the file mention
    Then the selected file is "src/main.rs"

  @issue-735
  Scenario: first @ activation shows loading while files load in the background
    Given workspace file loading is pending
    When the user types "@" in the editor
    Then the file mention popup is active
    And the file mention popup lists "loading files"

  @issue-735
  Scenario: loaded workspace files replace the loading state
    Given workspace file loading is pending
    When the user types "@" in the editor
    And workspace files finish loading "src/main.rs, src/lib.rs, README.md"
    Then the file mention popup is active
    And the file mention popup lists "@src/main.rs"

  @issue-735
  Scenario: stale workspace files request a non-blocking reload
    Given workspace files were loaded more than 30 seconds ago
    When the user types "@" in the editor
    Then workspace file loading should be requested
    And the file mention popup is active
