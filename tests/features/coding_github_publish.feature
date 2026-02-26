@done
Feature: GitHub Publish Boundary
  As the coding runtime coordinator
  I want to manage all GitHub/PR operations under policy control
  So that external side effects are safe, auditable, and coordinator-only

  Workers produce local artifacts (patches, commits, branches). The main agent
  decides when to publish. The coordinator executes GitHub actions from approved
  intent, enforcing safety gates (no force-push, protected branch awareness,
  credential scoping).

  # --- Succeeded-job publish operations ---
  # These scenarios all operate on a succeeded job with artifacts and a standard
  # GitHub policy (coordinator-owned side effects, deny force-push, deny destructive reset).

  Rule: Publish operations on succeeded jobs

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

    Scenario: Coordinator fetches PR status and returns decision-ready summary
      Given a PR exists for the job branch
      When the main agent requests publish action "get_pr_status"
      Then a "publish.request" event should be emitted with action "get_pr_status"
      And a "publish.result" event should be emitted with ok true
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

    # --- Audit trail ---

    Scenario: All publish events are persisted in the event log
      When the coordinator creates a PR for the job
      Then the event log should contain "publish.request" and "publish.result" events
      And credential values should be redacted in event payloads

    # --- Contract fidelity ---

    Scenario: Publish result always includes the action that was requested
      When the main agent requests publish action "create_pr" with valid parameters
      Then the "publish.result" event should include action "create_pr"
      And the action should match the original publish.request action

    # --- Repo allowlist ---

    Scenario: Coordinator denies publish to non-allowlisted repo
      Given a coding coordinator with GitHub repo allowlist ["org/approved-repo"]
      When the main agent requests publish action "push_branch" for repo "org/forbidden-repo"
      Then a "publish.result" event should be emitted with ok false
      And the error should indicate the repo is not in the allowlist

    # --- Network failure handling ---

    Scenario: Coordinator handles GitHub API timeout gracefully
      When the main agent requests publish action "create_pr"
      And the GitHub API request times out
      Then a "publish.result" event should be emitted with ok false
      And the error should indicate a network timeout

    Scenario: Coordinator handles GitHub API rate limit
      When the main agent requests publish action "push_branch"
      And the GitHub API returns a 429 rate limit response
      Then a "publish.result" event should be emitted with ok false
      And the error should indicate rate limiting

    # --- Protected branch detection ---

    Scenario: Coordinator detects protected branch from GitHub API
      When the main agent requests publish action "push_branch" to branch "main"
      And the GitHub API reports "main" as a protected branch
      Then a "publish.result" event should be emitted with ok false
      And the error should reference branch protection rules

  # --- Non-succeeded-job scenarios ---
  # These scenarios test publish behavior on jobs in states other than "succeeded",
  # or cross-cutting concerns that don't depend on the succeeded Background.

  Rule: Publish operations on non-succeeded or cross-cutting jobs

    Background:
      Given a coding coordinator with GitHub policy:
        | side_effects_owner       | coordinator |
        | force_push_default       | deny        |
        | destructive_reset_default| deny        |

    # --- Worker isolation from GitHub ---

    Scenario: Worker cannot emit publish events directly
      Given a coding job in state "running"
      When the worker attempts to emit a "publish.request" event
      Then the coordinator should reject the publish event
      And the worker should receive an error indicating publish is coordinator-only

    # --- Credential scoping ---

    Scenario: GitHub credentials are scoped to coordinator operations only
      Given a coding job in state "running"
      Then the worker process should not have GitHub credentials in its environment
      And only the coordinator should hold GitHub API tokens

    # --- Read-only queries on non-succeeded jobs ---

    Scenario: get_pr_status is allowed on non-succeeded jobs
      Given a coding job in state "running"
      When the main agent requests publish action "get_pr_status"
      Then a "publish.result" event should be emitted with ok true
      And the result should include the current PR check and review state

    # --- Job state validation ---

    Scenario: Coordinator rejects publish for a failed job
      Given a coding job in state "failed"
      When the main agent requests publish action "create_pr" for the failed job
      Then a "publish.result" event should be emitted with ok false
      And the error should indicate the job did not succeed

    Scenario: Coordinator rejects publish for a canceled job
      Given a coding job in state "canceled"
      When the main agent requests publish action "push_branch" for the canceled job
      Then a "publish.result" event should be emitted with ok false
      And the error should indicate the job was canceled

    # --- Multi-job aggregation (future — MVP is 1:1 run-to-job) ---

    @pending @future
    Scenario: Coordinator aggregates artifacts from multiple jobs into single PR
      Given two coding jobs have completed successfully for the same repo
      When the main agent requests a combined PR for both jobs
      Then the coordinator should merge the branches or patches
      And a single "publish.result" event should indicate the combined PR
