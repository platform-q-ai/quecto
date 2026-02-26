@done
Feature: Coding Job Operational Wiring
  As a Quecto operator
  I want coding_job to be wired through real entrypoints
  So that CLI and gateway sessions can run coding jobs end-to-end

  This feature covers the remaining operational work after the coding_job
  tool adapter landed: concrete infrastructure adapters, composition-root
  registration, and runtime behavior in long-running processes.

  # --- Infrastructure adapters ---

  Scenario: Repo validator adapter checks repository and base ref
    Given a workspace with a real git repository
    When the coding runtime validates repo "test-repo" and base ref "main"
    Then repo validation should succeed
    And base ref validation should succeed

  Scenario: Skill resolver adapter checks skills from workspace
    Given a workspace with installed skills
    When the coding runtime resolves skill "default-skill"
    Then skill resolution should succeed

  # --- Composition root wiring ---

  Scenario: CLI registry wiring includes coding_job
    Given a core tool registry for a workspace
    When coding_job wiring is applied for CLI and definitions are listed
    Then the registry should include a tool named "coding_job"

  Scenario: Gateway registry wiring includes coding_job
    Given a core tool registry for a workspace
    When coding_job wiring is applied for gateway and definitions are listed
    Then the registry should include a tool named "coding_job"

  # --- End-to-end runtime behavior ---

  Scenario: Workspace-backed coding_job can run and query job status
    Given a workspace-backed coding_job tool
    When I execute coding_job run for repo "test-repo" and base ref "main"
    Then the coding_job tool result should not be an error
    And the coding_job tool result should contain "job_id"
    When I execute coding_job status for the created job
    Then the coding_job tool result should not be an error
    And the coding_job tool result should contain "queued"

  Scenario: Workspace-backed coding_job exposes cleanup path for terminal jobs
    Given a workspace-backed coding_job tool
    And a coding job exists via workspace-backed coding_job
    When I cancel and cleanup the created coding job
    Then the coding_job tool result should not be an error
    When I list coding jobs
    Then the coding_job tool result should contain "jobs"

  # --- Operational safeguards ---

  Scenario: Runtime lifecycle policy defines coordinator scope
    When coding coordinator scope policy is queried
    Then CLI coding coordinator scope should be "per_session"
    And gateway inbound coding coordinator scope should be "per_session"
    And gateway background coding coordinator scope should be "shared"
