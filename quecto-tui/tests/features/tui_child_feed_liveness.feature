@child-feed-liveness
Feature: Child feed liveness under command backpressure
  As a TUI operator watching a spawned child agent
  I want the child feed's sync path to survive writer-queue pressure
  So that the child's main-panel feed never freezes while the parent is busy

  # The child-progress-freeze fix (2026-07-29), TUI half:
  # 1. `Command::Sync` is feed-liveness traffic and bypasses the #1238/#1305
  #    background reserve (it was refused exactly when the parent was busy).
  # 2. A refused Sync send must not be recorded as in-flight — that phantom
  #    sync stranded the feed until the parent went idle.

  @done
  Scenario: Sync bypasses the background reserve on a pressured queue
    Given a production writer queue filled to the background reserve
    Then a further background command should be refused with backpressure
    And a sync command should still be accepted

  @done
  Scenario: A truly full queue still refuses sync
    Given a production writer queue completely full of sync commands
    Then a further sync command should be refused with backpressure

  @done
  Scenario: A refused sync is not recorded as in-flight
    Given a tracked child feed whose command channel is full
    When a ledger advance hint arrives for that child
    Then no sync should be recorded as in-flight

  @done
  Scenario: The next ledger hint retries after a refused sync
    Given a tracked child feed whose command channel is full
    And a ledger advance hint arrives for that child
    When the channel frees a slot and a newer ledger hint arrives
    Then a sync command should be enqueued for the child
    And the newer revision should be recorded as in-flight
