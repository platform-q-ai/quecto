@done
Feature: Skills System
  As a user
  I want to install and manage skills that extend the agent
  So that I can customize what Quecto can do

  Skills use YAML frontmatter in SKILL.md files. Required fields:
  name, description. Optional fields:
  license, compatibility, metadata. The name must match the directory
  name and follow lowercase-alphanumeric-with-hyphens convention.

  # --- Frontmatter parsing ---

  Scenario: Parse skill with valid frontmatter
    Given a workspace skill directory "weather" with SKILL.md:
      """
      ---
      name: weather
      description: Fetch weather forecasts for any city
      ---
      ## What I do
      - Return current weather for a location
      """
    When the skill loader lists all skills
    Then the skill list should contain 1 skill
    And the skill list should include "weather"
    And the skill "weather" should have description "Fetch weather forecasts for any city"

  Scenario: Parse skill with all optional frontmatter fields
    Given a workspace skill directory "git-release" with SKILL.md:
      """
      ---
      name: git-release
      description: Create consistent releases and changelogs
      license: MIT
      compatibility: quecto
      metadata:
        audience: maintainers
        workflow: github
      ---
      ## Steps
      - Draft release notes
      """
    When the skill loader lists all skills
    Then the skill list should contain 1 skill
    And the skill "git-release" should have description "Create consistent releases and changelogs"

  Scenario: Skill body is content after frontmatter
    Given a workspace skill directory "code-review" with SKILL.md:
      """
      ---
      name: code-review
      description: Review code for quality
      ---
      You are a code review expert.
      Always suggest improvements.
      """
    When the skill "code-review" is loaded by name
    Then the loaded skill should exist
    And the loaded skill content should contain "code review expert"
    And the loaded skill content should not contain "name: code-review"

  # --- Validation ---

  Scenario: Skill without SKILL.md is skipped
    Given a workspace skill directory "empty-skill" without SKILL.md
    When the skill loader lists all skills
    Then the skill list should contain 0 skills

  Scenario: Skill without frontmatter is skipped
    Given a workspace skill directory "bad-skill" with SKILL.md:
      """
      Just some plain text without frontmatter delimiters.
      """
    When the skill loader lists all skills
    Then the skill list should contain 0 skills

  Scenario: Skill with name-directory mismatch is skipped
    Given a workspace skill directory "weather" with SKILL.md:
      """
      ---
      name: forecast
      description: Weather forecasts
      ---
      Content here
      """
    When the skill loader lists all skills
    Then the skill list should contain 0 skills

  Scenario: Skill with invalid name format is skipped
    Given a workspace skill directory "My_Skill" with SKILL.md:
      """
      ---
      name: My_Skill
      description: A skill with invalid name
      ---
      Content
      """
    When the skill loader lists all skills
    Then the skill list should contain 0 skills

  Scenario: Skill with missing description is skipped
    Given a workspace skill directory "no-desc" with SKILL.md:
      """
      ---
      name: no-desc
      ---
      Content
      """
    When the skill loader lists all skills
    Then the skill list should contain 0 skills

  # --- Multiple skills ---

  Scenario: List multiple valid skills
    Given a workspace skill directory "weather" with SKILL.md:
      """
      ---
      name: weather
      description: Weather forecasts
      ---
      Weather content
      """
    And a workspace skill directory "code-review" with SKILL.md:
      """
      ---
      name: code-review
      description: Code review assistant
      ---
      Review content
      """
    When the skill loader lists all skills
    Then the skill list should contain 2 skills
    And the skill list should include "weather"
    And the skill list should include "code-review"

  Scenario: Invalid skills are silently skipped alongside valid ones
    Given a workspace skill directory "weather" with SKILL.md:
      """
      ---
      name: weather
      description: Weather forecasts
      ---
      Weather content
      """
    And a workspace skill directory "bad-skill" with SKILL.md:
      """
      No frontmatter here
      """
    When the skill loader lists all skills
    Then the skill list should contain 1 skill
    And the skill list should include "weather"

  # --- CLI ---

  Scenario: Remove an installed skill
    Given a workspace with skill "weather" installed
    When I run quecto with arguments "skills remove weather"
    Then the exit code should be 0
    And the output should contain "removed successfully"

  # --- Install (not yet implemented) ---

  @done
  Scenario: Install a skill from GitHub URL
    Given a quecto base directory at a temporary path
    And a config file with a workspace directory
    And a mock GitHub API serving a skill repository at "user/repo/weather"
    When I run quecto with arguments "skills install user/repo/weather"
    Then the exit code should be 0
    And the output should contain "installed"
    And the workspace should contain a skill directory "weather"
    And the skill directory should contain a "SKILL.md" file

  @done
  Scenario: Install skill fails for invalid GitHub path
    Given a quecto base directory at a temporary path
    And a config file with a workspace directory
    When I run quecto with arguments "skills install invalid-path"
    Then the exit code should be 1
    And the output should contain "invalid skill path"

  @done
  Scenario: Install skill fails when already exists
    Given a quecto base directory at a temporary path
    And a config file with a workspace directory
    And a workspace with skill "weather" installed
    When I run quecto with arguments "skills install user/repo/weather"
    Then the exit code should be 1
    And the output should contain "already exists"
