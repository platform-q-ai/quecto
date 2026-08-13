@tui
Feature: Per-connection state bundled into the tab's connection (N=1)
  As a TUI user running multiple agent tabs
  I want each tab's connection to own its connection-scoped state,
  with every correlation id it mints namespaced to that connection
  So that broadcast responses can never land on the wrong tab's
  pending latches

  @wip @issue-1463
  Scenario: Solicited transcript fetches carry their connection's namespace
    Given a fresh headless TUI harness
    When a resume response arrives on the master connection
    Then the solicited transcript fetch it mints should carry the master connection's namespace

  @wip @issue-1463
  Scenario: A response bearing another connection's id does not resolve this tab's pending fetch
    Given a fresh headless TUI harness
    And a resume response arrives on the master connection
    When a transcript response arrives bearing another connection's id
    Then this tab's pending transcript fetch should remain unresolved
