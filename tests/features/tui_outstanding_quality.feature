@tui
Feature: TUI outstanding quality fixes (#729)
  As a TUI user
  I want expensive work and delivery failures to be visible without freezing the interface
  So that commands and file mentions behave predictably in real workspaces

  Scenario: @files autocomplete starts workspace enumeration off the UI thread
    Given the editor cursor is after an @files token
    And the workspace file cache is stale
    When the @files autocomplete is updated
    Then it should request a background workspace file load
    And the update should return immediately without running git or walking the filesystem on the UI thread
    And the popup should stay hidden or show cached results until the background load completes

  Scenario: @files autocomplete applies completed background loads
    Given a background workspace file load has completed with "src/main.rs" and "README.md"
    When the @files autocomplete receives the completed load
    And the editor cursor is after "@src"
    Then the popup should offer "src/main.rs"

  Scenario: Command send failures are surfaced to the user
    Given the TUI has queued a command to the agent
    When sending the command fails
    Then an error notification should appear explaining that the command was not sent
    And the failure should still be logged for diagnostics
