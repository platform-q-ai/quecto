@tui @security @done
Feature: TUI PID cast safety — u32 to i32 checked conversion
  As a TUI user
  I want the leader-only exit signal to use safe PID conversion
  So that a wrapped u32→i32 cast cannot address PID 1 (init) or a process group

  Background:
    Given the TUI spawns an agent as a child process
    And the child process runs in its own process group

  Scenario: Normal PID is converted safely
    Given the child process has PID 1234
    When the TUI converts the PID for the leader signal
    Then the converted PID should be 1234
    And SIGTERM should be sent to the single process 1234

  Scenario: PID at i32 maximum boundary converts safely
    Given the child process has PID 2147483647
    When the TUI converts the PID for the leader signal
    Then the converted PID should be 2147483647
    And SIGTERM should be sent to the single process 2147483647

  Scenario: PID exceeding i32 maximum is rejected
    Given the child process has PID 2147483648
    When the TUI converts the PID for the leader signal
    Then the conversion should fail
    And no signal should be sent
    And the error should mention the PID value

  Scenario: PID u32::MAX does not wrap to -1 (init)
    Given the child process has PID 4294967295
    When the TUI converts the PID for the leader signal
    Then the conversion should fail
    And no signal should be sent
    And SIGTERM must NOT be sent to PID 1

  Scenario: PID 0 is rejected to prevent signalling the caller's group
    Given the child process has PID 0
    When the TUI converts the PID for the leader signal
    Then the conversion should fail
    And no signal should be sent
