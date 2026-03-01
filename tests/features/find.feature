@done
Feature: Find Tool
  As an AI agent
  I want to find files by glob pattern using fd
  So that I can discover workspace files efficiently without wasting tokens on bash

  Background:
    Given a find tool workspace

  Scenario: Basic glob pattern finds matching files
    Given a find workspace file "src/main.rs"
    And a find workspace file "src/lib.rs"
    And a find workspace file "README.md"
    When I find files matching "*.rs"
    Then the find result should contain "main.rs"
    And the find result should contain "lib.rs"
    And the find result should not contain "README.md"
    And the find result should not be an error

  Scenario: No matches returns informative message
    Given a find workspace file "hello.txt"
    When I find files matching "*.rs"
    Then the find result should contain "No files found"
    And the find result should not be an error

  Scenario: Default path searches current workspace
    Given a find workspace file "notes.txt"
    When I find files matching "*.txt" with no path specified
    Then the find result should contain "notes.txt"
    And the find result should not be an error

  Scenario: Limit caps the number of results
    Given a find workspace with 1100 files named "file_NNN.txt"
    When I find files matching "*.txt" with limit 10
    Then the find result should contain "limit"
    And the find result should not be an error

  Scenario: Directory entries have trailing slash
    Given a find workspace directory "subdir"
    When I find files matching "subdir"
    Then the find result should contain "subdir/"
    And the find result should not be an error

  Scenario: Missing fd binary returns clear error
    When I find with missing fd binary for pattern "*.rs"
    Then the find result should be an error
    And the find result should contain "fd"

  Scenario: Search path outside workspace is blocked
    When I find files matching "*.conf" in path "/etc"
    Then the find result should be an error

  Scenario: Nested glob pattern matches deeply
    Given a find workspace file "a/b/c/deep.rs"
    When I find files matching "**/*.rs"
    Then the find result should contain "deep.rs"
    And the find result should not be an error
