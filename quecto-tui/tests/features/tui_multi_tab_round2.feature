@tui @done @multi-tab-round2
Feature: Multi-session TUI round-2 fix pass (#1466 / PR #1485 field regressions)
  As a TUI user who re-tested PR #1485 in the field
  I want the header line gone, a terminal-safe new-tab chord,
  and user sends to restored sub-agents delivered
  So that the multi-tab TUI is clean and messaging never dead-ends

  # Item 1 — the version/help header line is dropped, replaced by a blank
  # spacer so the tab bar and Master status line keep breathing room.

  Scenario: The frame renders no version header line
    Given a headless TUI
    When the frame renders
    Then no frame line contains the version header text
    And the first frame line is a blank spacer

  Scenario: The frame keeps the terminal height after the header swap
    Given a headless TUI
    When the frame renders
    Then the frame height equals the terminal height

  Scenario: The blank spacer follows the tab bar with multiple tabs
    Given a TUI with a second background tab
    When the frame renders
    Then the first frame line is the tab bar
    And the second frame line is a blank spacer
    And the frame height equals the terminal height

  # Item 2 — new-tab chord. Ctrl+T is already the tool-policy selector, so
  # the next best terminal-safe plain-control chord is Ctrl+N (0x0E — arrives
  # unmodified in every terminal and tmux; only Ctrl+SHIFT+N is taken).

  Scenario: Ctrl+N opens a new tab
    Given a headless TUI
    When the user presses Ctrl+N
    Then a second tab is open
    And the new tab is the active tab

  Scenario: Ctrl+T still opens the tool policy selector
    Given a headless TUI
    When the user presses Ctrl+T
    Then the tool policy selector is open
    And still only one tab is open

  Scenario: /hotkeys documents the new-tab chord
    Given a headless TUI
    When the user runs /hotkeys
    Then the help text lists Ctrl+N as the new-tab chord

  # Item 3 — user sends to restored sub-agents must reach the child, the
  # same way master-driven messaging does, instead of erroring "not attached".

  Scenario: A user message to a live restored sub-agent is delivered
    Given a running sub-agent restored from a resumed workspace is focused
    When the user submits a message to it
    Then the user entry appears in the sub-agent transcript
    And no delivery-failure error is surfaced

  Scenario: A user message to a reachable but detached sub-agent is delivered
    Given a reachable sub-agent still marked detached is focused
    When the user submits a message to it
    Then the user entry appears in the sub-agent transcript
    And no delivery-failure error is surfaced

  Scenario: A user message to a dead sub-agent still surfaces an error
    Given a dead restored sub-agent is focused
    When the user submits a message to it
    Then a delivery failure naming the sub-agent is visibly surfaced
