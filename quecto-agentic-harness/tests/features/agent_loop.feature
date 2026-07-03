@done
Feature: Agent Loop
  As the core orchestration engine
  I want to process messages through an LLM and execute tool calls
  So that the agent can reason and act on behalf of the user

  Scenario: Simple message without tool calls
    Given a configured agent with a mock LLM
    And the LLM returns a plain text response "The answer is 42"
    When the agent processes [message] "What is the answer?"
    Then the response should be "The answer is 42"

  Scenario: Message triggers a single tool call
    Given a configured agent with a mock LLM
    And the LLM returns a tool call for "read" with args:
      | path | notes.txt |
    And the tool "read" returns "Buy groceries"
    And the LLM then returns "Your notes say: Buy groceries"
    When the agent processes [message] "What are my notes?"
    Then the response should be "Your notes say: Buy groceries"

  Scenario: Long Unicode tool output has a safe progress preview
    Given a configured agent with a mock LLM
    And the LLM returns a tool call for "read" with args:
      | path | notes.txt |
    And the tool "read" returns long Unicode output
    And the LLM then returns "Done"
    When the agent reports progress while processing [message] "Read my notes"
    Then the tool result preview should contain only complete characters
    And the tool result preview should stay within the allowed display length

  Scenario Outline: Tool result preview respects display length boundaries
    Given a configured agent with a mock LLM
    And the LLM returns a tool call for "read" with args:
      | path | notes.txt |
    And the tool "read" returns output <position> the allowed display length
    And the LLM then returns "Done"
    When the agent reports progress while processing [message] "Read my notes"
    Then the tool result preview should match output <position> the allowed display length

    Examples:
      | position   |
      | below      |
      | exactly at |
      | above      |

  Scenario: Headless tool execution does not prepare a progress preview
    Given a configured agent with a mock LLM
    And the LLM returns a tool call for "read" with args:
      | path | notes.txt |
    And the tool "read" returns long Unicode output
    And the LLM then returns "Done"
    When the agent processes [message] "Read my notes"
    Then no progress preview should be reported

  Scenario: Headless multi-tool turns reuse the assistant tool requests
    Given a configured agent with a mock LLM
    And the LLM returns simultaneous tool calls for "read" and "write"
    And the LLM then returns "Done"
    When the agent processes [message] "Copy my notes to output.txt"
    Then the assistant tool requests should be reused for execution

  Scenario: Unchanged instructions are reused across turns
    Given an agent with stable dynamic instructions
    And the LLM returns a plain text response "First done"
    And the LLM then returns "Second done"
    When the agent processes two messages while the instructions stay the same
    Then the agent should reuse the unchanged instructions

  Scenario: Message triggers multiple tool calls in one turn
    Given a configured agent with a mock LLM
    And the LLM returns simultaneous tool calls for "read" and "write"
    And the LLM then returns "Done"
    When the agent reports progress while processing [message] "Copy my notes to output.txt"
    Then both tools should be executed in order
    And the completed turn should include both tool results in order
    And the final response should confirm completion

  Scenario: Tool iteration limit prevents infinite loops
    Given a configured agent with max_tool_iterations 3
    And the LLM always returns a tool call
    When the agent processes [message] "Do something"
    Then the agent should stop after 3 tool iterations
    And the response should indicate the iteration limit was reached

  Scenario: Agent includes tool definitions in LLM request
    Given a configured agent with tools "bash" and "read"
    When the agent sends a request to the LLM
    Then the request should include tool definitions for "bash" and "read"
    And each tool definition should have name, description, and parameters

  Scenario: Agent provides startup info
    Given a fully initialized agent
    When I query the startup info
    Then it should report the number of loaded tools
