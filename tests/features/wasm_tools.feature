@done
Feature: WASM Tool Ports
  As a developer
  I want each built-in tool ported to a WASM component
  So that tools run in isolated containers with declared capabilities only

  # --- Filesystem tools ---

  @done
  Scenario: WASM ReadFileTool reads a file from the workspace
    Given a WASM-containerized tool registry
    And a WASM workspace file "notes.txt" with content "buy milk"
    When the agent executes WASM tool "read" with args:
      | path | notes.txt |
    Then the WASM tool result should contain "buy milk"
    And the WASM tool result should not be an error

  @done
  Scenario: WASM ReadFileTool rejects paths outside workspace
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "read" with args:
      | path | /etc/passwd |
    Then the WASM tool result should be an error

  @done
  Scenario: WASM ReadFileTool enforces file size limit
    Given a WASM-containerized tool registry
    And a WASM workspace file "huge.txt" larger than 1 MiB
    When the agent executes WASM tool "read" with args:
      | path | huge.txt |
    Then the WASM tool result should be an error
    And the WASM error should mention "too large"

  @done
  Scenario: WASM WriteFileTool creates a file in the workspace
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "write" with args:
      | path    | output.txt  |
      | content | hello world |
    Then the WASM workspace file "output.txt" should exist
    And the WASM workspace file "output.txt" should contain "hello world"

  @done
  Scenario: WASM WriteFileTool creates parent directories
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "write" with args:
      | path    | sub/dir/file.txt |
      | content | nested content   |
    Then the WASM workspace file "sub/dir/file.txt" should exist

  @done
  Scenario: WASM EditTool replaces content in a file
    Given a WASM-containerized tool registry
    And a WASM workspace file "code.py" with content "print('hello')"
    When the agent executes WASM tool "edit" with args:
      | path    | code.py  |
      | oldText | hello    |
      | newText | goodbye  |
    Then the WASM workspace file "code.py" should contain "print('goodbye')"

  @done
  Scenario: WASM EditTool fails when substring not found
    Given a WASM-containerized tool registry
    And a WASM workspace file "code.py" with content "print('hello')"
    When the agent executes WASM tool "edit" with args:
      | path    | code.py     |
      | oldText | nonexistent |
      | newText | replacement |
    Then the WASM tool result should be an error

  @done
  Scenario: WASM AppendFileTool appends to an existing file
    Given a WASM-containerized tool registry
    And a WASM workspace file "log.txt" with content "line1\n"
    When the agent executes WASM tool "append_file" with args:
      | path    | log.txt |
      | content | line2\n |
    Then the WASM workspace file "log.txt" should contain "line1\nline2\n"

  @done
  Scenario: WASM AppendFileTool creates file if it does not exist
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "append_file" with args:
      | path    | new.txt     |
      | content | first line  |
    Then the WASM workspace file "new.txt" should exist
    And the WASM workspace file "new.txt" should contain "first line"

  @done
  Scenario: WASM ListDirTool lists workspace directory contents
    Given a WASM-containerized tool registry
    And a WASM workspace containing files "a.txt", "b.txt", and directory "subdir"
    When the agent executes WASM tool "list_dir" with args:
      | path | . |
    Then the WASM tool result should contain "a.txt"
    And the WASM tool result should contain "b.txt"
    And the WASM tool result should contain "subdir/"

  # --- CronTool ---

  @done
  Scenario: WASM CronTool adds a cron job
    Given a WASM-containerized tool registry with a cron store
    When the agent executes WASM tool "cron" with args:
      | action           | add             |
      | name             | daily-check     |
      | message          | run diagnostics |
      | interval_seconds | 86400           |
    Then the WASM tool result should not be an error
    And the WASM tool result should contain "cron op"

  @done
  Scenario: WASM CronTool lists cron jobs
    Given a WASM-containerized tool registry with a cron store
    And the WASM cron store contains a job named "hourly-ping"
    When the agent executes WASM tool "cron" with args:
      | action | list |
    Then the WASM tool result should contain "hourly-ping"

  @done
  Scenario: WASM CronTool removes a cron job
    Given a WASM-containerized tool registry with a cron store
    And the WASM cron store contains a job named "old-job"
    When the agent executes WASM tool "cron" with args:
      | action | remove  |
      | name   | old-job |
    Then the WASM tool result should not be an error
    And the WASM tool result should contain "cron op"

  # --- RecallTool ---

  @done
  Scenario: WASM RecallTool retrieves a spilled entry
    Given a WASM-containerized tool registry with a spill store
    And the WASM spill store contains entry "spill-42" with content "large tool output from earlier"
    When the agent executes WASM tool "recall" with args:
      | id | spill-42 |
    Then the WASM tool result should contain "large tool output from earlier"

  @done
  Scenario: WASM RecallTool lists all spill entries
    Given a WASM-containerized tool registry with a spill store
    And the WASM spill store contains entries "spill-1" and "spill-2"
    When the agent executes WASM tool "recall" with args:
      | id | list |
    Then the WASM tool result should contain "spill-1"
    And the WASM tool result should contain "spill-2"

  @done
  Scenario: WASM RecallTool returns error for unknown spill ID
    Given a WASM-containerized tool registry with an empty spill store
    When the agent executes WASM tool "recall" with args:
      | id | nonexistent |
    Then the WASM tool result should be an error

  # --- MessageTool ---

  @done
  Scenario: WASM MessageTool sends a message through host channel
    Given a WASM-containerized tool registry with a message channel
    When the agent executes WASM tool "message" with args:
      | text | Hello from the agent |
    Then the WASM message channel should have received "Hello from the agent"

  @done
  Scenario: WASM MessageTool sends to explicit target
    Given a WASM-containerized tool registry with a message channel
    When the agent executes WASM tool "message" with args:
      | text   | Alert!        |
      | target | telegram:9999 |
    Then the WASM message channel should have received "Alert!" for target "telegram:9999"

  # --- WebSearchTool ---

  @done
  Scenario: WASM WebSearchTool searches via HTTP host import
    Given a WASM-containerized tool registry with HTTP allowlist for search APIs
    And a mock search API returning results for "rust wasm"
    When the agent executes WASM tool "web_search" with args:
      | query | rust wasm |
    Then the WASM tool result should contain search results
    And the WASM tool result should not be an error

  @done
  Scenario: WASM WebSearchTool rejects requests when host not in allowlist
    Given a WASM-containerized tool registry
    When the agent executes WASM tool "web_search" with args:
      | query | test query |
    Then the WASM tool result should be an error
    And the WASM error should mention "allowlist"

  # --- Behavioral parity with native tools ---

  @done
  Scenario: WASM tools produce identical output to native tools for read
    Given a native tool registry and a WASM-containerized tool registry
    And both registries share the same workspace with file "test.txt" containing "identical output test"
    When both registries execute "read" with args:
      | path | test.txt |
    Then the WASM parity results should be identical

  @done
  Scenario: WASM tools produce identical output to native tools for write
    Given a native tool registry and a WASM-containerized tool registry
    When both registries execute "write" with args:
      | path    | out.txt        |
      | content | parity check   |
    Then the WASM parity files should have identical content

  @done
  Scenario: WASM tools produce identical output to native tools for list_dir
    Given a native tool registry and a WASM-containerized tool registry
    And both registries share the same workspace with files "x.txt" and "y.txt"
    When both registries execute "list_dir" with args:
      | path | . |
    Then the WASM parity results should be identical
