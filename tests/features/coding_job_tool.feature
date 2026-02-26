@done
Feature: Coding Job Tool
  As the LLM agent
  I want a coding_job tool that wraps the CodingCoordinator
  So that I can manage coding jobs through the standard tool interface

  The coding_job tool is the bridge between the agent loop and the coding
  coordinator. It accepts JSON with an "action" field and dispatches to the
  appropriate coordinator method, returning JSON responses.

  Background:
    Given a coding_job tool with a mock coordinator

  # --- Tool definition ---

  Scenario: Tool definition has correct name and schema
    Then the coding_job tool name should be "coding_job"
    And the coding_job tool description should mention coding jobs
    And the coding_job tool schema should require an "action" field

  # --- Run action ---

  Scenario: Run action creates a new coding job
    When I execute the coding_job tool with run goal "Add unit tests" repo "test-repo" ref "main"
    Then the coding_job result should not be an error
    And the coding_job result should contain "run_id"
    And the coding_job result should contain "job_id"
    And the coding_job result should contain "queued"

  Scenario: Run action rejects invalid repo
    When I execute the coding_job tool with run goal "Fix bug" repo "nonexistent-repo" ref "main"
    Then the coding_job result should be an error
    And the coding_job result should contain "invalid_repo"

  Scenario: Run action rejects invalid base ref
    When I execute the coding_job tool with run goal "Fix bug" repo "test-repo" ref "nonexistent"
    Then the coding_job result should be an error
    And the coding_job result should contain "invalid_base_ref"

  Scenario: Run action passes optional priority
    When I execute the coding_job tool with run priority "high"
    Then the coding_job result should not be an error
    And the coding_job result should contain "job_id"

  # --- Status action ---

  Scenario: Status action returns job state
    Given a coding job exists via the tool
    When I execute the coding_job tool with status for current job
    Then the coding_job result should not be an error
    And the coding_job result should contain "queued"
    And the coding_job result should contain "job_id"

  Scenario: Status action returns not_found for unknown job
    When I execute the coding_job tool with status for job "nonexistent_job"
    Then the coding_job result should be an error
    And the coding_job result should contain "not_found"

  # --- Cancel action ---

  Scenario: Cancel action cancels a queued job
    Given a coding job exists via the tool
    When I execute the coding_job tool with cancel for current job
    Then the coding_job result should not be an error
    And the coding_job result should contain "canceled"

  Scenario: Cancel action returns not_found for unknown job
    When I execute the coding_job tool with cancel for job "nonexistent_job"
    Then the coding_job result should be an error
    And the coding_job result should contain "not_found"

  # --- Cleanup action ---

  Scenario: Cleanup action on terminal job succeeds
    Given a coding job exists via the tool in state "canceled"
    When I execute the coding_job tool with cleanup for current job
    Then the coding_job result should not be an error
    And the coding_job result should contain "cleaned"

  Scenario: Cleanup action on running job is rejected
    Given a coding job exists via the tool in state "running"
    When I execute the coding_job tool with cleanup for current job
    Then the coding_job result should be an error
    And the coding_job result should contain "job_not_terminal"

  # --- List action ---

  Scenario: List action returns all jobs
    Given 3 coding jobs exist via the tool
    When I execute the coding_job tool with action "list"
    Then the coding_job result should not be an error
    And the coding_job result should contain "jobs"

  Scenario: List action with state filter
    Given 3 coding jobs exist via the tool
    When I execute the coding_job tool with list filter "queued"
    Then the coding_job result should not be an error

  # --- Error handling ---

  Scenario: Unknown action returns an error
    When I execute the coding_job tool with action "unknown_action"
    Then the coding_job result should be an error
    And the coding_job result should contain "unknown action"

  Scenario: Invalid JSON returns an error
    When I execute the coding_job tool with raw input "not valid json"
    Then the coding_job result should be an error
    And the coding_job result should contain "invalid JSON"

  Scenario: Missing action field returns an error
    When I execute the coding_job tool with raw input "{}"
    Then the coding_job result should be an error
    And the coding_job result should contain "action"
