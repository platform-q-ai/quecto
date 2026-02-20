Feature: E2E Real LLM
  End-to-end tests that call a real OpenAI endpoint.
  These scenarios are gated by the @real-llm tag and only run when
  QUECTO_REAL_LLM=1 is set. They require OPENAI_API_KEY in the
  environment and are excluded from normal CI runs.

  Background:
    Given a real LLM workspace is configured

  @done @real-llm
  Scenario: Simple text response from real LLM
    When I run the real LLM agent with message "Reply with exactly the word PONG and nothing else"
    Then the exit code should be 0
    And stdout should not be empty

  @done @real-llm
  Scenario: Real LLM writes a file via tool use
    When I run the real LLM agent with message "Create a file called hello.txt containing the text 'Hello from LLM' and nothing else. Do not include any other text in the file."
    Then the exit code should be 0
    And the file "hello.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: Real LLM reads and summarises a file
    Given a file "data.txt" in the e2e workspace with content "The quick brown fox jumps over the lazy dog"
    When I run the real LLM agent with message "Read the file data.txt and tell me what animal jumps. Reply with just the animal name."
    Then the exit code should be 0
    And stdout should not be empty

  @done @real-llm
  Scenario: Real LLM executes a shell command
    When I run the real LLM agent with message "Run the command 'echo HELLO_QUECTO' and tell me what it printed. Reply with just the output."
    Then the exit code should be 0
    And stdout should contain "HELLO_QUECTO"
