@tui @done @multi-tab-round2
Feature: Removed multi-session TUI round-2 affordances (#1596)
  As a TUI user
  I keep single-session behavior while old tab shortcuts are gone

  Scenario: The frame renders no version header line
    Given a headless TUI
    When the frame renders
    Then no frame line contains the version header text
    And the first frame line is a blank spacer

  Scenario: The frame keeps the terminal height after the header swap
    Given a headless TUI
    When the frame renders
    Then the frame height equals the terminal height

  Scenario: Multiple seeded tabs still do not render a tab bar
    Given a TUI with a second background tab
    When the frame renders
    Then the first frame line is a blank spacer
    And the frame height equals the terminal height

  Scenario: Ctrl+N no longer opens a new tab
    Given a headless TUI
    When the user presses Ctrl+N
    Then still only one tab is open

  Scenario: Ctrl+T still opens the tool policy selector
    Given a headless TUI
    When the user presses Ctrl+T
    Then the tool policy selector is open
    And still only one tab is open

  Scenario: /hotkeys does not document the old new-tab chord
    Given a headless TUI
    When the user runs /hotkeys
    Then the help text omits Ctrl+N as the new-tab chord

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
