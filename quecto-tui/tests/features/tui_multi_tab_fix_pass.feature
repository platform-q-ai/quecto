@tui @done @multi-tab-fix-pass
Feature: Removed multi-session TUI tab/workspace affordances (#1596)
  As a TUI user
  I no longer see or activate tab/workspace UI affordances
  So that the TUI behaves as a single-session interface

  Scenario: Clicking the old new-tab location does not open a tab
    Given a TUI with a second background tab
    When the user clicks the new-tab button
    Then still only two tabs are open

  Scenario: Clicking old tab-bar dead space changes nothing
    Given a TUI with a second background tab
    When the user clicks past the end of the tab bar
    Then the master tab remains active

  Scenario: Ctrl+PageUp no longer cycles tabs
    Given a TUI with two background tabs
    When the user presses Ctrl+PageUp
    Then the master tab remains active
