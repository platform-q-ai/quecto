@tui @done
Feature: TUI restores terminal fully on exit
  As a TUI user
  I want arrow keys to work normally in my shell after quitting
  So that the TUI doesn't corrupt my terminal state

  # reason: needs real terminal teardown capture — cooked/canonical mode and
  # cursor restoration are written straight to the process stdout/termios with
  # no injectable writer, so they can't be asserted headlessly.
  @pending
  Scenario: Terminal restored after normal exit
    Given the TUI is running in raw mode with Kitty protocol
    When the user exits with Ctrl+D
    Then the terminal should be in cooked/canonical mode
    And arrow keys should function normally in the shell
    And the cursor should be visible

  # reason: needs a real SIGINT + terminal teardown capture; the restore writes
  # to the process stdout/termios directly with no injectable writer.
  @pending
  Scenario: Terminal restored after Ctrl+C exit
    Given the TUI is running
    When the process receives SIGINT
    Then the terminal should still be restored

  # reason: needs real terminal teardown capture — bracketed paste has no state
  # flag; it is disabled by writing \x1b[?2004l straight to stdout in
  # Terminal::exit_raw_mode, which has no injectable writer to assert against.
  @pending
  Scenario: Bracketed paste mode disabled on exit
    Given the TUI enabled bracketed paste mode
    When the TUI exits
    Then bracketed paste should be disabled

  Scenario: modifyOtherKeys disabled on exit
    Given the TUI enabled modifyOtherKeys mode
    When the TUI exits
    Then modifyOtherKeys should be reset to mode 0
