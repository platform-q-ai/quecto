@done
Feature: Grep Tool
  As an AI agent
  I want to search file contents using ripgrep
  So that I can find patterns across workspace files efficiently

  Background:
    Given a grep tool workspace

  Scenario: Basic pattern search returns matches
    Given a grep workspace file "main.rs" with content:
      """
      fn main() {
          println!("hello world");
          let x = 42;
      }
      """
    When I grep for pattern "hello"
    Then the grep result should contain "hello"
    And the grep result should not be an error

  Scenario: No matches returns informative message
    Given a grep workspace file "empty.rs" with content:
      """
      fn nothing() {}
      """
    When I grep for pattern "xyz_does_not_exist"
    Then the grep result should contain "No matches found"
    And the grep result should not be an error

  Scenario: Case-insensitive search
    Given a grep workspace file "doc.txt" with content:
      """
      Hello World
      goodbye
      """
    When I grep for pattern "hello" with ignoreCase true
    Then the grep result should contain "Hello"
    And the grep result should not be an error

  Scenario: Literal string search does not treat as regex
    Given a grep workspace file "code.py" with content:
      """
      x = (a + b)
      y = 2
      """
    When I grep for pattern "(a + b)" with literal true
    Then the grep result should contain "(a + b)"
    And the grep result should not be an error

  Scenario: Glob filter restricts search to matching files
    Given a grep workspace file "main.rs" with content:
      """
      fn hello() {}
      """
    And a grep workspace file "notes.txt" with content:
      """
      hello notes
      """
    When I grep for pattern "hello" with glob "*.rs"
    Then the grep result should contain "main.rs"
    And the grep result should not be an error

  Scenario: Limit caps the number of matches
    Given a grep workspace file "many.txt" with 200 lines containing "needle"
    When I grep for pattern "needle" with limit 10
    Then the grep result should contain "10 matches limit reached"
    And the grep result should not be an error

  Scenario: Missing rg binary returns clear error
    When I grep with missing rg binary for pattern "anything"
    Then the grep result should be an error
    And the grep result should contain "rg"

  Scenario: Grep fixture allows pattern search outside workspace
    When I grep for pattern "root" in path "/etc/ssl/openssl.cnf"
    Then the grep result should not be an error

  @done
  Scenario: Context lines use file cache (Quecto compatibility — file-N- format)
    Given a grep workspace file "ctx.rs" with content:
      """
      line one
      fn target() {}
      line three
      """
    When I grep for pattern "target" with context 1
    Then the grep result should contain "ctx.rs:2:"
    And the grep result should contain "ctx.rs-1-"
    And the grep result should contain "ctx.rs-3-"
    And the grep result should not be an error

  @done
  Scenario: Match limit notice includes suggested increase
    Given a grep workspace file "many.txt" with 200 lines containing "needle"
    When I grep for pattern "needle" with limit 5
    Then the grep result should contain "5 matches limit reached"
    And the grep result should contain "limit=10"
    And the grep result should not be an error

  @done
  Scenario: Composite truncation notice when both match limit and line truncation apply
    Given a grep workspace file "long_lines.txt" with 10 lines of 600 chars containing "target"
    When I grep for pattern "target" with limit 3
    Then the grep result should contain "3 matches limit reached"
    And the grep result should contain "Use read tool to see full lines"
    And the grep result should not be an error

  @done
  Scenario: Filenames with colons are parsed correctly via JSON output
    Given a grep workspace file "time:zone.rs" with content:
      """
      fn timezone() {}
      """
    When I grep for pattern "timezone"
    Then the grep result should contain "time:zone.rs"
    And the grep result should not be an error

  # ─── rg parity (#2136): what agents otherwise reach for rg in bash for ───

  @done @issue-2136
  Scenario: Files mode lists each matching file once
    Given a grep workspace file "src/a.rs" with content:
      """
      fn needle() {}
      let x = needle();
      """
    And a grep workspace file "src/b.rs" with content:
      """
      needle
      """
    And a grep workspace file "src/c.rs" with content:
      """
      nothing here
      """
    When I grep with arguments:
      """
      {"pattern": "needle", "output": "files"}
      """
    Then the grep result should contain "src/a.rs"
    And the grep result should contain "src/b.rs"
    And the grep result should not contain "src/c.rs"
    And the grep result should not contain "fn needle"
    And the grep result should not be an error

  @done @issue-2136
  Scenario: Count mode reports each file's matches, busiest first
    Given a grep workspace file "one.rs" with content:
      """
      needle
      """
    And a grep workspace file "three.rs" with content:
      """
      needle
      needle
      needle
      """
    When I grep with arguments:
      """
      {"pattern": "needle", "output": "count"}
      """
        Then the grep result should contain "three.rs: 3"
    And the grep result should contain "one.rs: 1"
    And the grep result should list "three.rs" before "one.rs"
    And the grep result should not be an error

  @done @issue-2136
  Scenario: File types, several globs, whole words and several patterns narrow the search
    Given a grep workspace file "lib.rs" with content:
      """
      fn retry_backoff() {}
      fn backoffice() {}
      """
    And a grep workspace file "tool.py" with content:
      """
      def retry_backoff(): pass
      """
    And a grep workspace file "notes.md" with content:
      """
      jitter matters
      """
    When I grep with arguments:
      """
            {"patterns": ["backoff", "jitter"], "type": "rust"}
      """
    Then the grep result should contain "lib.rs:1:"
    And the grep result should contain "lib.rs:2:"
    And the grep result should not contain "tool.py"
    And the grep result should not contain "notes.md"
    When I grep with arguments:
      """
      {"patterns": ["backoff", "jitter"], "wordRegexp": true, "glob": ["*.rs", "*.md"]}
      """
    Then the grep result should contain "notes.md:1:"
    And the grep result should not contain "lib.rs"
    And the grep result should not contain "tool.py"

  @done @issue-2136
  Scenario: A multiline pattern shows every line it spans, and matches can be capped per file
    Given a grep workspace file "chain.rs" with content:
      """
      let a = builder()
          .retries(3)
          .build();
      let b = builder()
          .retries(5)
          .build();
      """
    When I grep with arguments:
      """
      {"pattern": "builder\\(\\)\\n\\s+\\.retries", "multiline": true, "maxPerFile": 1}
      """
    Then the grep result should contain "chain.rs:1: let a = builder()"
    And the grep result should contain "chain.rs:2:     .retries(3)"
    And the grep result should not contain "retries(5)"
    And the grep result should not be an error

  @done @issue-2136
  Scenario: An argument the tool cannot honour is refused with the valid choices
    When I grep with arguments:
      """
      {"pattern": "x", "output": "lines"}
      """
    Then the grep result should be an error
    And the grep result should contain "output must be one of content, files, count"
