@pending
Feature: WASM Tool Ports
  As a developer
  I want each built-in tool ported to a WASM component
  So that tools run in isolated containers with declared capabilities only

  # --- Filesystem tools ---

  @pending
  Scenario: WASM ReadFileTool reads a file from the workspace
    Given a WASM-containerized tool registry
    And a workspace file "notes.txt" with content "buy milk"
    When the agent executes WASM tool "read_file" with args:
      | path | notes.txt |
    Then the tool result should contain "buy milk"
    And the tool result should not be an error

  @pending
  Scenario: WASM ReadFileTool rejects paths outside workspace
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "read_file" with args:
      | path | /etc/passwd |
    Then the tool result should be an error

  @pending
  Scenario: WASM ReadFileTool enforces file size limit
    Given a WASM-containerized tool registry
    And a workspace file "huge.txt" larger than 1 MiB
    When the agent executes WASM tool "read_file" with args:
      | path | huge.txt |
    Then the tool result should be an error
    And the error should mention "too large"

  @pending
  Scenario: WASM WriteFileTool creates a file in the workspace
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "write_file" with args:
      | path    | output.txt  |
      | content | hello world |
    Then the file "output.txt" should exist in the workspace
    And the file "output.txt" should contain "hello world"

  @pending
  Scenario: WASM WriteFileTool creates parent directories
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "write_file" with args:
      | path    | sub/dir/file.txt |
      | content | nested content   |
    Then the file "sub/dir/file.txt" should exist in the workspace

  @pending
  Scenario: WASM EditFileTool replaces content in a file
    Given a WASM-containerized tool registry
    And a workspace file "code.py" with content "print('hello')"
    When the agent executes WASM tool "edit_file" with args:
      | path | code.py    |
      | old  | hello      |
      | new  | goodbye    |
    Then the file "code.py" should contain "print('goodbye')"

  @pending
  Scenario: WASM EditFileTool fails when substring not found
    Given a WASM-containerized tool registry
    And a workspace file "code.py" with content "print('hello')"
    When the agent executes WASM tool "edit_file" with args:
      | path | code.py    |
      | old  | nonexistent |
      | new  | replacement |
    Then the tool result should be an error

  @pending
  Scenario: WASM AppendFileTool appends to an existing file
    Given a WASM-containerized tool registry
    And a workspace file "log.txt" with content "line1\n"
    When the agent executes WASM tool "append_file" with args:
      | path    | log.txt |
      | content | line2\n |
    Then the file "log.txt" should contain "line1\nline2\n"

  @pending
  Scenario: WASM AppendFileTool creates file if it does not exist
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "append_file" with args:
      | path    | new.txt     |
      | content | first line  |
    Then the file "new.txt" should exist in the workspace
    And the file "new.txt" should contain "first line"

  @pending
  Scenario: WASM ListDirTool lists workspace directory contents
    Given a WASM-containerized tool registry
    And a workspace containing files "a.txt", "b.txt", and directory "subdir"
    When the agent executes WASM tool "list_dir" with args:
      | path | . |
    Then the tool result should contain "a.txt"
    And the tool result should contain "b.txt"
    And the tool result should contain "subdir/"

  # --- CronTool ---

  @pending
  Scenario: WASM CronTool adds a cron job
    Given a WASM-containerized tool registry with a cron store
    When the agent executes WASM tool "cron" with args:
      | action           | add           |
      | name             | daily-check   |
      | message          | run diagnostics |
      | interval_seconds | 86400         |
    Then the cron store should contain a job named "daily-check"

  @pending
  Scenario: WASM CronTool lists cron jobs
    Given a WASM-containerized tool registry with a cron store
    And the cron store contains a job named "hourly-ping"
    When the agent executes WASM tool "cron" with args:
      | action | list |
    Then the tool result should contain "hourly-ping"

  @pending
  Scenario: WASM CronTool removes a cron job
    Given a WASM-containerized tool registry with a cron store
    And the cron store contains a job named "old-job"
    When the agent executes WASM tool "cron" with args:
      | action | remove  |
      | name   | old-job |
    Then the cron store should not contain a job named "old-job"

  # --- RecallTool ---

  @pending
  Scenario: WASM RecallTool retrieves a spilled entry
    Given a WASM-containerized tool registry with a spill store
    And the spill store contains entry "spill-42" with content "large tool output from earlier"
    When the agent executes WASM tool "recall" with args:
      | id | spill-42 |
    Then the tool result should contain "large tool output from earlier"

  @pending
  Scenario: WASM RecallTool lists all spill entries
    Given a WASM-containerized tool registry with a spill store
    And the spill store contains entries "spill-1" and "spill-2"
    When the agent executes WASM tool "recall" with args:
      | id | list |
    Then the tool result should contain "spill-1"
    And the tool result should contain "spill-2"

  @pending
  Scenario: WASM RecallTool returns error for unknown spill ID
    Given a WASM-containerized tool registry with an empty spill store
    When the agent executes WASM tool "recall" with args:
      | id | nonexistent |
    Then the tool result should be an error

  # --- MessageTool ---

  @pending
  Scenario: WASM MessageTool sends a message through host channel
    Given a WASM-containerized tool registry with a message channel
    When the agent executes WASM tool "message" with args:
      | text | Hello from the agent |
    Then the message channel should have received "Hello from the agent"

  @pending
  Scenario: WASM MessageTool sends to explicit target
    Given a WASM-containerized tool registry with a message channel
    When the agent executes WASM tool "message" with args:
      | text   | Alert!        |
      | target | telegram:9999 |
    Then the message channel should have received "Alert!" for target "telegram:9999"

  # --- WebSearchTool ---

  @pending
  Scenario: WASM WebSearchTool searches via HTTP host import
    Given a WASM-containerized tool registry with HTTP allowlist for search APIs
    And a mock search API returning results for "rust wasm"
    When the agent executes WASM tool "web_search" with args:
      | query | rust wasm |
    Then the tool result should contain search results
    And the tool result should not be an error

  @pending
  Scenario: WASM WebSearchTool cannot make arbitrary HTTP requests
    Given a WASM-containerized tool registry with HTTP allowlist for search APIs only
    When the WASM web_search tool attempts to call http-request to "https://evil.com/exfil"
    Then the host should reject the request
    And the tool result should be an error

  # --- Behavioral parity with native tools ---

  @pending
  Scenario: WASM tools produce identical output to native tools for read_file
    Given a native tool registry and a WASM-containerized tool registry
    And both registries share the same workspace with file "test.txt" containing "identical output test"
    When both registries execute "read_file" with args:
      | path | test.txt |
    Then the results should be identical

  @pending
  Scenario: WASM tools produce identical output to native tools for write_file
    Given a native tool registry and a WASM-containerized tool registry
    When both registries execute "write_file" with args:
      | path    | out.txt        |
      | content | parity check   |
    Then the resulting files should have identical content

  @pending
  Scenario: WASM tools produce identical output to native tools for list_dir
    Given a native tool registry and a WASM-containerized tool registry
    And both registries share the same workspace with files "x.txt" and "y.txt"
    When both registries execute "list_dir" with args:
      | path | . |
    Then the results should be identical
