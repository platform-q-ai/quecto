@done
Feature: nsjail Exec Isolation
  As a system administrator
  I want shell commands to run inside nsjail containers
  So that accidental damage from LLM-generated commands is contained to the workspace

  # --- Basic execution ---

  @done
  Scenario: Command runs successfully inside nsjail
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo hello from jail |
    Then the tool result should contain "hello from jail"
    And the tool result should not be an error

  @done
  Scenario: Command exit code is captured correctly
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | exit 42 |
    Then the tool result should be an error
    And the tool result should mention exit code 42

  @done
  Scenario: Command stderr is captured
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo oops >&2 && exit 1 |
    Then the tool result should be an error
    And the tool result should contain "oops"

  # --- Workspace isolation ---

  @done
  Scenario: Command can read and write files in the workspace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace containing "input.txt" with content "data"
    When the agent executes nsjail tool "bash" with args:
      | command | cat input.txt > output.txt && echo done |
    Then the tool result should contain "done"
    And the file "output.txt" should exist in the workspace
    And the file "output.txt" should contain "data"

  @done
  Scenario: Command cannot read files outside the workspace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | cat /etc/shadow |
    Then the tool result should be an error

  @done
  Scenario: Command cannot write files outside the workspace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo pwned > /var/escape.txt |
    Then the tool result should be an error
    And the file "/var/escape.txt" should not exist on the host

  @done
  Scenario: Host toolchain is available read-only
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | which git |
    Then the tool result should contain "/usr/bin/git" or similar
    And the tool result should not be an error

  @done
  Scenario: Command cannot modify host toolchain
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | touch /usr/bin/evil |
    Then the tool result should be an error

  # --- Resource limits (rlimit-based) ---

  @done
  Scenario: Memory limit kills a memory-hogging command
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with memory limit 50 MB
    When the agent executes nsjail tool "bash" with args:
      | command | python3 -c "x = 'a' * (100 * 1024 * 1024)" |
    Then the tool result should be an error

  @done
  Scenario: PID limit prevents fork bombs
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID limit 32
    When the agent executes nsjail tool "bash" with args:
      | command | :(){ :\|:& };: |
    Then the tool result should be an error
    And the host should not be affected

  @done
  Scenario: Time limit kills a long-running command
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with time limit 2 seconds
    When the agent executes nsjail tool "bash" with args:
      | command | sleep 60 |
    Then the tool result should be an error
    And the nsjail execution should complete within 5 seconds

  @done
  Scenario: CPU time limit kills a CPU-hogging command
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with CPU time limit 2 seconds
    When the agent executes nsjail tool "bash" with args:
      | command | while true; do :; done |
    Then the tool result should be an error

  @done
  Scenario: nsjail uses rlimit_as for memory enforcement
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with memory limit 512 MB
    Then the nsjail command for "echo test" should contain "--rlimit_as"
    And the nsjail command for "echo test" should contain "512"

  @done
  Scenario: default nsjail memory limit is 4096 MB to allow Node/V8/JVM/Go runtimes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with default options
    Then the nsjail command for "echo test" should contain "--rlimit_as"
    And the nsjail command for "echo test" should contain "4096"

  @done
  Scenario: default nsjail CPU time limit is 28800 seconds (2 cores x 4-hour wall budget)
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with default options
    Then the nsjail command for "echo test" should contain "--rlimit_cpu"
    And the nsjail command for "echo test" should contain "28800"

  @done
  Scenario: default nsjail wall-clock time limit is 14400 seconds (4 hours)
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with default options
    Then the nsjail command for "echo test" should contain "--time_limit"
    And the nsjail command for "echo test" should contain "14400"

  @done
  Scenario: default nsjail tmp filesystem limit is 512 MB
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with default options
    Then the nsjail command for "echo test" should contain "none:/tmp:tmpfs:size="
    And the nsjail command for "echo test" should contain "536870912"

  @done
  Scenario: tmp_size_mb is configurable via exec settings
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with tmp size 1024 MB
    Then the nsjail command for "echo test" should contain "1073741824"

  @done
  Scenario: exec tool tokio timeout matches wall-clock nsjail limit by default
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with default options
    Then the exec tool default tokio timeout should equal the nsjail wall-clock limit

  @done
  Scenario: Node.js can start inside nsjail with default memory limit
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with default options
    When the agent executes nsjail tool "bash" with args:
      | command | node -e "console.log('ok')" |
    Then the tool result should contain "ok"
    And the tool result should not be an error

  @done
  Scenario: nsjail uses rlimit_nproc for PID enforcement
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID limit 128
    Then the nsjail command for "echo test" should contain "--rlimit_nproc"
    And the nsjail command for "echo test" should contain "128"

  @done
  Scenario: nsjail uses rlimit_cpu for CPU time enforcement
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with CPU time limit 15 seconds
    Then the nsjail command for "echo test" should contain "--rlimit_cpu"
    And the nsjail command for "echo test" should contain "15"

  @done
  Scenario: nsjail disables cgroup namespace
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "--disable_clone_newcgroup"

  @done
  Scenario: nsjail includes system RO bindmounts
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "--bindmount_ro"

  # --- Process isolation ---

  @done
  Scenario: Command cannot see host processes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID namespace
    When the agent executes nsjail tool "bash" with args:
      | command | ps aux |
    Then the tool result should not contain host process names
    And the process list should only show jail-internal processes

  @done
  Scenario: Command cannot signal host processes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with PID namespace
    When the agent executes nsjail tool "bash" with args:
      | command | kill -9 1 |
    Then the tool result should be an error
    And the host init process should not be affected

  # --- Network control ---

  @done
  Scenario: Network passthrough allows outbound access when configured
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with network passthrough enabled
    When the agent executes nsjail tool "bash" with args:
      | command | curl -s -o /dev/null -w '%{http_code}' https://example.com |
    Then the tool result should contain "200"

  @done
  Scenario: Network isolation blocks outbound access when configured
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with network isolation enabled
    When the agent executes nsjail tool "bash" with args:
      | command | curl -s --max-time 2 https://example.com |
    Then the tool result should be an error

  # --- Sandbox denylist integration ---

  @done
  Scenario: Sandbox denylist is applied before nsjail invocation
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with sandbox denylist
    When the agent executes nsjail tool "bash" with args:
      | command | rm -rf / |
    Then the tool result should be an error
    And the nsjail error should mention "dangerous pattern"
    And nsjail should not have been invoked

  @done
  Scenario: Sandbox allowlist is enforced before nsjail invocation
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with sandbox allowlist "echo,ls,cat"
    When the agent executes nsjail tool "bash" with args:
      | command | curl http://evil.com |
    Then the tool result should be an error
    And the nsjail error should mention "not in allowlist"

  # --- Configuration and fallback ---

  @done
  Scenario: exec.isolation config selects nsjail mode
    Given a config file with exec.isolation set to "nsjail"
    And nsjail is available on the system
    When the tool registry is constructed
    Then the exec tool should use nsjail isolation

  @done
  Scenario: exec.isolation config selects native mode
    Given a config file with exec.isolation set to "native"
    When the tool registry is constructed
    Then the exec tool should use native isolation with sandbox denylist only

  @done
  Scenario: nsjail unavailable triggers graceful fallback to native
    Given a config file with exec.isolation set to "nsjail"
    And exec native fallback is allowed
    And nsjail is not available on the system
    When the tool registry is constructed
    Then the exec tool should fall back to native isolation
    And a warning should be logged mentioning nsjail unavailability

  # --- Environment safety ---

  @done
  Scenario: QUECTO-prefixed env vars are not visible inside nsjail
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    And the environment has QUECTO_SECRET_KEY set to "hunter2"
    When the agent executes nsjail tool "bash" with args:
      | command | env |
    Then the tool result should not contain "QUECTO_SECRET_KEY"
    And the tool result should not contain "hunter2"

  # --- Writable /tmp (tmpfs) ---

  @done
  Scenario: Command can write to /tmp inside the sandbox
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo hello > /tmp/test.txt && cat /tmp/test.txt |
    Then the tool result should contain "hello"
    And the tool result should not be an error

  @done
  Scenario: mktemp works inside the sandbox
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | mktemp && echo ok |
    Then the tool result should contain "ok"
    And the tool result should not be an error

  @done
  Scenario: nsjail command includes bounded tmpfs mount for /tmp
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "none:/tmp:tmpfs:size="
    And the nsjail command for "echo test" should not contain "--tmpfsmount"

  @done
  Scenario: TMPDIR is set to /tmp inside the sandbox
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo $TMPDIR |
    Then the tool result should contain "/tmp"
    And the tool result should not be an error

  # --- Cleanup ---

  @done
  Scenario: nsjail sandbox is cleaned up after command completes
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo done |
    Then no nsjail processes should remain running
    And no stale mount namespaces should remain

  # --- /dev device node mounts ---

  @done
  Scenario: nsjail command includes bindmount_ro for /dev/null
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "/dev/null:/dev/null"

  @done
  Scenario: nsjail command includes bindmount_ro for /dev/urandom
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "/dev/urandom:/dev/urandom"

  @done
  Scenario: nsjail command includes bindmount_ro for /dev/zero
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "/dev/zero:/dev/zero"

  @done
  Scenario: nsjail command includes bindmount_ro for /dev/random
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    Then the nsjail command for "echo test" should contain "/dev/random:/dev/random"

  @done
  Scenario: Redirect to /dev/null succeeds inside nsjail
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with a workspace
    When the agent executes nsjail tool "bash" with args:
      | command | echo ok >/dev/null && echo done |
    Then the tool result should contain "done"
    And the tool result should not be an error

  # --- Output capture ---

  @done
  Scenario: Large stdout is captured up to the configured limit
    Given nsjail is available on the system
    And an nsjail-isolated exec tool with output capture limit 1 MiB
    When the agent executes nsjail tool "bash" with args:
      | command | python3 -c "print('A' * 1500000)" |
    Then the tool result should be truncated to approximately 1 MiB
    And the tool result should indicate truncation
