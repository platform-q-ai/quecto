@done
Feature: Skills System
  As a user
  I want to install and manage skills that extend the agent
  So that I can customize what Quecto can do

  Scenario: List skills from workspace
    Given a skill loader with workspace skill "weather" containing "Weather forecast skill"
    When the skills loader lists all skills
    Then the skill list should contain 1 skill
    And the skill list should include "weather"
    And the skill "weather" should have source "workspace"

  Scenario: List skills from multiple sources
    Given a skill loader with workspace skill "weather" containing "Weather skill"
    And a skill loader with global skill "calculator" containing "Calculator skill"
    And a skill loader with builtin skill "news" containing "News skill"
    When the skills loader lists all skills
    Then the skill list should contain 3 skills
    And the skill list should include "weather"
    And the skill list should include "calculator"
    And the skill list should include "news"

  Scenario: Load specific skill by name
    Given a skill loader with workspace skill "weather" containing "Weather forecast content"
    When the skill "weather" is loaded by name
    Then the loaded skill should exist
    And the loaded skill content should contain "Weather forecast content"

  Scenario: Load nonexistent skill returns none
    Given an empty skill loader
    When the skill "nonexistent" is loaded by name
    Then the loaded skill should not exist

  Scenario: Workspace skills take priority over global
    Given a skill loader with workspace skill "weather" containing "workspace version"
    And a skill loader with global skill "weather" containing "global version"
    When the skill "weather" is loaded by name
    Then the loaded skill should have source "workspace"
    And the loaded skill content should contain "workspace version"

  Scenario: Skill without SKILL.md has empty content
    Given a skill loader with workspace skill "empty_skill" without SKILL.md
    When the skills loader lists all skills
    Then the skill list should contain 1 skill
    And the skill "empty_skill" should have empty content

  @pending
  Scenario: Install a skill from GitHub
    When I run quecto with arguments "skills install sipeed/quecto-skills/weather"
    Then the exit code should be 0
    And the output should contain "installed successfully"

  Scenario: Remove an installed skill
    Given a workspace with skill "weather" installed
    When I run quecto with arguments "skills remove weather"
    Then the exit code should be 0
    And the output should contain "removed successfully"
