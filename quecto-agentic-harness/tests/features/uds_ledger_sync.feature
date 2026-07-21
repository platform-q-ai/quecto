Feature: Ledger sync over UDS
  As a UDS client watching an agent transcript
  I want to pull committed transcript changes by ledger version
  So that I can converge after missed live notifications or while the agent is busy

  @pending @issue-1194 @ledger-sync
  Scenario: Sync returns committed messages while an agent is busy
    Given a UDS session has committed transcript messages
    And the agent is busy handling a prompt
    When a client syncs from the previous ledger revision
    Then the client should receive the committed transcript messages
    And the client should know whether it is caught up

  @pending @issue-1194 @ledger-sync
  Scenario: Sync continues large deltas without duplicating history pages
    Given a UDS session has more committed transcript changes than fit in one sync response
    When a client syncs from an earlier ledger revision
    Then the client should receive a bounded chronological delta
    And the client should receive a sync revision to continue from

  @pending @issue-1194 @ledger-sync
  Scenario: Sync requests outside retained history require resynchronisation
    Given a UDS session no longer retains the requested ledger revision
    When a client syncs from that earlier revision
    Then the client should be told to resynchronise
    And the client should receive a bounded newest transcript slice

  @pending @issue-1194 @ledger-sync
  Scenario: History replacement changes the sync epoch
    Given a UDS session has a known sync epoch
    When the session history is replaced
    Then the client should observe the next sync epoch

  @pending @issue-1194 @ledger-sync
  Scenario: History replacement does not over-advance the ledger revision
    Given a UDS session has a known ledger revision
    When the session history is replaced
    Then the replacement should not advance the ledger revision more than once

  @pending @issue-1194 @ledger-sync
  Scenario: Ledger advance hints are emitted for new committed messages
    Given a UDS client is attached to a session
    When the session commits new transcript messages
    Then the client should receive a ledger advanced hint

  @pending @issue-1194 @ledger-sync
  Scenario: Ledger advance hints are not emitted for unchanged committed messages
    Given a UDS client is attached to a session
    When the committed transcript messages are unchanged
    Then the client should not receive a ledger advanced hint
