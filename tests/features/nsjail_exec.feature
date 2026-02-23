@pending
Feature: nsjail Exec Isolation
  As a system administrator
  I want shell commands to run inside nsjail containers
  So that accidental damage from LLM-generated commands is contained to the workspace

  # --- Basic execution ---

  @pending
  Scenario: Command runs successfully inside nsjail
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | echo hello from jail |
    Then the tool result should contain "hello from jail"
    And the tool result should not be an error

  @pending
  Scenario: Command exit code is captured correctly
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | exit 42 |
    Then the tool result should be an error
    And the tool result should mention exit code 42

  @pending
  Scenario: Command stderr is captured
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | echo oops >&2 && exit 1 |
    Then the tool result should be an error
    And the tool result should contain "oops"

  # --- Workspace isolation ---

  @pending
  Scenario: Command can read and write files in the workspace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace containing "input.txt" with content "data"
    When the agent executes tool "exec" with args:
      | command | cat input.txt > output.txt && echo done |
    Then the tool result should contain "done"
    And the file "output.txt" should exist in the workspace
    And the file "output.txt" should contain "data"

  @pending
  Scenario: Command cannot read files outside the workspace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | cat /etc/shadow |
    Then the tool result should be an error

  @pending
  Scenario: Command cannot write files outside the workspace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | echo pwned > /tmp/escape.txt |
    Then the tool result should be an error
    And the file "/tmp/escape.txt" should not exist on the host

  @pending
  Scenario: Host toolchain is available read-only
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | which git |
    Then the tool result should contain "/usr/bin/git" or similar
    And the tool result should not be an error

  @pending
  Scenario: Command cannot modify host toolchain
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | touch /usr/bin/evil |
    Then the tool result should be an error

  # --- Resource limits ---

  @pending
  Scenario: Memory limit kills a memory-hogging command
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with memory limit 50 MB
    When the agent executes tool "exec" with args:
      | command | python3 -c "x = 'a' * (100 * 1024 * 1024)" |
    Then the tool result should be an error

  @pending
  Scenario: PID limit prevents fork bombs
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID limit 32
    When the agent executes tool "exec" with args:
      | command | :(){ :\|:& };: |
    Then the tool result should be an error
    And the host should not be affected

  @pending
  Scenario: Time limit kills a long-running command
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with time limit 2 seconds
    When the agent executes tool "exec" with args:
      | command | sleep 60 |
    Then the tool result should be an error
    And the execution should complete within 5 seconds

  @pending
  Scenario: CPU time limit kills a CPU-hogging command
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with CPU time limit 2 seconds
    When the agent executes tool "exec" with args:
      | command | while true; do :; done |
    Then the tool result should be an error

  # --- Process isolation ---

  @pending
  Scenario: Command cannot see host processes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID namespace
    When the agent executes tool "exec" with args:
      | command | ps aux |
    Then the tool result should not contain host process names
    And the process list should only show jail-internal processes

  @pending
  Scenario: Command cannot signal host processes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID namespace
    When the agent executes tool "exec" with args:
      | command | kill -9 1 |
    Then the tool result should be an error
    And the host init process should not be affected

  # --- Network control ---

  @pending
  Scenario: Network passthrough allows outbound access when configured
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with network passthrough enabled
    When the agent executes tool "exec" with args:
      | command | curl -s -o /dev/null -w '%{http_code}' https://example.com |
    Then the tool result should contain "200"

  @pending
  Scenario: Network isolation blocks outbound access when configured
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with network isolation enabled
    When the agent executes tool "exec" with args:
      | command | curl -s --max-time 2 https://example.com |
    Then the tool result should be an error

  # --- Sandbox denylist integration ---

  @pending
  Scenario: Sandbox denylist is applied before nsjail invocation
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with sandbox denylist
    When the agent executes tool "exec" with args:
      | command | rm -rf / |
    Then the tool result should be an error
    And the error should mention "dangerous pattern"
    And nsjail should not have been invoked

  @pending
  Scenario: Sandbox allowlist is enforced before nsjail invocation
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with sandbox allowlist "echo,ls,cat"
    When the agent executes tool "exec" with args:
      | command | curl http://evil.com |
    Then the tool result should be an error
    And the error should mention "not in allowlist"

  # --- Configuration and fallback ---

  @pending
  Scenario: exec.isolation config selects nsjail mode
    Given a config file with exec.isolation set to "nsjail"
    And nsjail is available on the system
    When the tool registry is constructed
    Then the exec tool should use nsjail isolation

  @pending
  Scenario: exec.isolation config selects native mode
    Given a config file with exec.isolation set to "native"
    When the tool registry is constructed
    Then the exec tool should use native isolation with sandbox denylist only

  @pending
  Scenario: nsjail unavailable triggers graceful fallback to native
    Given a config file with exec.isolation set to "nsjail"
    And nsjail is not available on the system
    When the tool registry is constructed
    Then the exec tool should fall back to native isolation
    And a warning should be logged mentioning nsjail unavailability

  # --- Environment safety ---

  @pending
  Scenario: QUECTO-prefixed env vars are not visible inside nsjail
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    And the environment has QUECTO_SECRET_KEY set to "hunter2"
    When the agent executes tool "exec" with args:
      | command | env |
    Then the tool result should not contain "QUECTO_SECRET_KEY"
    And the tool result should not contain "hunter2"

  # --- Cleanup ---

  @pending
  Scenario: nsjail sandbox is cleaned up after command completes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes tool "exec" with args:
      | command | echo done |
    Then no nsjail processes should remain running
    And no stale mount namespaces should remain

  @pending
  Scenario: nsjail sandbox is cleaned up after parent crash
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with die-with-parent enabled
    When the parent process is killed during exec
    Then the nsjail sandbox process should also be terminated

  # --- Output capture ---

  @pending
  Scenario: Large stdout is captured up to the configured limit
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with output capture limit 1 MiB
    When the agent executes tool "exec" with args:
      | command | dd if=/dev/urandom bs=1024 count=2048 2>/dev/null \| base64 |
    Then the tool result should be truncated to approximately 1 MiB
    And the tool result should indicate truncation
