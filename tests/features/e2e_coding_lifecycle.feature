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
  # After the agent creates a job and checks its status, the lifecycle
  # driver should have ticked the job at least to "preparing".
  # If the status is still "queued", the lifecycle driver is not running.

  @wip
  Scenario: coding_job status shows job has advanced past queued after lifecycle tick
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"bump version","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                   |
      | text | Checked. |                                                                               |
    When I run quecto agent -s - -m "Create a coding job and check its status"
    Then the exit code should be 0
    And the coding job tool should have returned a status response
    And the coding job status in the tool response should not be "queued"

  # --- Gap 2: Worker subprocess launches and completes ---
  # The lifecycle driver should clone the repo and launch a quecto worker.
  # The worker runs the agent loop, creates files, and exits successfully.

  @wip
  Scenario: coding job lifecycle completes end-to-end with a real worker
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"create hello.txt with content hello world","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                                                |
      | text | Job completed. |                                                                                                      |
    When I run quecto agent -s - --max-time 30 -m "Run a coding job to create hello.txt"
    Then the exit code should be 0
    And the coding job status in the tool response should be "succeeded"

  # --- Gap 3: Worker produces observable file changes ---

  @wip
  Scenario: worker creates files in the job repo directory
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"create a file called output.txt containing test-output","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                                                              |
      | text | Job finished. |                                                                                                                    |
    When I run quecto agent -s - --max-time 30 -m "Run a coding job to create output.txt"
    Then the exit code should be 0
    And the coding job should have created a file "output.txt" in the job repo

  # --- Gap 4: quecto worker subcommand runs the full worker loop ---

  @wip
  Scenario: quecto worker subprocess executes with a real provider and emits events
    Given a job directory with a cloned test repo
    And a mock LLM that returns text "I created the file"
    When I run quecto worker with run-id "run_001" job-id "job_001" and goal "create hello.txt"
    Then the worker stdout should contain a "worker.ready" event
    And the worker stdout should contain a "worker.done" event
    And the worker exit code should be 0

  # --- Gap 5: Multiple jobs can be managed ---

  @wip
  Scenario: agent can create and track multiple coding jobs
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"task one","repo":"test-repo","base_ref":"main"}   |
      | call | coding_job | {"action":"run","goal":"task two","repo":"test-repo","base_ref":"main"}   |
      | call | coding_job | {"action":"list"}                                                         |
      | text | Listed both jobs. |                                                                     |
    When I run quecto agent -s - -m "Create two coding jobs and list them"
    Then the exit code should be 0
    And the coding job tool should have returned a list with 2 jobs

  # --- Gap 6: Lifecycle driver clones repo during preparation ---

  @wip
  Scenario: lifecycle driver clones repo and advances job past queued
    Given the mock LLM returns a tool call sequence:
      | call | coding_job | {"action":"run","goal":"test clone","repo":"test-repo","base_ref":"main"} |
      | call | coding_job | {"action":"status","job_id":"job_000001"}                                  |
      | text | Status checked. |                                                                       |
    When I run quecto agent -s - --max-time 15 -m "Run a job and check status after lifecycle tick"
    Then the exit code should be 0
    And the coding job status in the tool response should be one of "preparing,running,succeeded"
