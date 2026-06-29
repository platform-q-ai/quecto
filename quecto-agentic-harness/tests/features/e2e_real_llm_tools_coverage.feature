@done
Feature: E2E Real LLM Tool Coverage
  End-to-end tests for tools that previously had zero real-LLM coverage.
  Validates that a real LLM can correctly invoke grep and find tools,
  and that the find tool handles path-segment globs with nested .gitignore
  files correctly.

  Background:
    Given a real LLM workspace is configured

  # --- grep tool ---

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses grep to find a pattern in files
    Given a file "src/main.rs" in the e2e workspace with content "fn main() {\n    println!(\"HELLO_GREP_42\");\n}"
    And a file "src/lib.rs" in the e2e workspace with content "pub fn add(a: i32, b: i32) -> i32 { a + b }"
    When I run the real LLM agent with [message] "Use the grep tool to search for the pattern 'HELLO_GREP_42' in the workspace. If you find it, reply with exactly GREP_FOUND. If not found, reply with GREP_MISSING."
    Then the exit code should be 0
    And stdout should contain "GREP_FOUND"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses grep and reports no match
    Given a file "notes.txt" in the e2e workspace with content "nothing special here"
    When I run the real LLM agent with [message] "Use the grep tool to search for the pattern 'UNIQUE_XYZ_999' in the workspace. If no matches are found, reply with exactly GREP_NONE. Otherwise reply with GREP_HIT."
    Then the exit code should be 0
    And stdout should contain "GREP_NONE"

  # --- find tool ---

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses find to locate files by extension
    Given a file "docs/readme.md" in the e2e workspace with content "# Readme"
    And a file "docs/guide.md" in the e2e workspace with content "# Guide"
    And a file "src/main.rs" in the e2e workspace with content "fn main() {}"
    When I run the real LLM agent with [message] "Use the find tool to find all files matching '*.md' in the workspace. List the filenames you found, one per line."
    Then the exit code should be 0
    And stdout should contain "readme.md"
    And stdout should contain "guide.md"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses find with path-segment glob
    Given a file "src/app.rs" in the e2e workspace with content "fn app() {}"
    And a file "src/lib.rs" in the e2e workspace with content "pub fn lib() {}"
    And a file "tests/test.rs" in the e2e workspace with content "fn test() {}"
    When I run the real LLM agent with [message] "Use the find tool with pattern 'src/*.rs' to find Rust files only in the src directory. Reply with exactly the filenames found, separated by commas. Do not include files from other directories."
    Then the exit code should be 0
    And stdout should contain "app.rs"
    And stdout should contain "lib.rs"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses find and result is not suppressed by nested gitignore
    Given a file "project/data.json" in the e2e workspace with content "{}"
    And a file "project/readme.txt" in the e2e workspace with content "hello"
    And a file "vendor/.gitignore" in the e2e workspace with content "*.json"
    When I run the real LLM agent with [message] "Use the find tool with pattern '*.json' to search from the workspace root. If you find data.json, reply with exactly FIND_JSON_OK. If not found, reply with FIND_JSON_MISSING."
    Then the exit code should be 0
    And stdout should contain "FIND_JSON_OK"

  # --- web_fetch tool ---

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses web_fetch to retrieve a public URL
    Given a real LLM workspace is configured with web fetch enabled
    When I run the real LLM agent with [message] "Use the web_fetch tool to fetch the URL https://httpbin.org/html and check if the response contains the word 'Moby'. If it does, reply with exactly FETCH_MOBY_OK. If not, reply with FETCH_MOBY_FAIL."
    Then the exit code should be 0
    And stdout should contain "FETCH_MOBY_"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses web_fetch raw mode for JSON
    Given a real LLM workspace is configured with web fetch enabled
    When I run the real LLM agent with [message] "Use the web_fetch tool with raw mode to fetch https://httpbin.org/json and check if the response is valid JSON containing 'slideshow'. If yes, reply with exactly FETCH_JSON_OK. If not, reply with FETCH_JSON_FAIL."
    Then the exit code should be 0
    And stdout should contain "FETCH_JSON_"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM reports web_fetch error for non-existent host
    Given a real LLM workspace is configured with web fetch enabled
    When I run the real LLM agent with [message] "Use the web_fetch tool to fetch https://this-domain-does-not-exist-xyz123.example.com/ and tell me if it succeeded or failed. If it failed, reply with exactly FETCH_ERR_OK. If it succeeded reply with FETCH_ERR_FAIL."
    Then the exit code should be 0
    And stdout should contain "FETCH_ERR_"

  # --- recall tool ---

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses recall to list spilled outputs on fresh session
    When I run the real LLM agent with [message] "Use the recall tool with id 'list' to check for spilled outputs. If the result says 'No spilled outputs', reply with exactly RECALL_EMPTY_OK. Otherwise reply with RECALL_EMPTY_FAIL."
    Then the exit code should be 0
    And stdout should contain "RECALL_EMPTY_OK"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses recall with nonexistent ID
    When I run the real LLM agent with [message] "Use the recall tool with id 'nonexistent:turn99:0' to try to retrieve a spilled output. If the result indicates no output was found, reply with exactly RECALL_MISS_OK. Otherwise reply with RECALL_MISS_FAIL."
    Then the exit code should be 0
    And stdout should contain "RECALL_MISS_OK"

  # --- workflow tool ---

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses workflow tool to check status
    Given a real LLM workspace is configured with workflow enabled
    When I run the real LLM agent with [message] "Use the workflow tool with action 'status' to get the current workflow progress. If you get a response showing steps, reply with exactly WORKFLOW_STATUS_OK. If the tool is not available or errors, reply with WORKFLOW_STATUS_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_STATUS_OK"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses workflow tool to check and uncheck a step
    Given a real LLM workspace is configured with workflow enabled
    When I run the real LLM agent with [message] "Use the workflow tool: first call it with action 'check' and step 1. Then call it with action 'uncheck' and step 1. If both succeed, reply with exactly WORKFLOW_CHECK_OK. Otherwise reply with WORKFLOW_CHECK_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_CHECK_OK"

  @done @manual-real-llm @mock-llm
  Scenario: Real LLM uses workflow tool to set and clear issue
    Given a real LLM workspace is configured with workflow enabled
    When I run the real LLM agent with [message] "Use the workflow tool: call it with action 'set_issue', issueNumber 99, and issueTitle 'Test issue'. Then call it with action 'clear_issue'. If both succeed, reply with exactly WORKFLOW_ISSUE_OK. Otherwise reply with WORKFLOW_ISSUE_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_ISSUE_OK"
