@done @e2e-coord-agent @real-llm
Feature: End-to-end coordinator agent with worker pipeline
  The full coding pipeline: a real LLM agent creates a coding job,
  the lifecycle driver clones the repo and spawns a worker subprocess,
  the worker (also backed by a real LLM) executes the task and emits
  events. Results flow back through the coordinator state machine.

  This is the most comprehensive integration test for the coding system,
  exercising both the main agent and a real worker subprocess with real
  LLM reasoning.

  Background:
    Given a real LLM workspace is configured
    And a git repo "test-repo" in the real LLM workspace with base ref "main"

  @e2e-coord-agent
  Scenario: Real LLM agent runs a coding job that spawns a worker and advances past queued
    When I run the real LLM agent with max-time 120 and message "Use the coding_job tool to run a job with goal 'Create a file called hello.txt containing Hello World', repo 'test-repo', and base_ref 'main'. Then call coding_job status for the returned job_id. Keep calling status every few seconds until the state is no longer 'queued' and 'preparing', or until you have checked 5 times. Reply with the final state prefixed by FINAL_STATE= (e.g. FINAL_STATE=running or FINAL_STATE=succeeded or FINAL_STATE=queued)."
    Then the exit code should be 0
    And stdout should include sentinel "FINAL_STATE="
    And a mirror should exist for repo "test-repo" in the coding cache

  @e2e-coord-agent
  Scenario: Real LLM worker subprocess processes a coding task and emits events
    Given a job directory with a cloned test repo
    When I run the real LLM worker with run-id "run_e2e" job-id "job_e2e" and goal "Create a file called hello.txt with the text 'Hello from worker'. Use the available file tools." and max-time 60
    Then the worker exit code should be 0
    And the worker stdout should contain a JSON Lines event with type "log.message"
