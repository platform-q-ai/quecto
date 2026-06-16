@wip @tui
Feature: TUI Clean Architecture and executable BDD enforcement
  The quecto-tui crate must be shaped to the same architectural standards as
  the main quecto crate. Its production source is organised into Clean
  Architecture layers and the BDD suite executes TUI standards instead of only
  keeping them as pending documentation.

  Scenario: quecto-tui exposes Clean Architecture layers
    Then the quecto-tui source tree should contain layer "domain"
    And the quecto-tui source tree should contain layer "application"
    And the quecto-tui source tree should contain layer "infrastructure"
    And the quecto-tui source tree should contain layer "interface"

  Scenario: quecto-tui inner layers remain free of runtime I/O
    Then the quecto-tui domain source should not contain runtime I/O patterns
    And the quecto-tui application source should not contain runtime I/O patterns

  Scenario: quecto-tui layer dependencies point inward
    Then the quecto-tui domain source should not import outer layers
    And the quecto-tui application source should not import infrastructure or interface layers
    And the quecto-tui infrastructure source should not import interface layers

  Scenario: TUI standards are executable through BDD
    Then the BDD runner should execute TUI scenarios tagged wip or done
    And the TUI architecture feature should not contain pending scenarios
