@pending
Feature: Coordinator Todo Tracking Per Worker
  As the coding runtime coordinator
  I want to maintain a todo list per coding job
  So that the main agent can track worker progress and make goal-based decisions

  The coordinator owns todo state. Workers propose todo updates through events.
  The coordinator validates transitions and persists them. The main agent reads
  aggregated todo status via the status command.

  Background:
    Given a coding coordinator with a mock worker
    And a coding job in state "running"

  # --- Todo creation ---

  Scenario: Worker creates a todo item
    When the worker emits a "todo.create" event with:
      | todo_id | t1                     |
      | title   | Add parser unit tests  |
      | status  | pending                |
    Then the coordinator should record todo "t1" with status "pending"
    And the job's todo list should contain 1 item

  Scenario: Worker creates a todo with dependencies
    Given the job has todo "t1" with status "completed"
    When the worker emits a "todo.create" event with:
      | todo_id    | t2                   |
      | title      | Run integration tests|
      | status     | pending              |
      | depends_on | ["t1"]               |
    Then todo "t2" should have depends_on containing "t1"

  # --- Todo status transitions ---

  Scenario: Worker updates a todo to in_progress
    Given the job has todo "t1" with status "pending"
    When the worker emits a "todo.update" event for "t1" with status "in_progress"
    Then todo "t1" should have status "in_progress"

  Scenario: Worker completes a todo with result
    Given the job has todo "t1" with status "in_progress"
    When the worker emits a "todo.complete" event for "t1" with result "12 tests added"
    Then todo "t1" should have status "completed"
    And the completion result should be "12 tests added"

  Scenario: Worker completes a todo with artifact references
    Given the job has todo "t1" with status "in_progress"
    When the worker emits a "todo.complete" event for "t1" with artifact_refs ["test.log"]
    Then todo "t1" should have artifact_refs containing "test.log"

  Scenario: Worker blocks a todo with reason
    Given the job has todo "t1" with status "in_progress"
    When the worker emits a "todo.blocked" event for "t1" with reason "failing legacy test"
    Then todo "t1" should have status "blocked"
    And the blocked reason should be "failing legacy test"

  Scenario: Blocked todo includes needs field for main-agent decision
    Given the job has todo "t1" with status "in_progress"
    When the worker emits a "todo.blocked" event for "t1" with:
      | reason | conflicting test expectations |
      | needs  | main-agent decision           |
    Then the blocked event should include needs "main-agent decision"

  # --- Status command includes todos ---

  Scenario: Status response includes current todo list
    Given the job has todos:
      | todo_id | title              | status      |
      | t1      | Write parser tests | completed   |
      | t2      | Fix linter errors  | in_progress |
      | t3      | Run full test suite| pending     |
    When the main agent queries job status
    Then the status response should include 3 todo items
    And each todo should have todo_id, title, and status

  # --- Todo limits ---

  Scenario: Coordinator enforces max todo items per job
    Given the coordinator is configured with max_items_per_job 50
    And the job already has 50 todo items
    When the worker emits a "todo.create" event for a 51st todo
    Then the coordinator should reject the create with an error
    And the job should still have 50 todo items

  # --- Todo events are persisted ---

  Scenario: Todo events appear in the JSONL event log
    When the worker creates and completes a todo
    Then the event log should contain both "todo.create" and "todo.complete" events
    And the events should have correct envelope fields
