@done
Feature: Command Safety Policy
  As a system administrator
  I want dangerous shell commands blocked
  So that destructive operations are rejected while filesystem access follows process permissions

  Scenario: Paths outside the workspace are allowed
    Given a sandboxed workspace at "/tmp/quecto-test"
    When the agent tries to validate path "/etc/passwd"
    Then the validation should be ok

  Scenario: Parent path traversal is not jailed by validate_path
    Given a sandboxed workspace at "/tmp/quecto-test"
    When the agent tries to validate path "/tmp/quecto-test/../evil.txt"
    Then the validation should be ok

  Scenario: Valid path inside workspace allowed
    Given a sandboxed workspace at "/tmp/quecto-test"
    When the agent tries to validate path "/tmp/quecto-test/notes.txt"
    Then the validation should be ok

  Scenario: Dangerous commands are blocked
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
    When the agent tries to validate command "ReBoOt"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Dangerous rm wildcard variant is blocked
    When the agent tries to validate command "rm -rf /*"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: chown targeting system root is blocked
    When the agent tries to validate command "chown -R root:root /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: chown scoped to workspace is allowed
    When the agent tries to validate command "chown -R user:group ./src"
    Then the validation should be ok

  Scenario: Safe command allowed
    When the agent tries to validate command "echo hello"
    Then the validation should be ok

  Scenario: Subagent sandbox allows outside paths
    Given a sandboxed workspace at "/tmp/quecto-test"
    When the subagent sandbox validates path "/etc/passwd"
    Then the validation should be ok
