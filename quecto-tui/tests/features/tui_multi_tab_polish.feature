@tui @done @multi-tab-polish
Feature: Removed multi-session TUI background-tab affordances (#1596)
  As a TUI user
  I no longer use tab UI or workspace resume rows
  So that old multi-tab affordances stay inert

  Scenario: Switching no longer exposes an unread-dot affordance
    Given a TUI with a second background tab
    And 1 tokens already streamed to the background tab
    When the user switches to the background tab
    Then the background tab remains marked unread

  Scenario: Orphaned workspaces are not offered for resume
    Given a durable workspace labelled "Auth spike"
    When the resume selector opens with workspaces
    Then the resume selector is not open

  Scenario: Ctrl+Tab no longer switches to the next tab
    Given a TUI with a second background tab
    When the user presses Ctrl+Tab
    Then the master tab remains active
