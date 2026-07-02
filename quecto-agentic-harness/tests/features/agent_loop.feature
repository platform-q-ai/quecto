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

  Scenario: Tool result preview handles multibyte output at the byte limit
    Given a configured agent with a mock LLM
    And the LLM returns a tool call for "read" with args:
      | path | notes.txt |
    And the tool "read" returns output whose byte limit falls inside a multibyte character
    And the LLM then returns "Done"
    When the agent reports progress while processing [message] "Read my notes"
    Then the tool result preview should contain only complete characters
    And the tool result preview should stay within the byte limit


  Scenario: Message triggers multiple tool calls in sequence
    Given a configured agent with a mock LLM
    And the LLM returns tool calls in sequence: "read", "write"
    When the agent processes [message] "Copy my notes to output.txt"
    Then both tools should be executed in order
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
