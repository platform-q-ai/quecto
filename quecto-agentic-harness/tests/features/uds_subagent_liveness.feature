@subagent-liveness
Feature: Sub-agent liveness while the parent is busy
  As a TUI operator watching a parent agent that spawned children
  I want child progress to keep flowing while the parent turn is in flight
  So that the left-panel roster and child feeds never freeze until the parent goes idle

  # The child-progress-freeze fix (2026-07-29). Two harness mechanisms:
  # 1. Mid-turn `TurnCompleted` publishes must EMIT their `ledger_advanced`
  #    hints — the TUI feed re-syncs only on that event (#1283 regression).
  # 2. `get_subagents` and child-targeted `sync` must be answered from the
  #    connection's reader task, never queued behind the serial dispatch loop
  #    that a parent turn occupies end-to-end.

  @done
  Scenario: A completed inner turn emits a ledger advance hint mid-turn
    Given a prompt turn is in flight against a fresh conversation snapshot
    When an inner turn completes with new messages
    Then a ledger advance hint should be emitted before the prompt finishes

  @done
  Scenario: An unchanged inner-turn publish emits no ledger advance hint
    Given a prompt turn is in flight against a fresh conversation snapshot
    When the same inner turn completes twice
    Then exactly one ledger advance hint should be emitted

  @done
  Scenario: Streaming tokens do not advance the ledger
    Given a prompt turn is in flight against a fresh conversation snapshot
    When only streaming tokens arrive
    Then no ledger advance hint should be emitted

  @done
  Scenario: A busy parent answers get_subagents from the reader task
    Given the parent dispatch loop is occupied by a turn
    When a client sends get_subagents with correlation id "gs-live"
    Then the command should be handled off the dispatch loop
    And the response should carry correlation id "gs-live" and a snapshot marker

  @done
  Scenario: A busy parent answers a child-targeted sync from the reader task
    Given the parent dispatch loop is occupied by a turn
    When a client sends a sync addressed to child "worker-1"
    Then the command should be handled off the dispatch loop
    And the client should receive a correlated sync response

  @done
  Scenario: A parent-scoped sync is left to the parent ledger fast path
    Given the parent dispatch loop is occupied by a turn
    When a client sends a sync without a child address
    Then the liveness interceptor should leave the command alone

  # The TUI child feed connects DIRECTLY to the child's socket and sends a
  # plain sync (no agent_id). That path is served by the child-local ledger
  # fast path (#1197) even while the child's own dispatch loop is occupied —
  # pinned end-to-end through the full reader dispatch (PR #1307 review).
  @done
  Scenario: A busy child answers its direct feed sync from the ledger fast path
    Given the child dispatch loop is occupied by a turn
    When its feed client sends a plain sync for the committed ledger
    Then the sync should be answered inline without queuing behind the turn
    And the sync response should carry the committed messages
