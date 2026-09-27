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
    Then the ls result should contain "[Entries 1-5 of 20 shown"
    And the ls result should contain "Next: offset=5"
    And the ls result should not be an error

  @done
  Scenario: Default limit is 500 entries (Quecto compatibility)
    Given ls workspace with 600 files named "file_NNN.txt"
    When I list the workspace
    Then the ls result should contain "[Entries 1-500 of 600 shown"
    And the ls result should not be an error

  @done
  Scenario: Float limit parameter is accepted
    Given ls workspace with 20 files named "file_NNN.txt"
    When I list the workspace with float limit 5.0
    Then the ls result should contain "[Entries 1-5 of 20 shown"
    And the ls result should not be an error

  @done
  Scenario: A truncated listing is the sorted start of the whole directory (#2188)
    Given ls workspace with 1200 files named "fN"
    When I list the workspace with limit 20
    Then the ls result should list sorted entries 1 to 20 of the directory
    And the ls result should contain "[Entries 1-20 of 1200 shown (sorted case-insensitively; limit 20 reached)"
    And the ls result should contain "Next: offset=20"
    And the ls result should not be an error

  @done
  Scenario: Offset continues where the previous page stopped (#2188)
    Given ls workspace with 45 files named "fN"
    When I list the workspace with limit 20 and offset 20
    Then the ls result should list sorted entries 21 to 40 of the directory
    And the ls result should contain "Next: offset=40"
    And the ls result should not be an error

  @done
  Scenario: Listing a file says to read it instead (#2189)
    Given ls workspace file "notes.txt"
    When I list the path "notes.txt"
    Then the ls result should be a refusal
    And the ls result should contain "notes.txt is a file, not a directory; read it with read."

  @done
  Scenario: Listing a missing directory names the directory to list instead (#2189)
    Given ls workspace directory "src"
    When I list the path "src/missing"
    Then the ls result should be a refusal
    And the ls result should contain "directory not found: src/missing (looked for "
    And the ls result should contain "Check the path, or list src with ls."
