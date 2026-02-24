@pending
Feature: GitHub Publish Boundary
  As the coding runtime coordinator
  I want to manage all GitHub/PR operations under policy control
  So that external side effects are safe, auditable, and coordinator-only

  Workers produce local artifacts (patches, commits, branches). The main agent
  decides when to publish. The coordinator executes GitHub actions from approved
  intent, enforcing safety gates (no force-push, protected branch awareness,
  credential scoping).

  Background:
    Given a coding coordinator with GitHub policy:
      | side_effects_owner       | coordinator |
      | force_push_default       | deny        |
      | destructive_reset_default| deny        |
    And a coding job in state "succeeded" with artifacts

  # --- Branch push ---

  Scenario: Coordinator pushes job branch on main-agent request
    When the main agent requests publish action "push_branch" for the job
    Then a "publish.request" event should be emitted with action "push_branch"
    And the coordinator should push the job branch to remote
    And a "publish.result" event should be emitted with ok true

  Scenario: Coordinator denies force-push by default
    When the main agent requests publish action "push_branch" with force true
    Then a "publish.result" event should be emitted with ok false
    And the error should indicate force-push is denied by policy

  # --- PR creation ---

  Scenario: Coordinator creates a pull request from job artifacts
    When the main agent requests publish action "create_pr" with:
      | title | Add parser unit tests       |
      | base  | main                        |
      | head  | quecto/job/job_abc123       |
      | body  | Adds comprehensive parser tests |
    Then a "publish.request" event should be emitted with action "create_pr"
    And a "publish.result" event should be emitted with ok true
    And the result should include pr_number and url

  Scenario: Coordinator updates an existing pull request
    Given a PR already exists for the job branch
    When the main agent requests publish action "update_pr" with updated body
    Then a "publish.result" event should be emitted with ok true

  # --- Review and labels ---

  Scenario: Coordinator requests reviewers on a pull request
    Given a PR exists for the job branch
    When the main agent requests publish action "request_review" with reviewers ["alice", "bob"]
    Then a "publish.result" event should be emitted with ok true

  Scenario: Coordinator adds labels to a pull request
    Given a PR exists for the job branch
    When the main agent requests publish action "add_labels" with labels ["automated", "needs-review"]
    Then a "publish.result" event should be emitted with ok true

  # --- Status and checks ---

  Scenario: Coordinator fetches PR status and reports to main agent
    Given a PR exists for the job branch
    When the main agent queries the PR status
    Then the coordinator should return the check status and review state
    And the main agent should receive a decision-ready summary

  # --- Safety gates ---

  Scenario: Coordinator blocks push to protected branch
    When the main agent requests publish action "push_branch" to branch "main"
    Then a "publish.result" event should be emitted with ok false
    And the error should indicate the branch is protected

  Scenario: Coordinator blocks destructive reset by default
    When the main agent requests a destructive git reset on the remote
    Then the coordinator should deny the operation
    And the error should reference the destructive_reset_default policy

  # --- Worker isolation from GitHub ---

  Scenario: Worker cannot emit publish events directly
    Given a coding job in state "running"
    When the worker attempts to emit a "publish.request" event
    Then the coordinator should reject the event
    And the worker should receive an error indicating publish is coordinator-only

  # --- Credential scoping ---

  Scenario: GitHub credentials are scoped to coordinator operations only
    Given a coding job in state "running"
    Then the worker process should not have GitHub credentials in its environment
    And only the coordinator should hold GitHub API tokens

  # --- Audit trail ---

  Scenario: All publish events are persisted in the event log
    When the coordinator creates a PR for the job
    Then the event log should contain "publish.request" and "publish.result" events
    And credential values should be redacted in event payloads
