@done
Feature: REPL — Interactive Conversational Mode
  As a user
  I want to run quecto with no arguments to enter an interactive session
  So that I can have a back-and-forth conversation with the agent in my terminal

  Scenario: Entering REPL with no arguments
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Hello! How can I help?"
    When I start quecto in REPL mode
    And I type "Hi there"
    And I type "/exit"
    Then stdout should contain "Hello! How can I help?"
    And the exit code should be 0

  Scenario: REPL shows a welcome banner on startup
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start quecto in REPL mode
    And I type "/exit"
    Then stdout should contain "quecto"
    And stdout should contain "Type /help for commands"

  Scenario: REPL with named session persists conversation
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "First reply"
    When I start quecto in REPL mode with flags "-s myrepl"
    And I type "Hello"
    And I type "/exit"
    Then a session file for "myrepl" should exist in the base directory
    And the session should contain the user message "Hello"
    And the session should contain the assistant message "First reply"

  Scenario: REPL with system prompt
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Arrr, I be a pirate!"
    When I start quecto in REPL mode with flags "--system 'You are a pirate'"
    And I type "Who are you?"
    And I type "/exit"
    Then stdout should contain "Arrr, I be a pirate!"

  Scenario: REPL with model override
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Custom model reply"
    When I start quecto in REPL mode with flags "--model gpt-5-mini"
    And I type "Hi"
    And I type "/exit"
    Then stdout should contain "Custom model reply"

  Scenario: REPL multi-turn conversation
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns sequential responses:
      | The capital of France is Paris.   |
      | The capital of Germany is Berlin.  |
    When I start quecto in REPL mode
    And I type "What is the capital of France?"
    And I type "What about Germany?"
    And I type "/exit"
    Then stdout should contain "Paris"
    And stdout should contain "Berlin"

  Scenario: REPL exits on Ctrl+D (EOF)
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start quecto in REPL mode
    And I send EOF
    Then the exit code should be 0

  Scenario: REPL /help command shows available commands
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start quecto in REPL mode
    And I type "/help"
    And I type "/exit"
    Then stdout should contain "/exit"
    And stdout should contain "/help"
    And stdout should contain "/clear"

  Scenario: REPL /clear resets conversation history
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns sequential responses:
      | First response  |
      | After clear     |
    When I start quecto in REPL mode with flags "-s cleartest"
    And I type "Hello"
    And I type "/clear"
    And I type "Fresh start"
    And I type "/exit"
    Then stdout should contain "After clear"
    And the session should contain 1 user message

  Scenario: REPL handles LLM tool calls
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call for "bash" with args '{"command": "echo hello"}'
    And then the mock LLM returns a text response "The command output was: hello"
    When I start quecto in REPL mode
    And I type "Run echo hello"
    And I type "/exit"
    Then stdout should contain "hello"

  Scenario: REPL handles provider errors gracefully
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns an HTTP 500 error
    When I start quecto in REPL mode
    And I type "Hello"
    And I type "/exit"
    Then stdout should contain "Error"
    And the exit code should be 0

  Scenario: Empty input is ignored in REPL
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start quecto in REPL mode
    And I type ""
    And I type "/exit"
    Then the exit code should be 0

  @done
  Scenario: REPL progress callback fires ToolStarted and ToolFinished events during tool call
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call for "bash" with args '{"command": "echo hi"}'
    And then the mock LLM returns a text response "Done"
    When I run the REPL with a progress recorder
    And I type "Run echo hi"
    And I type "/exit"
    Then the progress recorder should have received a "ToolStarted" event for tool "bash"
    And the progress recorder should have received a "ToolFinished" event for tool "bash"

  @done
  Scenario: REPL progress callback fires Thinking and Done events
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Hello!"
    When I run the REPL with a progress recorder
    And I type "Say hi"
    And I type "/exit"
    Then the progress recorder should have received a "Thinking" event
    And the progress recorder should have received a "Done" event

  @done
  Scenario: REPL progress callback is not fired for non-agent slash commands
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run the REPL with a progress recorder
    And I type "/help"
    And I type "/exit"
    Then the progress recorder should have received 0 progress events

  @done
  Scenario: REPL spinner renders tool names on TTY stderr during agentic run
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call for "bash" with args '{"command": "echo hi"}'
    And then the mock LLM returns a text response "Done"
    When I start quecto in REPL mode as a TTY
    And I type "Run it"
    And I type "/exit"
    Then stderr should contain "bash"

  Scenario: REPL spinner renders tool arguments on TTY stderr during agentic run
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call for "bash" with args '{"command": "echo hi"}'
    And then the mock LLM returns a text response "Done"
    When I start quecto in REPL mode as a TTY
    And I type "Run it"
    And I type "/exit"
    Then stderr should contain "echo hi"

  @done
  Scenario: REPL progress output does not appear on stdout
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call for "bash" with args '{"command": "echo hi"}'
    And then the mock LLM returns a text response "All done"
    When I start quecto in REPL mode as a TTY
    And I type "Run it"
    And I type "/exit"
    Then stdout should contain "All done"
    And stdout should not contain "⠋"
    And stdout should not contain "⠙"
