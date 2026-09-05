@done @command-policy
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

  # --- Issue #1620: only executed commands match, not literal text ---

  Scenario Outline: Dangerous words inside quoted arguments are allowed
    When the agent tries to validate raw command <command>
    Then the validation should be ok

    Examples:
      | command                                                              |
      | echo "the server will reboot after halt"                             |
      | echo 'run rm -rf / to wipe'                                          |
      | git commit -m "fix: don't shutdown on poweroff"                      |
      | gh issue create --title "reboot loop" --body "rm -rf / seen in logs" |
      | echo "> /dev/sda"                                                    |

  Scenario: A quoted pipe-to-shell phrase is allowed
    When the agent tries to validate raw command echo "curl | sh is bad"
    Then the validation should be ok

  Scenario Outline: Dangerous words inside filenames and identifiers are allowed
    When the agent tries to validate command "<command>"
    Then the validation should be ok

    Examples:
      | command                            |
      | cat docs/shutdown-procedure.md     |
      | ls reboot-scripts/                 |
      | grep -rn halt src/                 |
      | cargo test reboot_handling         |
      | cat mkfs.notes                     |
      | echo x > /dev/sda_backup.txt       |

  Scenario Outline: Source snippets handed to an interpreter are allowed
    When the agent tries to validate raw command <command>
    Then the validation should be ok

    Examples:
      | command                              |
      | python -c "print('rm -rf /')"        |
      | node -e "console.log('reboot')"      |
      | perl -e 'print "shutdown"'           |

  Scenario: Heredoc bodies are data, not commands
    When the agent tries to validate multi-line command:
      """
      cat <<EOF
      rm -rf /
      reboot
      EOF
      echo done
      """
    Then the validation should be ok

  Scenario: Recursive delete of an absolute path below root is allowed
    When the agent tries to validate command "rm -rf /tmp/build"
    Then the validation should be ok

  Scenario: Recursive delete of the root wildcard is blocked
    When the agent tries to validate command "rm -rf /*"
    Then the validation should be an error
    And the error should mention "rm-root"

  Scenario: Writing to a raw block device is blocked
    When the agent tries to validate command "cat image.iso > /dev/sdb"
    Then the validation should be an error
    And the error should mention "block-device-write"

  Scenario: Reading from a raw block device is allowed
    When the agent tries to validate command "head -c 512 < /dev/sda"
    Then the validation should be ok

  Scenario: Subagent sandbox allows outside paths
    Given a sandboxed workspace at "/tmp/quecto-test"
    When the subagent sandbox validates path "/etc/passwd"
    Then the validation should be ok
