@done
Feature: Configuration-only REPL
  As a Quecto user
  I want the no-argument interactive shell limited to setup and configuration
  So that agent operation remains in dedicated agent interfaces

  Scenario: Help clearly describes the limited REPL scope
    Given a temp base directory
    When I start quecto in REPL mode
    And I type "help"
    And I type "exit"
    Then stdout should contain "Setup and configuration commands"
    And stdout should contain "auth login"
    And stdout should contain "use `quecto agent` or `quecto-tui`"
    And stdout should not contain "/spawn"
    And stdout should not contain "/clear"

  Scenario: Agent prompts and commands are unavailable
    Given a temp base directory
    When I start quecto in REPL mode
    And I type "tell me a joke"
    And I type "/spawn child"
    And I type "/agent list"
    And I type "exit"
    Then stdout should contain "Unsupported REPL command"
    And stdout should contain "only for login, setup, and configuration"
    And the exit code should be 0

  Scenario: Configuration status remains available
    Given a temp base directory
    When I start quecto in REPL mode
    And I type "status"
    And I type "exit"
    Then stdout should contain "quecto Status"
    And stdout should contain "Config:"
