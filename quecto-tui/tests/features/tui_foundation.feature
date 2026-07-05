@tui
Feature: TUI Foundation
  The quecto-tui binary is a standalone terminal UI client that communicates
  with a quecto agent process over a Unix domain socket. These scenarios verify
  the Phase 1 foundation: UDS client connectivity, event handling, key parsing,
  component rendering, and differential output.
  These scenarios are specifications for the quecto-tui crate; tested via
  unit tests in quecto-tui/src/. Tagged @tui to exclude from normal BDD runs.

  @tui @done
  Scenario: Command send failures are observable
    Given the TUI command channel is disconnected
    When the TUI tries to send a command to the agent
    Then the TUI should show an error notification for the failed command send
    And the send failure should not be handled only through stderr

  @tui @done
  Scenario: DiffRenderer write and flush failures are observable
    Given the TUI renderer output fails while writing or flushing
    When the TUI renders a frame
    Then the DiffRenderer should return the render error instead of ignoring it
    And the TUI should show an error notification for the failed render
