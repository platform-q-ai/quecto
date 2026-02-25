@done
Feature: Bare Mirror Cache and Per-Job Clone
  As the coding runtime coordinator
  I want to maintain a local bare mirror and clone per-job repos from it
  So that job startup is fast and each job has fully isolated git state

  Per-job clones from a local bare mirror give strong isolation (separate
  .git internals per job) while keeping startup fast. The mirror is updated
  via git fetch with flock-based locking to prevent collisions with
  concurrent clone operations.

  # --- Mirror creation ---

  Scenario: Coordinator creates bare mirror on first job for a repo
    Given a coding coordinator with cache directory "repos/mirrors"
    And no mirror exists for repo "org/my-app"
    When a coding job is submitted for repo "org/my-app"
    Then the coordinator should create a bare mirror at "repos/mirrors/org__my-app.git"
    And the mirror should be a valid bare git repository

  Scenario: Coordinator reuses existing mirror for subsequent jobs
    Given a bare mirror already exists for repo "org/my-app"
    When a coding job is submitted for repo "org/my-app"
    Then the coordinator should not create a new mirror
    And the existing mirror should be used for cloning

  # --- Mirror updates ---

  Scenario: Coordinator fetches latest refs before cloning
    Given a bare mirror exists for repo "org/my-app"
    And the remote has new commits since last fetch
    When a coding job is submitted for repo "org/my-app"
    Then the coordinator should run git fetch on the mirror before cloning
    And the mirror should contain the latest refs

  Scenario: Mirror fetch acquires exclusive flock
    Given a bare mirror exists for repo "org/my-app"
    When the coordinator starts a git fetch on the mirror
    Then an exclusive flock should be held on the mirror directory
    And concurrent clone attempts should wait for the lock to release

  Scenario: Stale mirror lock is detected and released
    Given a bare mirror with a lock file held by a dead process (PID check fails)
    When the coordinator attempts to fetch the mirror
    Then the stale lock should be force-released
    And the fetch should proceed normally

  # --- Per-job clone ---

  Scenario: Coordinator clones from mirror into job directory
    Given a bare mirror exists for repo "org/my-app"
    And a coding job with job_id "job_000001"
    When the coordinator prepares the job
    Then the repo should be cloned into "jobs/job_000001/repo"
    And the clone should use the local mirror as the source
    And the clone should be a full (non-bare) repository

  Scenario: Clone acquires shared read lock on mirror
    Given a bare mirror exists for repo "org/my-app"
    When the coordinator clones from the mirror for a new job
    Then a shared flock should be held on the mirror during clone
    And multiple concurrent clones should be able to proceed in parallel

  Scenario: Clone checks out requested base ref
    Given a bare mirror exists with branch "main" and "develop"
    And a coding job requests base ref "develop"
    When the coordinator clones and prepares the job
    Then the working tree should be checked out at "develop"

  Scenario: Clone creates job branch from base ref
    Given a bare mirror exists for repo "org/my-app"
    And a coding job with job_id "job_000001" and base ref "main"
    When the coordinator clones and prepares the job
    Then a branch "quecto/job/job_000001" should be created from "main"
    And the working tree should be on "quecto/job/job_000001"

  # --- Clone failure handling ---

  Scenario: Clone fails when base ref does not exist in mirror
    Given a bare mirror exists for repo "org/my-app"
    And a coding job requests base ref "nonexistent-branch"
    When the coordinator attempts to clone and prepare the job
    Then the job should transition to "failed" with error_code "invalid_base_ref"

  Scenario: Clone fails when mirror directory is corrupted
    Given a corrupted bare mirror exists for repo "org/my-app"
    When the coordinator attempts to clone from the mirror
    Then the job should transition to "failed" with error_code "clone_error"
    And the coordinator should log the git error details

  Scenario: Clone timeout transitions job to failed
    Given a bare mirror exists for repo "org/my-app"
    When the clone operation exceeds the configured timeout
    Then the job should transition to "failed" with error_code "clone_timeout"

  # --- Flock contention ---

  Scenario: Clone waits while mirror fetch holds exclusive lock
    Given a bare mirror with an active exclusive fetch lock
    When the coordinator attempts to clone from the mirror
    Then the clone should block until the fetch lock is released
    And the clone should succeed after the lock is released

  Scenario: Fetch waits while clones hold shared locks
    Given a bare mirror with 2 active shared clone locks
    When the coordinator attempts to fetch the mirror
    Then the fetch should wait until all shared clone locks are released
    And the fetch should succeed after clones complete

  # --- Job directory isolation ---

  Scenario: Each job has its own independent git state
    Given 2 coding jobs for the same repo "org/my-app"
    When both jobs are cloned and prepared
    Then "jobs/job_000001/repo/.git" and "jobs/job_000002/repo/.git" should be separate
    And a commit in job_000001 should not appear in job_000002
    And a force-push in job_000001 should not affect job_000002

  Scenario: Job directory is within workspace boundary
    Given a coding coordinator with workspace "/home/quecto/workspace"
    When a job is cloned and prepared
    Then the job directory should be under the workspace path
    And the job directory path should not contain path traversal sequences

  # --- Cleanup ---

  Scenario: Job directory is removed on cleanup
    Given a coding job in state "succeeded" with job directory "jobs/job_000001"
    When the coordinator cleans up the job
    Then the directory "jobs/job_000001" should be removed
    And the bare mirror should not be affected

  Scenario: Mirror is preserved across job cleanups
    Given 2 coding jobs for repo "org/my-app" have been cleaned up
    Then the bare mirror at "repos/mirrors/org__my-app.git" should still exist
    And the mirror should be valid for future clones

  Scenario: Cleanup with keep_artifacts preserves artifact directory
    Given a coding job in state "succeeded" with artifacts in "jobs/job_000001/artifacts"
    When the coordinator cleans up the job with keep_artifacts true
    Then the repo directory "jobs/job_000001/repo" should be removed
    But the artifact directory "jobs/job_000001/artifacts" should be preserved

  # --- Mirror path safety ---

  Scenario: Mirror path is derived safely from repo identifier
    Given repo identifiers "org/my-app", "org/my-app.git", "../escape/attempt"
    Then the mirror path for "org/my-app" should be "org__my-app.git"
    And the mirror path for "../escape/attempt" should be rejected as invalid
    And no mirror path should escape the cache directory

  # --- Clone duration tracking ---

  Scenario: Clone duration is recorded in job.ready event
    Given a bare mirror exists for repo "org/my-app"
    When the coordinator clones, prepares, and launches the worker
    Then the "job.ready" event should include clone_duration_ms
    And clone_duration_ms should be a positive integer
