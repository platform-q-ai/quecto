@done
Feature: Security Sandbox
  As a system administrator
  I want tools restricted to the workspace directory
  So that the agent cannot access or modify files outside its sandbox

  Scenario: Read file blocked outside workspace
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When the agent tries to validate path "/etc/passwd"
    Then the validation should be an error
    And the error should mention "outside working dir"

  Scenario: Write file blocked outside workspace
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When the agent tries to validate path "/tmp/evil.txt"
    Then the validation should be an error

  Scenario: Path traversal blocked
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When the agent tries to validate path "/tmp/quecto-test/../evil.txt"
    Then the validation should be an error

  Scenario: Relative path traversal blocked
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When the agent tries to validate path "notes/../../etc/passwd"
    Then the validation should be an error

  Scenario: Double-slash traversal blocked
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When the agent tries to validate path "/tmp/quecto-test//..//evil.txt"
    Then the validation should be an error

  Scenario: Valid path inside workspace allowed
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When the agent tries to validate path "/tmp/quecto-test/notes.txt"
    Then the validation should be ok

  Scenario: Dangerous commands blocked even without workspace restriction
    Given restrict_to_workspace is false
    When the agent tries to validate command "rm -rf /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario Outline: Dangerous command patterns are blocked
    When the agent tries to validate command "<command>"
    Then the validation should be an error

    Examples:
      | command              |
      | rm -rf /             |
      | mkfs /dev/sda        |
      | dd if=/dev/zero      |
      | shutdown -h now      |
      | reboot               |

  Scenario: Dangerous command check is case-insensitive
    Given restrict_to_workspace is false
    When the agent tries to validate command "ReBoOt"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Dangerous rm wildcard variant is blocked
    Given restrict_to_workspace is false
    When the agent tries to validate command "rm -rf /*"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  # --- #304: Narrowed chown denylist pattern ---

  Scenario: chown targeting system root is blocked
    Given restrict_to_workspace is false
    When the agent tries to validate command "chown -R root:root /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: chown scoped to workspace is allowed
    Given restrict_to_workspace is false
    When the agent tries to validate command "chown -R user:group ./src"
    Then the validation should be ok

  Scenario: Safe command allowed when restriction disabled
    Given restrict_to_workspace is false
    When the agent tries to validate command "echo hello"
    Then the validation should be ok

  Scenario: Workspace restriction can be disabled
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is false
    When the agent tries to validate path "/tmp/quecto-external/test.txt"
    Then the validation should be ok

  Scenario: Subagent inherits workspace restrictions
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    And a subagent context inheriting restrict_to_workspace
    When the subagent sandbox validates path "/etc/passwd"
    Then the validation should be an error
    And the error should mention "outside working dir"

  Scenario: Heartbeat tasks inherit workspace restrictions
    Given a sandboxed workspace at "/tmp/quecto-test"
    And restrict_to_workspace is true
    When a heartbeat sandbox validates path "/etc/shadow"
    Then the validation should be an error
    And the error should mention "outside working dir"
