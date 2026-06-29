@tui @security
Feature: TUI stdin buffer size cap — prevent unbounded memory growth
  As a TUI user
  I want the stdin buffer capped at a safe maximum size
  So that broken bracketed paste or malicious input cannot exhaust memory

  Scenario: Buffer rejects data beyond size cap
    Given the stdin buffer is empty
    When 64KB of data is fed into the buffer
    Then the buffer should accept the data
    When 1 more byte is fed
    Then the extra byte should be silently dropped
    And the buffer size should not exceed 64KB

  Scenario: Normal input well within cap
    Given the stdin buffer is empty
    When a 100-byte escape sequence is fed
    Then the buffer should accept all bytes
    And drain_complete should return the sequence

  Scenario: Broken bracketed paste does not grow unbounded
    Given the stdin buffer is empty
    When a bracketed paste start marker arrives without end marker
    And 100KB of paste content follows
    Then the buffer should stop accepting data at 64KB
    And memory usage should remain bounded

  Scenario: Paste end-marker scan is efficient
    Given a large bracketed paste (60KB) with proper end marker
    When drain_complete is called
    Then the paste should be extracted as one sequence
    And the scan should not exhibit O(n²) behavior
