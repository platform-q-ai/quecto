@done
Feature: CLI Interface
  As a user
  I want a command-line interface with clear subcommands
  So that I can interact with Quecto from the terminal

  Scenario: No arguments enters REPL mode
    When I run quecto with no arguments
    Then quecto should enter interactive REPL mode

  Scenario: Help subcommand shows usage
    When I run quecto with arguments "help"
    Then the exit code should be 0
    And the output should contain "Usage: quecto [command]"
    And the output should contain "onboard"
    And the output should contain "agent"
    And the output should contain "gateway"
    And the output should contain "status"
    And the output should contain "auth"
    And the output should contain "cron"
    And the output should contain "skills"
    And the output should contain "version"

  Scenario: Show version
    When I run quecto with arguments "version"
    Then the exit code should be 0
    And the output should match "quecto \d+\.\d+\.\d+"

  Scenario: Show version with --version flag
    When I run quecto with arguments "--version"
    Then the exit code should be 0
    And the output should match "quecto \d+\.\d+\.\d+"

  Scenario: Show version with -v flag
    When I run quecto with arguments "-v"
    Then the exit code should be 0
    And the output should match "quecto \d+\.\d+\.\d+"

  Scenario: Unknown command shows error and help
    When I run quecto with arguments "foobar"
    Then the exit code should be 1
    And the stderr should contain "Unknown command: foobar"
    And the output should contain "Usage: quecto [command]"

  # Agent one-shot mode is covered in agent_cli.feature
  # REPL interactive mode is covered in repl.feature
