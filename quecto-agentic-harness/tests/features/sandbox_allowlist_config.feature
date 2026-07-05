@sandbox @done
Feature: Sandbox command allowlist configuration

  As a user running Quecto in an untrusted environment
  I want to configure a command allowlist in my agent defaults
  So that only explicitly permitted commands can be executed, while the denylist remains enforced

  Background:
    Given a workspace at "/tmp/quecto-test"

  Scenario: Allowlist is off by default
    Given the agent has no command allowlist configured
    When the agent validates the command "cat /etc/passwd"
    Then the command should be permitted

  Scenario: Allowlist restricts commands to listed tokens
    Given the agent has command allowlist "echo, ls"
    When the agent validates the command "cat /etc/passwd"
    Then the command should be rejected with "not in allowlist"

  Scenario: Allowlist permits listed commands
    Given the agent has command allowlist "echo, ls"
    When the agent validates the command "echo hello"
    Then the command should be permitted

  Scenario: Denylist still blocks dangerous commands even when allowlisted
    Given the agent has command allowlist "rm, echo"
    When the agent validates the command "rm -rf /"
    Then the command should be rejected with "dangerous pattern"
