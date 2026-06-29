@done
Feature: LsTool — Quecto compatibility
  As an AI agent
  I want to list directory contents with Quecto-compatible behaviour
  So that I can navigate workspaces efficiently

  Background:
    Given an ls tool workspace

  Scenario: Lists files and directories
    Given ls workspace file "a.txt"
    And ls workspace directory "subdir"
    When I list the workspace
    Then the ls result should contain "a.txt"
    And the ls result should contain "subdir/"
    And the ls result should not be an error

  @done
  Scenario: Empty directory returns informative message (Quecto compatibility)
    When I list the workspace
    Then the ls result should contain "(empty directory)"
    And the ls result should not be an error

  @done
  Scenario: Case-insensitive sort (Quecto compatibility)
    Given ls workspace file "Makefile"
    And ls workspace file "app.rs"
    And ls workspace file "Zoo.rs"
    When I list the workspace
    Then the ls result should have "app.rs" before "Makefile"
    And the ls result should have "Makefile" before "Zoo.rs"
    And the ls result should not be an error

  @done
  Scenario: Limit parameter caps entries returned (Quecto compatibility)
    Given ls workspace with 20 files named "file_NNN.txt"
    When I list the workspace with limit 5
    Then the ls result should contain "5 entries limit reached"
    And the ls result should contain "limit=10"
    And the ls result should not be an error

  @done
  Scenario: Default limit is 500 entries (Quecto compatibility)
    Given ls workspace with 600 files named "file_NNN.txt"
    When I list the workspace
    Then the ls result should contain "500 entries limit reached"
    And the ls result should not be an error

  @done
  Scenario: Float limit parameter is accepted
    Given ls workspace with 20 files named "file_NNN.txt"
    When I list the workspace with float limit 5.0
    Then the ls result should contain "5 entries limit reached"
    And the ls result should not be an error
