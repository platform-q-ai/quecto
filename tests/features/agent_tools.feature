@done
Feature: Agent Tool System
  As an AI agent
  I want access to tools for interacting with the system
  So that I can help users with real-world tasks

  Scenario: Execute a shell command
    Given a tool workspace
    When the agent executes tool "bash" with args:
      | command | echo hello |
    Then the tool result should contain "hello"
    And the tool result should not be an error

  @agent-tools
  Scenario: Execute large output command truncates to tail
    Given a tool workspace with exec timeout 1 second
    When the agent executes tool "bash" with args:
      | command | printf 'x%.0s' {1..100000} |
    Then the tool result should contain "x"
    And the tool result should not be an error

  Scenario: bash truncates large output and provides temp file hint
    Given a tool workspace
    When the agent executes tool "bash" with args:
      | command | seq 1 3000 |
    Then the tool result should contain "3000"
    And the tool result should contain "[Output truncated"
    And the tool result should not be an error

  Scenario: Read a file
    Given a tool workspace
    And a file "notes.txt" exists with content "important notes"
    When the agent executes tool "read" with args:
      | path | notes.txt |
    Then the tool result should contain "important notes"
    And the tool result should not be an error

  Scenario: Read a file with offset pagination
    Given a tool workspace
    And a file "multi.txt" exists with content "line1\nline2\nline3\nline4\nline5"
    When the agent executes tool "read" with args:
      | path   | multi.txt |
      | offset | 3         |
    Then the tool result should contain "line3"
    And the tool result should not be an error

  Scenario: Read a large file truncates with continuation hint
    Given a tool workspace
    And a large file "big.txt" exists with 3000 lines
    When the agent executes tool "read" with args:
      | path | big.txt |
    Then the tool result should contain "[Showing lines"
    And the tool result should not be an error

  Scenario: Write a file
    Given a tool workspace
    When the agent executes tool "write" with args:
      | path    | output.txt   |
      | content | hello world  |
    Then the file "output.txt" should exist in the workspace
    And the file "output.txt" should contain "hello world"

  Scenario: Edit a file
    Given a tool workspace
    And a file "code.py" exists with content "print('hello')"
    When the agent executes tool "edit" with args:
      | path    | code.py  |
      | oldText | hello    |
      | newText | world    |
    Then the file "code.py" should contain "print('world')"
    And the tool result should contain "@@"

  Scenario: Edit rejects ambiguous match
    Given a tool workspace
    And a file "dup.py" exists with content "x = 1\nx = 1"
    When the agent executes tool "edit" with args:
      | path    | dup.py |
      | oldText | x = 1  |
      | newText | x = 2  |
    Then the tool result should be an error
    And the tool result should contain "matches 2"

  Scenario: Edit handles BOM and CRLF normalisation
    Given a tool workspace
    And a file "win.txt" exists with CRLF line endings and content "hello\nworld"
    When the agent executes tool "edit" with args:
      | path    | win.txt |
      | oldText | hello   |
      | newText | hi      |
    Then the file "win.txt" should contain "hi"
    And the tool result should not be an error

  Scenario: Append to a file
    Given a tool workspace
    And a file "log.txt" exists with content "line1"
    When the agent executes tool "append_file" with args:
      | path    | log.txt |
      | content | line2   |
    Then the file "log.txt" should contain "line1"
    And the file "log.txt" should contain "line2"

  Scenario: List a directory
    Given a tool workspace
    And a file "a.txt" exists with content "a"
    And a file "b.txt" exists with content "b"
    When the agent executes tool "list_dir" with args:
      | path | . |
    Then the tool result should contain "a.txt"
    And the tool result should contain "b.txt"

  Scenario: Tool registry lists core tools
    Given a tool workspace
    Then the tool registry should contain "bash"
    And the tool registry should contain "read"
    And the tool registry should contain "write"
    And the tool registry should contain "edit"
    And the tool registry should contain "append_file"
    And the tool registry should contain "list_dir"

  Scenario: Message tool sends to channel via bus
    Given a message tool with default target "telegram:12345"
    When the agent sends a message "Task completed!" via the message tool
    Then the outbound bus should have a message for "telegram:12345" with text "Task completed!"

  Scenario: Spawn tool validates and creates subagent config
    Given a spawn tool with allowed agents "news-bot" and "weather-bot"
    When the agent executes the spawn tool with task "Summarize news"
    Then the spawn result should confirm the subagent was spawned

  Scenario: Spawn tool rejects disallowed agent
    Given a spawn tool with allowed agents "news-bot" and "weather-bot"
    When the agent executes the spawn tool with task "evil" and agent_id "evil-bot"
    Then the spawn result should be an error mentioning "not allowed"

  @done
  Scenario: Web search via DuckDuckGo returns results
    Given a tool workspace
    And a web search tool configured with a mock DuckDuckGo API
    And the mock search API returns results for "rust programming":
      | title                        | url                                  |
      | Rust Programming Language    | https://www.rust-lang.org/           |
      | Rust (programming language)  | https://en.wikipedia.org/wiki/Rust   |
    When the agent executes tool "web_search" with args:
      | query | rust programming |
    Then the tool result should contain "Rust Programming Language"
    And the tool result should contain "rust-lang.org"
    And the tool result should not be an error

  @done
  Scenario: Web search via Brave API returns results
    Given a tool workspace
    And a web search tool configured with a mock Brave Search API and api_key "bsk-test"
    And the mock Brave API returns results for "weather today":
      | title           | url                         |
      | Weather Today   | https://weather.example.com |
    When the agent executes tool "web_search" with args:
      | query | weather today |
    Then the tool result should contain "Weather Today"
    And the tool result should not be an error

  @done
  Scenario: Web search falls back to DuckDuckGo when Brave key is missing
    Given a tool workspace
    And a web search tool configured with no Brave API key
    And a mock DuckDuckGo API that returns results
    When the agent executes tool "web_search" with args:
      | query | test query |
    Then the tool result should contain search results
    And the search should have used DuckDuckGo

  @done
  Scenario: Web search handles API error gracefully
    Given a tool workspace
    And a web search tool configured with a mock DuckDuckGo API
    And the mock search API returns an HTTP 503 error
    When the agent executes tool "web_search" with args:
      | query | test query |
    Then the tool result should be an error
    And the tool result should contain "Search failed"
