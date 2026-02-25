@done
Feature: Worker Tool Trait Wrappers
  As the nsjail coding worker
  I need Tool trait wrappers around the worker coding functions
  So that the worker agent loop can invoke them through the standard ToolRegistry

  The worker runs an LLM-tool loop inside the sandbox. The agent loop
  requires tools that implement the domain Tool trait. This feature wraps
  the pure functions from worker_tools.rs (edit_file, grep_content,
  find_files, read_file_paginated) as Tool trait implementations that
  parse JSON arguments, delegate to the pure function, and serialize
  results as JSON in ToolResult.content.

  Background:
    Given a worker tool registry with job directory
    And the job directory contains files:
      | path         | content                                   |
      | src/main.rs  | fn main() {\n    println!("hello");\n}\n   |
      | src/lib.rs   | pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n |
      | README.md    | # My App\n\nA sample project.\n           |
      | .gitignore   | target/\n*.log\n                           |

  # --- WorkerEditTool ---

  Scenario: WorkerEditTool definition has correct name and schema
    Then the worker tool registry should contain a tool named "worker_edit"
    And the "worker_edit" tool definition should require fields "file_path, old_string, new_string"

  Scenario: WorkerEditTool executes a successful edit
    When I execute worker tool "worker_edit" with arguments:
      """
      {"file_path": "src/main.rs", "old_string": "hello", "new_string": "world"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the worker tool result JSON should have a non-empty "diff"

  Scenario: WorkerEditTool reports path violation
    When I execute worker tool "worker_edit" with arguments:
      """
      {"file_path": "../etc/passwd", "old_string": "root", "new_string": "hacked"}
      """
    Then the worker tool result should indicate an error
    And the worker tool result content should contain "path violation"

  Scenario: WorkerEditTool reports ambiguity error with match details
    When I execute worker tool "worker_edit" with arguments:
      """
      {"file_path": "src/lib.rs", "old_string": "a", "new_string": "x"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to false
    And the worker tool result JSON should have "match_count" greater than 1

  Scenario: WorkerEditTool supports preview mode
    When I execute worker tool "worker_edit" with arguments:
      """
      {"file_path": "src/main.rs", "old_string": "hello", "new_string": "world", "preview_only": true}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the file "src/main.rs" in the job directory should still contain "hello"

  Scenario: WorkerEditTool reports missing required arguments
    When I execute worker tool "worker_edit" with arguments:
      """
      {"file_path": "src/main.rs"}
      """
    Then the worker tool result should indicate an error
    And the worker tool result content should contain "old_string"

  # --- WorkerGrepTool ---

  Scenario: WorkerGrepTool definition has correct name and schema
    Then the worker tool registry should contain a tool named "worker_grep"
    And the "worker_grep" tool definition should require "pattern"

  Scenario: WorkerGrepTool finds matching lines
    When I execute worker tool "worker_grep" with arguments:
      """
      {"pattern": "println"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the worker tool result JSON "matches" array should not be empty

  Scenario: WorkerGrepTool returns empty matches for no hits
    When I execute worker tool "worker_grep" with arguments:
      """
      {"pattern": "nonexistent_string_xyz"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the worker tool result JSON "matches" array should be empty

  Scenario: WorkerGrepTool respects gitignore by default
    Given the job directory contains a file "target/debug.log" with content "println debug"
    When I execute worker tool "worker_grep" with arguments:
      """
      {"pattern": "println"}
      """
    Then the worker tool result JSON "matches" array should not contain a file matching "target/"

  # --- WorkerFindTool ---

  Scenario: WorkerFindTool definition has correct name and schema
    Then the worker tool registry should contain a tool named "worker_find"
    And the "worker_find" tool definition should require "glob"

  Scenario: WorkerFindTool finds files matching glob pattern
    When I execute worker tool "worker_find" with arguments:
      """
      {"glob": "**/*.rs"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the worker tool result JSON "files" array should contain "src/main.rs"

  Scenario: WorkerFindTool returns empty list for unmatched pattern
    When I execute worker tool "worker_find" with arguments:
      """
      {"glob": "**/*.java"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON "files" array should be empty

  # --- WorkerReadTool ---

  Scenario: WorkerReadTool definition has correct name and schema
    Then the worker tool registry should contain a tool named "worker_read"
    And the "worker_read" tool definition should require "file_path"

  Scenario: WorkerReadTool reads file content with pagination
    When I execute worker tool "worker_read" with arguments:
      """
      {"file_path": "src/main.rs", "offset": 0, "limit": 100}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the worker tool result JSON should have "total_lines" greater than 0
    And the worker tool result JSON "content" should contain "fn main()"

  Scenario: WorkerReadTool reports path violation for escape attempt
    When I execute worker tool "worker_read" with arguments:
      """
      {"file_path": "../../etc/shadow"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to false
    And the worker tool result JSON "error" should contain "path violation"

  Scenario: WorkerReadTool uses default offset and limit when omitted
    When I execute worker tool "worker_read" with arguments:
      """
      {"file_path": "src/main.rs"}
      """
    Then the worker tool result should succeed
    And the worker tool result JSON should have "ok" equal to true
    And the worker tool result JSON "content" should contain "fn main()"

  # --- Registry integration ---

  Scenario: All worker tools are registered and discoverable
    Then the worker tool registry should contain exactly 4 tools
    And the worker tool registry definitions should include "worker_edit"
    And the worker tool registry definitions should include "worker_grep"
    And the worker tool registry definitions should include "worker_find"
    And the worker tool registry definitions should include "worker_read"

  Scenario: Worker tool registry rejects unknown tool name
    When I execute worker tool "nonexistent_worker_tool" with arguments:
      """
      {}
      """
    Then the worker tool execution should fail with an unknown tool error
