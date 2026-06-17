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
    And the quecto-tui infrastructure source should not import application or interface layers
    And the quecto-tui infrastructure layer should own runtime adapters

  Scenario: quecto-tui production files live inside Clean Architecture layers
    Then every quecto-tui production Rust file should be under a Clean Architecture layer

  Scenario: quecto-tui crate roots stay as thin composition entrypoints
    Then the quecto-tui library root should expose only Clean Architecture layers
    And the quecto-tui binary root should delegate to the interface layer

  Scenario: quecto-tui architecture is enforced by the same architecture test target as quecto
    Then the architecture test target should enforce quecto-tui Clean Architecture layers
    And the architecture test target should enforce quecto-tui runtime I/O boundaries
    And the architecture test target should enforce quecto-tui root file placement

  Scenario: TUI standards are executable through BDD
    Then the BDD runner should execute TUI scenarios tagged wip or done
    And the TUI architecture feature should not contain pending scenarios

  Scenario: TUI scrollback remains stable while an assistant response streams
    Given a quecto-tui chat view is scrolled into history
    When streaming assistant content extends the conversation
    Then the quecto-tui chat viewport should keep showing the same historical lines

  Scenario: TUI scrollback stops at a full page instead of blanking while streaming
    Given a quecto-tui chat view is scrolled beyond the oldest full page
    When streaming assistant content extends the conversation
    Then the quecto-tui chat viewport should still show a full historical page

  Scenario: TUI slash autocomplete exposes session resume
    Then the quecto-tui slash autocomplete should include command "resume"

  Scenario: TUI rejects unknown slash commands locally
    Then quecto-tui should reject unknown slash commands before sending a prompt

  Scenario: TUI can list and resume persisted CLI sessions over UDS
    Then the UDS protocol should support listing sessions
    And the UDS protocol should support resuming a session

  Scenario: TUI resume selector is readable above chat history
    Then the quecto-tui resume selector should render with an opaque border

  Scenario: TUI workflow bar shows stage status and hotkey tips
    Then the quecto-tui workflow bar should include stage status and hotkey tips

  Scenario: TUI shows a Pi-style workflow widget above the editor
    Then the quecto-tui workflow widget should render as a full-width yellow status bar above the editor

  Scenario: TUI workflow panel matches the Pi checklist in read-only mode
    Then the quecto-tui workflow panel should render the Pi workflow checklist in read-only mode
