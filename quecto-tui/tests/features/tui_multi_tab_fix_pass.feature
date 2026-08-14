@tui @done @multi-tab-fix-pass
Feature: Multi-session TUI fix pass (#1466 / PR #1485 field regressions)
  As a TUI user who tested PR #1485 in the field
  I want the tab bar, key chords, resume overlay, workspace resurrection,
  dead sub-agent handling, and background spinners fixed
  So that multi-session tabs are actually usable day to day

  # Item 1 — tab bar: herdr-style reverse-video number blocks (spike design)

  Scenario: The tab bar renders number blocks without a default tab name
    Given a TUI with a second background tab
    When the tab bar renders
    Then the bar shows the active tab as a cyan block and the rest as dim blocks
    And the bar never shows a default ":Master" suffix
    And the bar ends with a dim new-tab button

  Scenario: Clicking a tab block focuses that tab
    Given a TUI with a second background tab
    When the user clicks inside the second tab block
    Then the background tab becomes the active tab

  Scenario: Clicking the new-tab button opens a tab
    Given a TUI with a second background tab
    When the user clicks the new-tab button
    Then a third tab is open

  Scenario: Clicking the tab bar's dead space changes nothing
    Given a TUI with a second background tab
    When the user clicks past the end of the tab bar
    Then the first tab is still the active tab
    And no new tab is open

  # Item 2 — terminal-safe cycle chords (Hyprland grabs Alt/Ctrl+Tab).
  # Three tabs so direction is falsifiable: prev from the first tab wraps to
  # the LAST tab, while next lands on the second.

  Scenario: Ctrl+PageDown cycles to the next tab
    Given a TUI with two background tabs
    When the user presses Ctrl+PageDown
    Then the second tab becomes the active tab

  Scenario: Ctrl+PageUp cycles to the previous tab
    Given a TUI with two background tabs
    When the user presses Ctrl+PageUp
    Then the last tab becomes the active tab

  # Item 3 — /resume overlay: recency order + recognizable rows

  Scenario: Workspaces are listed most-recently-active first
    Given two durable workspaces with different last-active times
    When the resume selector opens with workspaces
    Then the first workspace row is the most recently active one

  Scenario: Workspace rows show a relative last-active time
    Given a durable workspace last active two hours ago
    When the resume selector opens with workspaces
    Then the workspace row shows "2h ago"

  Scenario: Workspace rows show a conversation snippet per tab
    Given a durable workspace whose tab summary is "fix the auth bug"
    When the resume selector opens with workspaces
    Then the workspace row shows the snippet "fix the auth bug"

  # Item 4 — workspace resurrection: first entry must not take a lossy path

  Scenario: Every stored session resumes even when stored tab ids are stale
    Given a TUI with a second background tab
    When a workspace manifest with stale stored tab ids is restored
    Then every stored session is carried by a tab

  # Item 5 — dead sub-agents must not swallow messages

  Scenario: Sending to a detached sub-agent surfaces a visible outcome
    Given a detached sub-agent is focused
    When the user submits a message to the focused sub-agent
    Then a delivery failure naming the sub-agent is visibly surfaced

  # Item 6 — background-tab spinners keep animating

  Scenario: A busy background tab keeps the tab bar spinner animating
    Given a TUI with a second background tab
    And a running turn on the background tab
    When an animation service tick runs
    Then the rendered tab-bar spinner glyph changes
    And the animation tick requests a repaint
