@tui
Feature: Per-connection state bundled into Connection (N=1)
  As the multi-session TUI epic #1467
  I want connection-scoped state moved off App into the per-tab Connection
  structures behind active_conn()/active_conn_mut() accessors (issue #1463),
  with every minted correlation id namespaced to its connection
  So that broadcast responses can never land on the wrong tab's pending
  latches while N=1 frames stay byte-identical

  @wip @issue-1463
  Scenario: Solicited transcript-fetch ids carry the connection namespace
    Given a fresh headless TUI harness
    When a resume response mints a solicited transcript fetch id
    Then the minted correlation id should begin with the master connection namespace

  @wip @issue-1463
  Scenario: Master frames stay byte-identical through the per-connection move
    Given a baseline frame from a master token handled directly
    And a fresh headless TUI harness
    When the same master token arrives through the master connection feed
    Then the frame should be identical to the direct-handling baseline
