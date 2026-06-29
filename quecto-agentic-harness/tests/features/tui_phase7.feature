@tui @pending
Feature: TUI Phase 7 — Theming, Kitty Protocol, and Terminal Polish
  Visual polish and advanced terminal features.

  Scenario: Theme applies semantic colors
    Given a theme "dark"
    When accent color is applied to "hello"
    Then the output should contain cyan ANSI escape

  Scenario: Theme applies error color
    Given a theme "dark"
    When error color is applied to "fail"
    Then the output should contain red ANSI escape

  Scenario: Named themes are selectable
    Given available themes "dark", "light"
    When "light" is selected
    Then the active theme should be "light"

  Scenario: Kitty protocol query is sent on startup
    Given a terminal that supports Kitty protocol
    When the TUI starts
    Then it should send CSI ? u query

  Scenario: Kitty protocol response enables enhanced keys
    Given a Kitty protocol response with flags 7
    When parsed
    Then Kitty protocol should be marked active

  Scenario: Fallback to modifyOtherKeys when Kitty unsupported
    Given no Kitty protocol response within timeout
    When fallback triggers
    Then modifyOtherKeys mode 2 should be enabled

  Scenario: SIGWINCH triggers resize
    Given the TUI is running
    When a SIGWINCH signal is received
    Then terminal dimensions should be refreshed
    And the renderer should be invalidated

  Scenario: SIGTSTP suspends and resumes cleanly
    Given the TUI is in raw mode
    When Ctrl+Z is pressed
    Then raw mode should be exited before suspend
    And raw mode should be re-entered on resume

  Scenario: Panic handler restores terminal
    Given the TUI is in raw mode
    When a panic occurs
    Then termios should be restored to cooked mode
    And the cursor should be visible
