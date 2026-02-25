@done
Feature: Non-Blocking Coordinator Integration
  As the main agent
  I want the coding coordinator to run as an async background task
  So that the agent loop remains responsive for user conversation while jobs execute

  The coordinator runs as a tokio::spawn background task. Communication
  between the main agent's tool loop and the coordinator uses async channels.
  The coding_job tool sends commands to the coordinator and receives
  responses without blocking the agent's LLM call loop.

  # --- Async channel architecture ---

  Scenario: Coordinator runs as a background tokio task
    Given a nonblocking coordinator bus with buffer size 16
    When the coordinator bus is started
    Then the coordinator bus should have a command sender
    And the coordinator bus should have a command receiver

  Scenario: coding_job tool sends commands via channel and receives response
    Given a nonblocking coordinator bus with a background handler
    When a command with action "run" is sent via the coordinator bus
    Then the command should arrive at the coordinator handler
    And a response should be sent back via the oneshot channel
    And the caller should receive the response without blocking

  Scenario: coding_job status query does not block on running workers
    Given a nonblocking coordinator bus with a background handler
    And a status query is dispatched for job "job_000001"
    When the handler processes the status query
    Then the status response should be returned from coordinator state
    And the response should arrive promptly without waiting for workers

  # --- Agent loop not blocked ---

  Scenario: Agent processes user message while coding job runs in background
    Given a nonblocking coordinator bus with a background handler
    And a coding command is being processed by the handler
    When a second independent command is sent via a cloned sender
    Then the second command should be buffered in the channel
    And the agent loop sender should not block

  Scenario: Agent can call non-coding tools while coordinator is busy
    Given a nonblocking coordinator bus with buffer size 16
    And a command is in flight to the coordinator
    When the agent performs an independent operation
    Then the independent operation should complete without waiting
    And the in-flight command should remain pending

  Scenario: Multiple status queries do not block each other
    Given a nonblocking coordinator bus with a background handler
    When 3 status queries are sent concurrently
    Then all 3 responses should be received
    And no query should wait for another query's response

  # --- Event delivery ---

  Scenario: Coordinator response delivers completion info
    Given a nonblocking coordinator bus with a background handler
    When a command with action "status" is sent and the handler replies with state "succeeded"
    Then the response body should contain the succeeded state
    And the response should indicate success

  Scenario: Coordinator buffers commands when handler is slow
    Given a nonblocking coordinator bus with buffer size 5
    When 5 commands are sent before the handler processes any
    Then all 5 commands should be buffered in the channel
    And none should be lost

  # --- Channel backpressure ---

  Scenario: Full command channel applies backpressure
    Given a nonblocking coordinator bus with buffer size 2
    When 2 commands fill the channel buffer
    Then a third command via try_send should fail with channel full
    And the first 2 commands should still be receivable

  Scenario: Slow consumer does not cause coordinator deadlock
    Given a nonblocking coordinator bus with buffer size 4
    When 4 commands are sent and the handler drains them one by one
    Then all 4 responses should be received in order
    And the coordinator should not deadlock

  # --- Graceful shutdown ---

  Scenario: Coordinator handle returns None after all senders dropped
    Given a nonblocking coordinator bus with a coordinator handle
    When all command senders are dropped
    Then the coordinator handle recv should return None
    And the coordinator loop should exit cleanly

  Scenario: Channel receiver returns None after bus is dropped
    Given a nonblocking coordinator bus with a background handler
    When the coordinator bus and all senders are dropped
    Then the handler's recv should return None
    And the tool should detect that the coordinator is unavailable

  # --- Error isolation ---

  Scenario: Dropped reply channel signals caller error
    Given a nonblocking coordinator bus with a background handler
    When a command is sent but the handler drops the reply_tx without responding
    Then the caller's oneshot recv should return an error
    And the caller should treat it as a coordinator failure

  Scenario: Handler continues after one command fails
    Given a nonblocking coordinator bus with a background handler
    When the first command's reply_tx is dropped and a second command is processed normally
    Then the second command should receive a valid response
    And the handler should remain operational

  # --- Composition root wiring ---

  Scenario: CLI dispatch mode is synchronous per-session
    Given the dispatch mode is determined for CLI agent
    Then the dispatch mode should be Synchronous
    And no background coordinator bus should be needed

  Scenario: Gateway dispatch mode is background shared
    Given the dispatch mode is determined for gateway
    Then the dispatch mode should be Background
    And commands should flow through the coordinator bus
