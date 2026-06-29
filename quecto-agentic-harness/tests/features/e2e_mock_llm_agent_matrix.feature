@done @mock-llm
Feature: E2E Mock LLM Agent Matrix
  Deterministic no-cost coverage for representative agent CLI flows that used
  to require live LLM provider calls.

  Scenario: Mocked OpenAI agent returns sentinel token
    Given a mocked OpenAI workspace is configured
    And the mock provider returns a text response "MATRIX_A1"
    When I run quecto agent -s - -m "Reply with the token MATRIX_A1 in your response"
    Then the exit code should be 0
    And stdout should contain "MATRIX_A1"

  Scenario: Mocked OpenAI agent writes a file via tool call
    Given a mocked OpenAI workspace is configured
    And the mock LLM first returns a tool call for "write" with args:
      | path    | matrix1.txt   |
      | content | MATRIX_FILE_1 |
    And the mock LLM then returns a text response "File written"
    When I run quecto agent -s - -m "Create file matrix1.txt with content MATRIX_FILE_1"
    Then the exit code should be 0
    And the file "matrix1.txt" should exist in the e2e workspace
    And the file "matrix1.txt" in the e2e workspace should contain "MATRIX_FILE_1"

  Scenario: Mocked Anthropic agent returns sentinel token
    Given a mocked Anthropic workspace is configured
    And the mock provider returns a text response "ANTHROPIC_MOCK_OK"
    When I run quecto agent --model "anthropic-api/claude-sonnet-4-6" -s - -m "Reply with ANTHROPIC_MOCK_OK"
    Then the exit code should be 0
    And stdout should contain "ANTHROPIC_MOCK_OK"
