Feature: End-to-End Subprocess Protocol
  As a parent agent or orchestration system
  I want to spawn Quecto as a child process with controlled flags
  So that subagents run in isolation with inherited config and bounded resources

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Basic subprocess invocation ---

  @pending
  Scenario: Parent process spawns child agent via CLI
    Given the mock LLM returns a text response "Subtask complete"
    When I spawn quecto as a subprocess with args: agent -s child-001 -m "Do the subtask"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "Subtask complete"
    And a session file should exist for key "cli:child-001"

  @pending
  Scenario: Child process inherits QUECTO_BASE_DIR from parent
    Given the mock LLM returns a text response "Inherited config"
    When I set QUECTO_BASE_DIR to the temp directory
    And I spawn quecto as a subprocess with args: agent -s - -m "test"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "Inherited config"

  # --- Session isolation ---

  @pending
  Scenario: Parent and child use separate sessions
    Given the mock LLM returns a text response "Parent response"
    When I run quecto agent -s parent -m "Parent message"
    And the mock LLM returns a text response "Child response"
    And I spawn quecto as a subprocess with args: agent -s child -m "Child message"
    Then the session "cli:parent" should not contain "Child message"
    And the session "cli:child" should not contain "Parent message"

  @pending
  Scenario: Ephemeral child leaves no session trace
    Given the mock LLM returns a text response "No trace"
    When I spawn quecto as a subprocess with args: agent -s - -m "Ephemeral task"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "No trace"
    And no session files should exist

  # --- Resource limits on child processes ---

  @pending
  Scenario: Child process respects max-iterations flag
    Given the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I spawn quecto as a subprocess with args: agent -s - --max-iterations 2 -m "Loop"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "iteration limit"

  @pending
  Scenario: Child process respects max-time flag
    Given the mock LLM takes 5 seconds to respond
    When I spawn quecto as a subprocess with args: agent -s - --max-time 1 -m "Slow"
    Then the subprocess exit code should be 2
    And the subprocess stderr should contain "max-time exceeded"

  # --- System prompt injection for child agents ---

  @pending
  Scenario: Parent injects task context via system prompt
    Given the mock LLM returns a text response "Task understood"
    When I spawn quecto as a subprocess with args: agent -s child-task --system "You are a research agent" -m "Research topic X"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "Task understood"

  @pending
  Scenario: Child agent uses different model than parent
    Given the mock LLM returns a text response "Using mini model"
    When I spawn quecto as a subprocess with args: agent -s - --model gpt-5-mini -m "Quick task"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "Using mini model"

  # --- Error propagation ---

  @pending
  Scenario: Child process failure is detectable by parent via exit code
    Given the mock LLM returns an HTTP 500 error
    When I spawn quecto as a subprocess with args: agent -s - -m "Fail"
    Then the subprocess exit code should be 1

  @pending
  Scenario: Child process timeout is detectable by parent via exit code
    Given the mock LLM takes 5 seconds to respond
    When I spawn quecto as a subprocess with args: agent -s - --max-time 1 -m "Timeout"
    Then the subprocess exit code should be 2
    And the subprocess stderr should contain "max-time exceeded"
