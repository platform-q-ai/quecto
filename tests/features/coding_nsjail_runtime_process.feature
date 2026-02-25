@wip @coding-live
Feature: NsjailWorkerRuntime Real Process Spawn
  As the coding coordinator
  I need NsjailWorkerRuntime to spawn real OS processes with piped I/O
  So that coding workers actually execute and communicate via JSON Lines IPC

  The runtime must replace mock PID assignment with real std::process::Command
  spawning, pipe stdin/stdout/stderr, and track process lifecycle. These tests
  use a helper script instead of real nsjail to validate the spawn/IPC/lifecycle
  mechanics without requiring nsjail capabilities.

  # --- Real process spawn ---

  Scenario: launch() returns a real OS PID
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches a worker process
    Then the returned PID should be a valid OS process ID
    And the process should be alive

  Scenario: launch() spawns a process that can be found in the OS
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches a worker process
    Then the PID should correspond to a real OS process

  Scenario: Multiple launches return distinct PIDs
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches 3 worker processes
    Then all 3 PIDs should be distinct
    And all 3 processes should be alive

  # --- Stdin/stdout IPC ---

  Scenario: send_command() writes to the worker's stdin
    Given a nsjail runtime configured with an echo worker script
    When the runtime launches a worker process
    And the runtime sends command "hello world" to the worker
    Then the worker should echo back "hello world" on stdout

  Scenario: read_event() returns Valid for well-formed JSON Lines
    Given a nsjail runtime configured with a json-lines worker script
    When the runtime launches a worker process
    And the runtime sends command "emit" to the worker
    Then read_event should return a Valid event

  Scenario: read_event() returns Malformed for non-JSON output
    Given a nsjail runtime configured with a plain-text worker script
    When the runtime launches a worker process
    And the runtime sends command "say" to the worker
    Then read_event should return a Malformed event

  Scenario: read_event() returns None when no output is available
    Given a nsjail runtime configured with a silent worker script
    When the runtime launches a worker process
    Then read_event should return None

  # --- Stderr capture ---

  Scenario: read_stderr() captures worker stderr output
    Given a nsjail runtime configured with a stderr worker script
    When the runtime launches a worker process
    And the runtime sends command "warn" to the worker
    And the runtime waits briefly for stderr
    Then read_stderr should contain the warning message

  Scenario: Stderr capture is capped at 1 MiB for real processes
    Given a nsjail runtime configured with a large-stderr worker script
    When the runtime launches a worker process
    And the runtime sends command "flood" to the worker
    And the runtime waits for the worker to exit
    Then read_stderr should be at most 1048576 bytes

  # --- Process exit detection ---

  Scenario: status() returns Running for a live process
    Given a nsjail runtime configured with a long-running worker script
    When the runtime launches a worker process
    Then the runtime status should be Running

  Scenario: status() returns Exited with code 0 after clean exit
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches a worker process
    And the runtime sends command "exit" to the worker
    And the runtime waits for the worker to exit
    Then the runtime status should be Exited with code 0

  Scenario: status() returns Exited with non-zero code after failure
    Given a nsjail runtime configured with a failing worker script
    When the runtime launches a worker process
    And the runtime waits for the worker to exit
    Then the runtime status should be Exited with code 1

  # --- Kill ---

  Scenario: kill() terminates a running worker process
    Given a nsjail runtime configured with a long-running worker script
    When the runtime launches a worker process
    And the runtime kills the worker
    Then the process should not be alive
    And the runtime status should be Killed

  Scenario: kill() is idempotent for already-exited processes
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches a worker process
    And the runtime sends command "exit" to the worker
    And the runtime waits for the worker to exit
    Then killing the worker should not return an error

  # --- Cleanup ---

  Scenario: cleanup() releases resources for a terminated worker
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches a worker process
    And the runtime sends command "exit" to the worker
    And the runtime waits for the worker to exit
    And the runtime cleans up the worker
    Then the runtime status should be Killed with reason containing "unknown"

  # --- is_alive ---

  Scenario: is_alive() returns true for running processes
    Given a nsjail runtime configured with a long-running worker script
    When the runtime launches a worker process
    Then the process should be alive

  Scenario: is_alive() returns false after process exits
    Given a nsjail runtime configured with a helper worker script
    When the runtime launches a worker process
    And the runtime sends command "exit" to the worker
    And the runtime waits for the worker to exit
    Then the process should not be alive

  # --- Error handling ---

  Scenario: launch() fails when the binary does not exist
    Given a nsjail runtime configured with a nonexistent binary
    When the runtime attempts to launch a worker process
    Then the launch should fail with an error containing "spawn"

  Scenario: send_command() fails for an unknown PID
    Given a nsjail runtime configured with a helper worker script
    When the runtime sends command "hello" to PID 99999
    Then the send should fail with an error containing "unknown"

  Scenario: kill() for unknown PID returns an error
    Given a nsjail runtime configured with a helper worker script
    When the runtime kills PID 99999
    Then the kill should fail with an error containing "unknown"
