@wip
Feature: End-to-end coding job lifecycle
  The main quecto agent manages the coding coordinator.
  The coordinator manages workers that clone repos, execute coding tasks,
  and report results back through the agent.

  These scenarios test the full pipeline from the LLM issuing a coding_job
  tool call through to the coordinator ticking jobs through their lifecycle,
  workers executing, and results flowing back to the agent.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a test git repo in the e2e workspace

  # --- Gap 1: Lifecycle driver must tick jobs past queued ---
  # After the agent creates a job, the lifecycle driver should advance it.
  # We verify by checking that a mirror and job directory were created.

  @wip
  Scenario: lifecycle driver creates mirror and job directory for a coding job
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"bump version","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                   |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                   |
      | text | Job created. |                                                                          |
    When I run quecto agent -s - --max-time 15 -m "Create a coding job"
    Then the exit code should be 0
    And a mirror should exist for repo "test-repo" in the coding cache

  # --- Gap 2: Multiple jobs can be managed ---

  @wip
  Scenario: agent can create multiple coding jobs
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"task one","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"run","goal":"task two","repo":"test-repo","base_ref":"main"} |
      | text | Created two jobs. |                                                                  |
    When I run quecto agent -s - -m "Create two coding jobs"
    Then the exit code should be 0

  # --- Gap 3: quecto worker subcommand runs the full worker loop ---

  @wip
  Scenario: quecto worker subprocess runs the full agent loop and emits events
    Given a job directory with a cloned test repo
    And a mock LLM that returns text "I created the file"
    When I run quecto worker with run-id "run_001" job-id "job_001" and goal "create hello.txt"
    Then the worker stdout should contain a JSON Lines event with type "log.message"
    And the worker exit code should be 0

  # --- Gap 4: Lifecycle driver clones repo during preparation ---

  @wip
  Scenario: lifecycle driver clones repo for a job
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"test clone","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                 |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                 |
      | text | Job created. |                                                                         |
    When I run quecto agent -s - --max-time 15 -m "Run a job"
    Then the exit code should be 0
    And a job directory should exist in the coding cache

  # --- Gap 5: coding_job tool is registered and runs ---

  @wip
  Scenario: coding_job tool is registered and creates a job via the agent
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"test registration","repo":"test-repo","base_ref":"main"} |
      | text | Done. |                                                                                      |
    When I run quecto agent -s - -m "Create a job"
    Then the exit code should be 0
    And the agent should not have reported tool errors in stderr

  # --- Gap 6: Saved session contains coding_job tool results ---

  @wip
  Scenario: non-ephemeral session captures coding_job tool results
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"session test","repo":"test-repo","base_ref":"main"} |
      | text | Done. |                                                                                  |
    When I run quecto agent -s lifecycle-test -m "Create a job"
    Then the exit code should be 0
    And the saved session "lifecycle-test" should contain a tool result with "job_id"
