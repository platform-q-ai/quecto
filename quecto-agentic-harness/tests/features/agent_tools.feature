@done
Feature: Agent Tool System
  As an AI agent
  I want access to tools for interacting with the system
  So that I can help users with real-world tasks

  Scenario: Execute a shell command
    Given a tool workspace
    When the agent executes tool "bash" with args:
      | command | echo hello |
    Then the [ToolResult] should contain "hello"
    And the [ToolResult] should not be an error

  @agent-tools
  Scenario: Execute large output command truncates to tail
    Given a tool workspace with exec timeout 1 second
    When the agent executes tool "bash" with args:
      | command | printf 'x%.0s' {1..100000} |
    Then the [ToolResult] should contain "x"
    And the [ToolResult] should not be an error

  Scenario: bash truncates large output and provides temp file hint
    Given a tool workspace
    When the agent executes tool "bash" with args:
      | command | seq 1 3000 |
    Then the [ToolResult] should contain "3000"
    And the [ToolResult] should contain "[Showing lines"
    And the [ToolResult] should not be an error

  Scenario: Read a file
    Given a tool workspace
    And a file "notes.txt" exists with content "important notes"
    When the agent executes tool "read" with args:
      | path | notes.txt |
    Then the [ToolResult] should contain "important notes"
    And the [ToolResult] should not be an error

  Scenario: Read a file with offset pagination
    Given a tool workspace
    And a file "multi.txt" exists with content "line1\nline2\nline3\nline4\nline5"
    When the agent executes tool "read" with args:
      | path   | multi.txt |
      | offset | 3         |
    Then the [ToolResult] should contain "line3"
    And the [ToolResult] should not be an error

  Scenario: Read a large file truncates with continuation hint
    Given a tool workspace
    And a large file "big.txt" exists with 3000 lines
    When the agent executes tool "read" with args:
      | path | big.txt |
    Then the [ToolResult] should contain "[Showing lines"
    And the [ToolResult] should not be an error

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
    And the [ToolResult] should contain "+1 print('world')"

  Scenario: Edit rejects ambiguous match
    Given a tool workspace
    And a file "dup.py" exists with content "x = 1\nx = 1"
    When the agent executes tool "edit" with args:
      | path    | dup.py |
      | oldText | x = 1  |
      | newText | x = 2  |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "matches 2"

  Scenario: Edit handles BOM and CRLF normalisation
    Given a tool workspace
    And a file "win.txt" exists with CRLF line endings and content "hello\nworld"
    When the agent executes tool "edit" with args:
      | path    | win.txt |
      | oldText | hello   |
      | newText | hi      |
    Then the file "win.txt" should contain "hi"
    And the [ToolResult] should not be an error


  Scenario: List a directory
    Given a tool workspace
    And a file "a.txt" exists with content "a"
    And a file "b.txt" exists with content "b"
    When the agent executes tool "ls" with args:
      | path | . |
    Then the [ToolResult] should contain "a.txt"
    And the [ToolResult] should contain "b.txt"

  Scenario: ls uses current directory when path is omitted
    Given a tool workspace
    And a file "hello.txt" exists with content "hi"
    When the agent executes tool "ls" with empty args
    Then the [ToolResult] should contain "hello.txt"
    And the [ToolResult] should not be an error

  Scenario: ls truncates when entry limit exceeded
    Given a tool workspace with 1100 files
    When the agent executes tool "ls" with args:
      | path | . |
    Then the [ToolResult] should contain "entries limit reached"
    And the [ToolResult] should not be an error

  Scenario: Tool registry lists core tools
    Given a tool workspace
    Then the tool registry should contain "bash"
    And the tool registry should contain "read"
    And the tool registry should contain "write"
    And the tool registry should contain "edit"
    And the tool registry should contain "ls"

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
    Then the [ToolResult] should contain "Rust Programming Language"
    And the [ToolResult] should contain "rust-lang.org"
    And the [ToolResult] should not be an error

  @done
  Scenario: Web search via Brave API returns results
    Given a tool workspace
    And a web search tool configured with a mock Brave Search API and api_key "bsk-test"
    And the mock Brave API returns results for "weather today":
      | title           | url                         |
      | Weather Today   | https://weather.example.com |
    When the agent executes tool "web_search" with args:
      | query | weather today |
    Then the [ToolResult] should contain "Weather Today"
    And the [ToolResult] should not be an error

  @done
  Scenario: Web search falls back to DuckDuckGo when Brave key is missing
    Given a tool workspace
    And a web search tool configured with no Brave API key
    And a mock DuckDuckGo API that returns results
    When the agent executes tool "web_search" with args:
      | query | test query |
    Then the [ToolResult] should contain search results
    And the search should have used DuckDuckGo

  @done
  Scenario: Web search handles API error gracefully
    Given a tool workspace
    And a web search tool configured with a mock DuckDuckGo API
    And the mock search API returns an HTTP 503 error
    When the agent executes tool "web_search" with args:
      | query | test query |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "Search failed"
