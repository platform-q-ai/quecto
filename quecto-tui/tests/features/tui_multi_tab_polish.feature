@tui @done @multi-tab-polish
Feature: Multi-session TUI background-tab quality and polish (#1466)
  As a TUI user running several tab agents at once
  I want background tabs to stay quiet, bounded, and clearly signposted
  So that N idle tabs cost nothing and activity is never missed

  Scenario: Background-tab tokens do not schedule paints
    Given a TUI with a second background tab
    And the first tab is focused
    When 20 tokens stream to the background tab
    Then no frame is painted for the background stream
    And no frame is painted even after the render loop settles

  Scenario: Streamed background output marks the tab unread
    Given a TUI with a second background tab
    When 20 tokens stream to the background tab
    Then the background tab is marked unread

  Scenario: Switching to a tab clears its unread dot
    Given a TUI with a second background tab
    And 20 tokens already streamed to the background tab
    And the background tab is marked unread
    When the user switches to the background tab
    Then the background tab is no longer marked unread

  Scenario: A tab switch paints exactly one frame
    Given a TUI with a second background tab
    And 20 tokens already streamed to the background tab
    When the user switches to the background tab
    Then exactly one frame is painted for the switch

  Scenario: An ended background turn clears the spinner
    Given a TUI with a second background tab
    And the background tab has a running turn
    When the background turn ends
    Then the background tab shows no spinner

  Scenario: An ended background turn marks the tab unread
    Given a TUI with a second background tab
    And the background tab has a running turn
    When the background turn ends
    Then the background tab is marked unread

  Scenario: A running background turn shows the tab spinner
    Given a TUI with a second background tab
    When a turn starts on the background tab
    Then the background tab shows a spinner

  Scenario: A busy background tab keeps the tab-bar spinner animating
    Given a TUI with a second background tab
    And a running turn on the background tab
    When an animation service tick runs
    Then the rendered tab-bar spinner glyph changes

  Scenario: Retained sub-agent sessions are capped at 30 per tab
    Given a TUI with a second background tab
    And 3 sub-agent sessions already started on the background tab
    When 31 sub-agent sessions start on the active tab
    Then the active tab retains exactly 30 sessions
    And the background tab still retains exactly 3 sessions

  Scenario: Workspaces resume by label
    Given a durable workspace labelled "Auth spike"
    When the resume selector opens with workspaces
    Then the workspace row shows the label "Auth spike"
    And the workspace row does not show the raw workspace id

  Scenario: Orphaned workspaces are not offered for resume
    Given a durable workspace with no resumable sessions
    And workspace garbage collection has run
    When the resume selector opens with workspaces
    Then the orphaned workspace is not offered for resume

  Scenario: Ctrl+Tab switches to the next tab
    Given a TUI with a second background tab
    When the user presses Ctrl+Tab
    Then the background tab becomes the active tab
