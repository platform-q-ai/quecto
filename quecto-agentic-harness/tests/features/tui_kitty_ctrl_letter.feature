@tui @pending
Feature: TUI handles Kitty keyboard protocol Ctrl+letter sequences
  Issue #496: Ctrl+D (and other Ctrl+letter combos) arrive as CSI u
  sequences under Kitty keyboard protocol but are not parsed, so
  Ctrl+D does not exit the app.

  Scenario: Kitty Ctrl+D parsed as Key::Ctrl('d')
    Given the Kitty keyboard protocol is active
    When the terminal sends CSI 100;5u (Ctrl+D)
    Then the parsed key should be Ctrl('d')

  Scenario: Kitty Ctrl+C parsed as Key::Ctrl('c')
    Given the Kitty keyboard protocol is active
    When the terminal sends CSI 99;5u (Ctrl+C)
    Then the parsed key should be Ctrl('c')

  Scenario: Kitty Ctrl+A parsed as Key::Ctrl('a')
    When the terminal sends CSI 97;5u (Ctrl+A)
    Then the parsed key should be Ctrl('a')

  Scenario: Kitty Ctrl+Z parsed as Key::Ctrl('z')
    When the terminal sends CSI 122;5u (Ctrl+Z)
    Then the parsed key should be Ctrl('z')

  Scenario: Kitty Ctrl+L parsed as Key::Ctrl('l')
    When the terminal sends CSI 108;5u (Ctrl+L)
    Then the parsed key should be Ctrl('l')

  Scenario: Kitty plain letter not misidentified as Ctrl
    When the terminal sends CSI 100;1u (plain 'd', no modifier)
    Then the parsed key should be Char('d')

  Scenario: Kitty Shift+Enter still works
    When the terminal sends CSI 13;2u
    Then the parsed key should be ShiftEnter

  Scenario: Kitty Alt+letter parsed
    When the terminal sends CSI 100;3u (Alt+D)
    Then the parsed key should be Alt('d')
