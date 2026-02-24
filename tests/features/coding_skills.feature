@pending
Feature: Skill Injection Policy
  As the coding runtime coordinator
  I want to inject skills into worker jobs under policy control
  So that workers have appropriate guidance while maintaining reproducibility

  The coordinator resolves skills from workspace sources, applies allowlist/denylist
  policy, snapshots the effective skill set at job start, and injects it into the
  worker's system context. Workers can suggest additional skills but cannot load
  them directly.

  Background:
    Given a coding coordinator with skill policy:
      | enable_injection | true                          |
      | default          | ["rust-style", "test-first"]  |
      | allowlist        | ["rust-style", "test-first", "security-checklist"] |
      | denylist         | ["forbidden-skill"]           |

  # --- Skill injection at job start ---

  Scenario: Coordinator injects default skills into worker context
    When a coding job starts with no additional skills requested
    Then a "skills.applied" event should be emitted
    And the skills list should be ["rust-style", "test-first"]
    And the snapshot_ref should point to a persisted skills snapshot file

  Scenario: Coordinator injects task-specific skills alongside defaults
    When a coding job starts with skills ["security-checklist"]
    Then a "skills.applied" event should be emitted
    And the skills list should include "rust-style", "test-first", and "security-checklist"

  Scenario: Coordinator rejects denylisted skills
    When a coding job starts with skills ["forbidden-skill"]
    Then the run command should fail with error code "policy_denied"
    And no job should be created

  Scenario: Coordinator rejects skills not in allowlist
    When a coding job starts with skills ["unknown-skill"]
    Then the run command should fail with error code "policy_denied"

  # --- Skill snapshots ---

  Scenario: Skill content is snapshotted at job start for reproducibility
    When a coding job starts and skills are injected
    Then the snapshot file should contain the full text content of each skill
    And the snapshot should be immutable for the duration of the job

  Scenario: Skills applied artifact is recorded
    When a coding job starts and skills are injected
    Then a "skills_applied.json" artifact should exist in the job directory
    And it should record which skills were applied and their source

  # --- Worker skill suggestions ---

  Scenario: Worker suggests an additional skill
    Given a coding job in state "running"
    When the worker emits a "skills.suggested" event with:
      | skills | ["security-checklist"]     |
      | reason | touches authentication flow |
    Then the coordinator should record the suggestion
    And the suggestion should be visible in job status for main-agent review

  Scenario: Worker cannot load skills directly
    Given a coding job in state "running"
    When the worker attempts to read a skill file from the workspace
    Then the worker should not have access to the skills directory
    And skill loading should only be possible through coordinator injection

  # --- Disabled injection ---

  Scenario: No skills are injected when injection is disabled
    Given a coding coordinator with skill policy enable_injection false
    When a coding job starts
    Then no "skills.applied" event should be emitted
    And the worker system context should not contain skill content

  # --- Profile-based skill resolution ---

  Scenario: Skills are resolved based on job profile
    Given a coding coordinator with profile "backend" that includes skills ["api-design"]
    When a coding job starts with profile "backend"
    Then the effective skill set should include "api-design" plus defaults
