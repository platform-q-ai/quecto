@done
Feature: Worker Entrypoint Subcommand
  As the nsjail coding runtime
  I need a `quecto worker` subcommand
  So that the coordinator can launch a coding worker inside the sandbox

  The worker entrypoint parses CLI arguments, validates the job
  directory, builds the worker tool registry and event emitter, and
  runs the coding agent loop. In BDD tests we exercise the argument
  parsing, validation, and initial wiring without requiring a real
  LLM provider.

  # --- Argument parsing ---

  Scenario: Worker accepts required flags
    When I run quecto worker with flags:
      | flag       | value      |
      | --run-id   | run-abc    |
      | --job-id   | job-123    |
      | --job-dir  | /tmp/test  |
      | --goal     | fix bug    |
    Then the worker args should parse successfully
    And the parsed run_id should be "run-abc"
    And the parsed job_id should be "job-123"
    And the parsed goal should be "fix bug"

  Scenario: Worker rejects missing run-id
    When I run quecto worker with flags:
      | flag       | value      |
      | --job-id   | job-123    |
      | --job-dir  | /tmp/test  |
      | --goal     | fix bug    |
    Then the worker args should fail with "run-id"

  Scenario: Worker rejects missing job-id
    When I run quecto worker with flags:
      | flag       | value      |
      | --run-id   | run-abc    |
      | --job-dir  | /tmp/test  |
      | --goal     | fix bug    |
    Then the worker args should fail with "job-id"

  Scenario: Worker rejects missing job-dir
    When I run quecto worker with flags:
      | flag       | value      |
      | --run-id   | run-abc    |
      | --job-id   | job-123    |
      | --goal     | fix bug    |
    Then the worker args should fail with "job-dir"

  Scenario: Worker rejects missing goal
    When I run quecto worker with flags:
      | flag       | value      |
      | --run-id   | run-abc    |
      | --job-id   | job-123    |
      | --job-dir  | /tmp/test  |
    Then the worker args should fail with "goal"

  Scenario: Worker accepts optional model override
    When I run quecto worker with flags:
      | flag       | value          |
      | --run-id   | run-abc        |
      | --job-id   | job-123        |
      | --job-dir  | /tmp/test      |
      | --goal     | fix bug        |
      | --model    | gpt-4o         |
    Then the worker args should parse successfully
    And the parsed model should be "gpt-4o"

  Scenario: Worker defaults model to None when not specified
    When I run quecto worker with flags:
      | flag       | value      |
      | --run-id   | run-abc    |
      | --job-id   | job-123    |
      | --job-dir  | /tmp/test  |
      | --goal     | fix bug    |
    Then the worker args should parse successfully
    And the parsed model should be empty

  # --- Job directory validation ---

  Scenario: Worker validates that job directory exists
    Given a temporary job directory with files:
      | path        | content       |
      | src/main.rs | fn main() {}  |
    When I validate the worker job directory
    Then the worker job directory validation should succeed

  Scenario: Worker rejects non-existent job directory
    When I validate a non-existent worker job directory "/tmp/nonexistent-quecto-test-dir"
    Then the worker job directory validation should fail with "does not exist"

  # --- Tool registry wiring ---

  Scenario: Worker builds a tool registry with coding tools
    Given a temporary job directory with files:
      | path        | content       |
      | src/main.rs | fn main() {}  |
    When I build the worker tool registry for the job directory
    Then the built worker registry should contain "worker_edit"
    And the built worker registry should contain "worker_grep"
    And the built worker registry should contain "worker_find"
    And the built worker registry should contain "worker_read"

  # --- Event emitter wiring ---

  Scenario: Worker creates an event emitter with correct run and job IDs
    When I create a worker event emitter for run "run-abc" and job "job-123"
    Then the worker emitter should emit events with run_id "run-abc"
    And the worker emitter should emit events with job_id "job-123"

  # --- CLI dispatch ---

  Scenario: quecto worker is recognized as a valid command
    When I run quecto with args "worker --help"
    Then the worker cli exit code should not indicate unknown command
    And the worker cli output should not contain "Unknown command"

  Scenario: quecto help includes worker command
    When I run quecto with args "help"
    Then the worker cli output should contain "worker"

  # --- Exit behavior ---

  Scenario: Worker rejects unknown flags
    When I run quecto worker with flags:
      | flag          | value      |
      | --run-id      | run-abc    |
      | --job-id      | job-123    |
      | --job-dir     | /tmp/test  |
      | --goal        | fix bug    |
      | --bad-flag    | oops       |
    Then the worker args should fail with "bad-flag"

  Scenario: Worker accepts max-iterations flag
    When I run quecto worker with flags:
      | flag             | value      |
      | --run-id         | run-abc    |
      | --job-id         | job-123    |
      | --job-dir        | /tmp/test  |
      | --goal           | fix bug    |
      | --max-iterations | 50         |
    Then the worker args should parse successfully
    And the parsed max_iterations should be 50
