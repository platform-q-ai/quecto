@pending
Feature: Artifact Export and Retrieval
  As the coding runtime coordinator
  I want to capture and persist job artifacts (patches, logs, summaries)
  So that the main agent can inspect results and decide whether to publish

  When a job completes (success or failure), the coordinator captures
  artifacts from the worker's job directory: git patches, commit metadata,
  test output, run logs, and a structured summary. Artifacts are stored
  in the job's artifact directory and referenced by event payloads.

  Background:
    Given a coding coordinator with artifact storage enabled
    And a coding job that has completed worker execution

  # --- Patch export ---

  Scenario: Coordinator captures git diff as patch artifact
    Given the worker committed changes to "src/main.rs" and "src/lib.rs"
    When the coordinator exports artifacts for the job
    Then a "patch.diff" artifact should exist in "jobs/job_000001/artifacts/"
    And the patch should contain a valid unified diff
    And an "artifact.created" event should be emitted with artifact_type "patch"

  Scenario: Patch artifact is empty when worker made no changes
    Given the worker made no code changes
    When the coordinator exports artifacts
    Then the "patch.diff" artifact should be empty or absent
    And no patch artifact event should be emitted

  # --- Commit metadata ---

  Scenario: Coordinator captures commit metadata
    Given the worker made 3 commits on the job branch
    When the coordinator exports artifacts
    Then a "commits.json" artifact should exist
    And it should contain 3 entries with hash, message, author, and timestamp
    And an "artifact.created" event should be emitted with artifact_type "commits"

  # --- Run log ---

  Scenario: Coordinator captures worker run log
    Given the worker produced tool execution output during the run
    When the coordinator exports artifacts
    Then a "run.log" artifact should exist
    And it should contain the worker's stderr and diagnostic output
    And an "artifact.created" event should be emitted with artifact_type "log"

  Scenario: Run log is truncated when it exceeds size limit
    Given the worker produced 10 MB of diagnostic output
    When the coordinator exports artifacts
    Then the "run.log" artifact should be truncated to the configured limit
    And a truncation marker should be appended to the log

  # --- Structured summary ---

  Scenario: Coordinator captures structured summary on success
    Given the worker completed with summary "all 12 tests pass"
    When the coordinator exports artifacts
    Then a "summary.json" artifact should exist
    And it should include the goal, state "succeeded", summary text, and artifact list
    And an "artifact.created" event should be emitted with artifact_type "summary"

  Scenario: Coordinator captures structured summary on failure
    Given the worker failed with error_code "tool_error" and detail "edit ambiguity"
    When the coordinator exports artifacts
    Then a "summary.json" artifact should exist
    And it should include state "failed", error_code, and error_detail

  # --- Test output ---

  Scenario: Coordinator captures test output artifact when tests were run
    Given the worker ran tests and produced stdout/stderr
    When the coordinator exports artifacts
    Then a "test_output.log" artifact should exist
    And an "artifact.created" event should be emitted with artifact_type "test_output"

  Scenario: No test output artifact when no tests were run
    Given the worker did not run any test commands
    When the coordinator exports artifacts
    Then no "test_output.log" artifact should exist

  # --- Artifact references in status ---

  Scenario: Status response includes artifact list after export
    Given the coordinator has exported artifacts for a succeeded job
    When the main agent queries job status
    Then the status response should include artifacts ["patch.diff", "commits.json", "summary.json", "run.log"]

  Scenario: Artifact list is empty for jobs that have not exported yet
    Given a coding job is still running
    When the main agent queries job status
    Then the artifacts list should be empty

  # --- Artifact directory structure ---

  Scenario: Artifacts are stored in per-job directory
    Given a coding job with job_id "job_000001"
    When artifacts are exported
    Then all artifacts should be under "jobs/job_000001/artifacts/"
    And no artifacts should be written outside the job directory

  Scenario: Artifact directory is created if it does not exist
    Given no artifact directory exists for the job
    When the coordinator begins artifact export
    Then the directory should be created automatically
    And the export should succeed

  # --- Cleanup interaction ---

  Scenario: Cleanup with keep_artifacts preserves artifact directory
    Given a completed job with exported artifacts
    When the coordinator cleans up the job with keep_artifacts true
    Then the artifact directory should be preserved
    And the repo directory should be removed

  Scenario: Cleanup without keep_artifacts removes everything
    Given a completed job with exported artifacts
    When the coordinator cleans up the job with keep_artifacts false
    Then both the artifact directory and repo directory should be removed

  # --- Secrets in artifacts ---

  Scenario: Secrets are not present in any artifact files
    Given the worker's environment contained API keys
    When the coordinator exports artifacts
    Then "patch.diff" should not contain any API key patterns
    And "run.log" should not contain any API key patterns
    And "summary.json" should not contain any API key patterns

  # --- Artifact event envelope ---

  Scenario: Artifact events include required metadata
    When the coordinator emits an "artifact.created" event
    Then the event payload should include artifact_type, path, and size_bytes
    And the event should have source "coordinator"
    And the path should be relative to the job directory

  # --- Skills snapshot artifact ---

  Scenario: Applied skills are recorded as artifact
    Given the coordinator injected skills "rust-style" and "test-first" at job start
    When the coordinator exports artifacts
    Then a "skills_applied.json" artifact should exist
    And it should list the applied skill names and their source

  # --- Spawn log artifact ---

  Scenario: Child agent spawn log is recorded as artifact
    Given the job spawned a child agent "security-reviewer" that completed
    When the coordinator exports artifacts
    Then a "spawn_log.json" artifact should exist
    And it should include the spawn request, decision, and result
