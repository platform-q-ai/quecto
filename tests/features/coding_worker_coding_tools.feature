@done
Feature: Worker Coding Tool Implementations
  As a coding worker running inside nsjail
  I want first-class coding tools with rich diagnostics
  So that I can reliably read, edit, search, and build code within the job repo

  The worker runs a tool loop inside the sandbox. Tools operate on the
  per-job clone only. This feature covers the worker-side tool
  implementations that go beyond the existing shared tools: enhanced edit
  with diagnostics, grep/find with .gitignore awareness, edit preview,
  and safe git wrappers.

  Background:
    Given a coding worker running inside nsjail
    And a job repo with files:
      | path              | content                           |
      | src/main.rs       | fn main() {\n    println!("hello");\n}\n |
      | src/lib.rs        | pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n |
      | tests/test_add.rs | use mylib::add;\n#[test]\nfn test_add() {\n    assert_eq!(add(1, 2), 3);\n}\n |
      | README.md         | # My App\n\nA sample project.\n   |
      | .gitignore        | target/\n*.log\n                   |

  # --- Enhanced edit_file ---

  Scenario: Exact replace edit succeeds and returns unified diff
    When the worker edits "src/main.rs" replacing "hello" with "world"
    Then the edit should succeed
    And the tool result should include a unified diff showing the change
    And the result should include the first changed line number

  Scenario: Edit detects ambiguous match and returns candidates
    Given "src/lib.rs" contains the string "a" in multiple locations
    When the worker edits "src/lib.rs" replacing "a" with "x"
    Then the edit should fail with an ambiguity error
    And the error should report the number of matches found
    And the error should include line numbers of each match

  Scenario: Edit detects no-op when old and new strings are identical
    When the worker edits "src/main.rs" replacing "hello" with "hello"
    Then the edit should fail with a no-op error
    And the error should indicate no change would be made

  Scenario: Edit handles CRLF line endings transparently
    Given "src/main.rs" has CRLF line endings
    When the worker edits "src/main.rs" replacing "hello" with "world" using LF in the request
    Then the edit should succeed
    And the file should retain its CRLF line endings

  Scenario: Edit handles BOM-prefixed files safely
    Given "src/main.rs" starts with a UTF-8 BOM
    When the worker edits "src/main.rs" replacing "hello" with "world"
    Then the edit should succeed
    And the BOM should be preserved in the output file

  Scenario: Edit normalizes smart punctuation in match
    Given "README.md" contains a smart quote character
    When the worker edits "README.md" using the ASCII equivalent in the search string
    Then the edit should succeed via smart punctuation normalization fallback

  Scenario: Fuzzy match fallback when exact match fails
    Given "src/main.rs" has trailing whitespace differences from the search string
    When the worker edits "src/main.rs" with exact match disabled and fuzzy enabled
    Then the edit should succeed via fuzzy matching
    And the result should indicate fuzzy match was used

  # --- edit_preview ---

  Scenario: Edit preview computes diff without writing
    When the worker previews an edit to "src/main.rs" replacing "hello" with "world"
    Then the result should include a unified diff
    But the file "src/main.rs" should not be modified on disk

  Scenario: Edit preview detects ambiguity without modifying file
    Given "src/lib.rs" contains "a" in multiple locations
    When the worker previews an edit replacing "a" with "x"
    Then the result should report an ambiguity error
    And the file should not be modified

  # --- grep_content ---

  Scenario: Grep finds matches with file and line numbers
    When the worker greps for pattern "fn " in the repo
    Then the result should include matches in "src/main.rs" and "src/lib.rs"
    And each match should include file path and line number

  Scenario: Grep respects .gitignore
    Given a file "target/debug/output.log" exists in the repo
    When the worker greps for pattern "content" in the repo
    Then the results should not include files under "target/"
    And the results should not include "*.log" files

  Scenario: Grep supports substring patterns
    When the worker greps for pattern "assert_eq!(add(1, 2)" in the repo
    Then the result should include a match in "tests/test_add.rs"

  Scenario: Grep returns empty result for no matches
    When the worker greps for pattern "nonexistent_function_xyz" in the repo
    Then the result should indicate no matches found
    And the result should not be an error

  # --- find_files ---

  Scenario: Find files by glob pattern
    When the worker finds files matching "src/**/*.rs"
    Then the result should include "src/main.rs" and "src/lib.rs"
    And the result should not include "tests/test_add.rs"

  Scenario: Find respects .gitignore
    Given directories "target/debug/" exist with files inside
    When the worker finds files matching "**/*"
    Then the results should not include files under "target/"

  Scenario: Find returns stable ordering
    When the worker finds files matching "**/*.rs"
    Then the results should be sorted alphabetically

  # --- Safe git wrappers ---

  Scenario: Worker runs git status
    Given the worker has modified "src/main.rs"
    When the worker runs the git_status tool
    Then the result should show "src/main.rs" as modified

  Scenario: Worker runs git diff
    Given the worker has modified "src/main.rs"
    When the worker runs the git_diff tool
    Then the result should include a unified diff for "src/main.rs"

  Scenario: Worker runs git add and commit
    Given the worker has modified "src/main.rs"
    When the worker runs git_add for "src/main.rs"
    And the worker runs git_commit with message "fix: update greeting"
    Then the commit should succeed
    And git log should show the new commit

  Scenario: Worker can create and switch branches
    When the worker runs git_branch to create "feature/new-thing"
    Then the branch should be created
    And the worker should be able to switch to it

  Scenario: Destructive git commands are blocked by default
    When the worker attempts to run "git push --force"
    Then the command should be blocked
    And the error should indicate destructive git operations are not allowed

  Scenario: Git reset --hard is blocked by default
    When the worker attempts to run "git reset --hard HEAD~5"
    Then the command should be blocked
    And the error should reference the safety policy

  Scenario: Git clean -fd is blocked by default
    When the worker attempts to run "git clean -fd"
    Then the command should be blocked

  # --- read_file with pagination ---

  Scenario: Read file with line offset and limit
    When the worker reads "src/main.rs" with offset 1 and limit 1
    Then the result should contain only one line
    And the result should include truncation metadata indicating more lines exist

  Scenario: Read file returns continuation hint for large files
    Given a file "src/large.rs" with 500 lines
    When the worker reads "src/large.rs" with default limit
    Then the result should include a continuation hint for the next offset

  # --- Tool boundary enforcement ---

  Scenario: Worker tools cannot access files outside job directory
    When the worker attempts to read "/etc/passwd"
    Then the read should fail with a path violation error

  Scenario: Worker tools cannot write files outside job directory
    When the worker attempts to write to "/tmp/escape.txt"
    Then the write should fail with a path violation error

  Scenario: Worker exec tool inherits nsjail resource limits
    When the worker runs an exec command
    Then the command should run within the same nsjail sandbox
    And the command should be subject to the job's resource limits
