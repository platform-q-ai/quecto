@tui @done @multi-tab-polish
Feature: Multi-session TUI background-tab quality and polish (#1466)
  As a TUI user running several tab agents at once
  I want background tabs to stay quiet, bounded, and clearly signposted
  So that N idle tabs cost nothing and activity is never missed

  Scenario: Background-tab tokens do not schedule paints
    Given a TUI with a second background tab
    When 20 tokens stream to the background tab
    Then no frame is painted for the background stream
    And no frame is painted even after the render loop settles

  Scenario: Streamed background output marks the tab unread
    Given a TUI with a second background tab
    When 20 tokens stream to the background tab
    Then the background tab is marked unread

  Scenario: Switching to an unread tab repaints once and clears the unread dot
    Given a TUI with a second background tab
    And 20 tokens already streamed to the background tab
    When the user switches to the background tab
    Then exactly one frame is painted for the switch
    And the background tab is no longer marked unread

  Scenario: An ended background turn sets the unread dot and clears the spinner
    Given a TUI with a second background tab
    And the background tab has a running turn
    When the background turn ends
    Then the background tab shows no spinner
    And the background tab is marked unread

  Scenario: A running background turn shows the tab spinner
    Given a TUI with a second background tab
    When a turn starts on the background tab
    Then the background tab shows a spinner

  Scenario: A running background turn keeps animation ticking
    Given a TUI with a second background tab
    When a turn starts on the background tab
    Then the TUI still requests animation ticks while only a background tab is busy

  Scenario: Retained sub-agent sessions are capped at 30 per tab
    Given a TUI with a second background tab
    And 3 sub-agent sessions already started on the background tab
    When 31 sub-agent sessions start on the active tab
    Then the active tab retains exactly 30 sessions
    And the background tab still retains exactly 3 sessions

  Scenario: Workspaces resume by label and last-active time
    Given a durable workspace labelled "Auth spike"
    When the resume selector opens with workspaces
    Then the workspace row shows the label "Auth spike"
    And the workspace row does not show the raw workspace id

  Scenario: Orphaned workspaces are garbage-collected out of resume
    Given a durable workspace with no resumable sessions
    When workspace garbage collection runs
    And the resume selector opens with workspaces
    Then the orphaned workspace is not offered for resume

  Scenario: The kitty Ctrl+Tab alias matches the Alt tab-switch primary
    Given a TUI with a second background tab
    When the kitty Ctrl+Tab sequence is pressed
    Then the background tab becomes the active tab
