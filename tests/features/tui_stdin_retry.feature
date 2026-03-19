@tui
Feature: TUI stdin buffer retry loop — multi-fragment escape sequences
  As a TUI user on a slow SSH/serial connection
  I want escape sequences split across 3+ reads to be reassembled
  So that arrow keys and other CSI sequences work reliably

  Background:
    Given the TUI uses a StdinBuffer with retry-on-pending logic
    And the escape timeout is 10ms per retry

  Scenario: 2-fragment CSI split is reassembled
    Given fragment 1 arrives: ESC (0x1b)
    And 5ms later fragment 2 arrives: "[A"
    When the retry loop processes pending data
    Then the emitted sequence should be ESC[A (Up arrow)
    And no bytes should be force-drained as individual bytes

  Scenario: 3-fragment CSI split is reassembled
    Given fragment 1 arrives: ESC (0x1b)
    And 5ms later fragment 2 arrives: "["
    And 5ms later fragment 3 arrives: "A"
    When the retry loop processes pending data
    Then the emitted sequence should be ESC[A (Up arrow)
    And the ESC and "[" should NOT be emitted as separate bytes

  Scenario: Bare ESC after all retries exhausted
    Given only ESC (0x1b) arrives
    And no more data arrives within the retry window
    When the retry loop exhausts all attempts
    Then ESC should be emitted as a bare Escape key
    And the buffer should be empty

  Scenario: Retry loop is capped to prevent infinite wait
    Given an incomplete CSI sequence that never completes
    When the retry loop runs
    Then it should stop after at most 5 retry iterations
    And force-drain the incomplete bytes

  Scenario: Complete sequence does not trigger retry loop
    Given a complete CSI sequence ESC[A arrives in one read
    When the buffer processes the data
    Then the sequence should be emitted immediately
    And no retry loop should be entered
