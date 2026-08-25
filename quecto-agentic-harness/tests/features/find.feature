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

  Scenario: Find fixture allows search path outside workspace
    When I find files matching "*.conf" outside workspace in path "/etc"
    Then the find result should not be an error

  Scenario: Nested glob pattern matches deeply
    Given a find workspace file "a/b/c/deep.rs"
    When I find files matching "**/*.rs"
    Then the find result should contain "deep.rs"
    And the find result should not be an error

  @done
  Scenario: Nested .gitignore in subdirectory is respected in git repo
    Given a find tool git workspace
    And a find workspace file "src/main.rs"
    And a find workspace file "src/generated/auto.rs"
    And a find workspace gitignore "src/.gitignore" ignoring "generated/"
    When I find files matching "*.rs"
    Then the find result should contain "main.rs"
    And the find result should not contain "auto.rs"
    And the find result should not be an error

  @done
  Scenario: Float limit parameter is accepted
    Given a find workspace with 10 files named "file_NNN.txt"
    When I find files matching "*.txt" with float limit 5.0
    Then the find result should contain "limit"
    And the find result should not be an error

  Scenario: Path-segment glob matches files in named subdirectory
    Given a find workspace file "src/main.rs"
    And a find workspace file "src/lib.rs"
    And a find workspace file "docs/readme.md"
    When I find files matching "src/*.rs"
    Then the find result should contain "main.rs"
    And the find result should contain "lib.rs"
    And the find result should not contain "readme.md"
    And the find result should not be an error

  Scenario: Path-segment glob with explicit path narrows search
    Given a find workspace file "nested/a.txt"
    And a find workspace file "nested/b.log"
    And a find workspace file "other/c.txt"
    When I find files matching "nested/*.txt" in path "."
    Then the find result should contain "a.txt"
    And the find result should not contain "b.log"
    And the find result should not contain "c.txt"
    And the find result should not be an error

  Scenario: Exact relative path pattern matches single file
    Given a find workspace file "src/config.json"
    And a find workspace file "src/other.json"
    And a find workspace file "top.json"
    When I find files matching "src/config.json"
    Then the find result should contain "config.json"
    And the find result should not contain "other.json"
    And the find result should not contain "top.json"
    And the find result should not be an error

  Scenario: Pattern description does not mislead about path-segment support
    Then the find tool description should support path-segment glob patterns

  Scenario: Catch-all gitignore in workspace subdirectory does not suppress root search
    Given a find workspace file "notes.txt"
    And a find workspace file "sub/.gitignore" with content "*\n!.gitignore\n"
    When I find files matching "*.txt"
    Then the find result should contain "notes.txt"
    And the find result should not be an error

  Scenario: Root path search finds files despite nested gitignore
    Given a find workspace file "readme.md"
    And a find workspace file "deep/nested/.gitignore" with content "*\n"
    When I find files matching "*.md" in path "."
    Then the find result should contain "readme.md"
    And the find result should not be an error

  @done
  Scenario: Nested gitignore with specific patterns does not suppress files globally
    Given a find workspace file "root.json"
    And a find workspace file "project/data.json"
    And a find workspace file "vendor/.gitignore" with content "*.json\n"
    When I find files matching "*.json"
    Then the find result should contain "root.json"
    And the find result should contain "data.json"
    And the find result should not be an error

  @done
  Scenario: Path-segment glob not suppressed by unrelated nested gitignore
    Given a find workspace file "app/src/config.json"
    And a find workspace file "app/src/main.rs"
    And a find workspace file "other/.gitignore" with content "*.json\n"
    When I find files matching "app/src/*.json" in path "."
    Then the find result should contain "config.json"
    And the find result should not be an error
