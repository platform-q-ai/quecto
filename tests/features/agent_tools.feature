@done
Feature: Agent Tool System
  As an AI agent
  I want access to tools for interacting with the system
  So that I can help users with real-world tasks

  Scenario: Execute a shell command
    Given a tool workspace
    When the agent executes tool "exec" with args:
      | command | echo hello |
    Then the tool result should contain "hello"
    And the tool result should not be an error

  Scenario: Read a file
    Given a tool workspace
    And a file "notes.txt" exists with content "important notes"
    When the agent executes tool "read_file" with args:
      | path | notes.txt |
    Then the tool result should contain "important notes"
    And the tool result should not be an error

  Scenario: Write a file
    Given a tool workspace
    When the agent executes tool "write_file" with args:
      | path    | output.txt   |
      | content | hello world  |
    Then the file "output.txt" should exist in the workspace
    And the file "output.txt" should contain "hello world"

  Scenario: Edit a file
    Given a tool workspace
    And a file "code.py" exists with content "print('hello')"
    When the agent executes tool "edit_file" with args:
      | path    | code.py          |
      | old     | hello            |
      | new     | world            |
    Then the file "code.py" should contain "print('world')"

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
    Then the tool registry should contain "exec"
    And the tool registry should contain "read_file"
    And the tool registry should contain "write_file"
    And the tool registry should contain "edit_file"
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

  @pending
  Scenario: Web search returns results
    Given a web search tool configured with DuckDuckGo
    When the agent executes tool "web_search" with args:
      | query | rust programming language |
    Then the tool result should contain search results
    And each result should have a title and URL
