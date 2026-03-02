@done
Feature: E2E Real LLM Tool Coverage
  End-to-end tests for tools that previously had zero real-LLM coverage.
  Validates that a real LLM can correctly invoke grep and find tools,
  and that the find tool handles path-segment globs with nested .gitignore
  files correctly.

  Background:
    Given a real LLM workspace is configured

  # --- grep tool ---

  @done @real-llm
  Scenario: Real LLM uses grep to find a pattern in files
    Given a file "src/main.rs" in the e2e workspace with content "fn main() {\n    println!(\"HELLO_GREP_42\");\n}"
    And a file "src/lib.rs" in the e2e workspace with content "pub fn add(a: i32, b: i32) -> i32 { a + b }"
    When I run the real LLM agent with message "Use the grep tool to search for the pattern 'HELLO_GREP_42' in the workspace. If you find it, reply with exactly GREP_FOUND. If not found, reply with GREP_MISSING."
    Then the exit code should be 0
    And stdout should contain "GREP_FOUND"

  @done @real-llm
  Scenario: Real LLM uses grep and reports no match
    Given a file "notes.txt" in the e2e workspace with content "nothing special here"
    When I run the real LLM agent with message "Use the grep tool to search for the pattern 'UNIQUE_XYZ_999' in the workspace. If no matches are found, reply with exactly GREP_NONE. Otherwise reply with GREP_HIT."
    Then the exit code should be 0
    And stdout should contain "GREP_NONE"

  # --- find tool ---

  @done @real-llm
  Scenario: Real LLM uses find to locate files by extension
    Given a file "docs/readme.md" in the e2e workspace with content "# Readme"
    And a file "docs/guide.md" in the e2e workspace with content "# Guide"
    And a file "src/main.rs" in the e2e workspace with content "fn main() {}"
    When I run the real LLM agent with message "Use the find tool to find all files matching '*.md' in the workspace. List the filenames you found, one per line."
    Then the exit code should be 0
    And stdout should contain "readme.md"
    And stdout should contain "guide.md"

  @done @real-llm
  Scenario: Real LLM uses find with path-segment glob
    Given a file "src/app.rs" in the e2e workspace with content "fn app() {}"
    And a file "src/lib.rs" in the e2e workspace with content "pub fn lib() {}"
    And a file "tests/test.rs" in the e2e workspace with content "fn test() {}"
    When I run the real LLM agent with message "Use the find tool with pattern 'src/*.rs' to find Rust files only in the src directory. Reply with exactly the filenames found, separated by commas. Do not include files from other directories."
    Then the exit code should be 0
    And stdout should contain "app.rs"
    And stdout should contain "lib.rs"

  @done @real-llm
  Scenario: Real LLM uses find and result is not suppressed by nested gitignore
    Given a file "project/data.json" in the e2e workspace with content "{}"
    And a file "project/readme.txt" in the e2e workspace with content "hello"
    And a file "vendor/.gitignore" in the e2e workspace with content "*.json"
    When I run the real LLM agent with message "Use the find tool with pattern '*.json' to search from the workspace root. If you find data.json, reply with exactly FIND_JSON_OK. If not found, reply with FIND_JSON_MISSING."
    Then the exit code should be 0
    And stdout should contain "FIND_JSON_OK"

  # Note: cron tool is gateway-only (not available in CLI agent or REPL).
  # It has 20+ mock BDD scenarios. Real-LLM cron coverage would require
  # the gateway Telegram mock harness — deferred to a gateway-specific PR.
