@done @mock-llm
Feature: E2E Mock LLM agent flows (zero-cost copy of the live @real-llm suite)
  Deterministic, no-network reproductions of the behaviours the @real-llm
  e2e suite asserts. Every scenario drives the real `quecto agent` entry point
  against a WireMock-backed provider (see configure_mock_provider_workspace and
  the mock provider response steps in tests/bdd/e2e_steps.rs), so the suite
  makes ZERO paid provider calls and passes with no API key present. This is the
  default pre-push e2e lane; the live @real-llm suite is retained for occasional
  on-demand validation (see docs/real-llm-mocking-plan.md).

  Scenario: Mocked agent returns a plain text token
    Given a mocked OpenAI workspace is configured
    And the mock LLM returns a text response "MOCK_TEXT_OK"
    When I run quecto agent -s - -m "Reply with the token MOCK_TEXT_OK"
    Then the exit code should be 0
    And stdout should contain "MOCK_TEXT_OK"

  Scenario: Mocked agent honours a system prompt style marker
    Given a mocked OpenAI workspace is configured
    And the mock LLM returns a text response "STYLE_OK greetings"
    When I run quecto agent --system "Always include STYLE_OK in your response" -m "Say hello"
    Then the exit code should be 0
    And stdout should contain "STYLE_OK"

  Scenario: Mocked agent writes a file via a write tool-call
    Given a mocked OpenAI workspace is configured
    And the mock LLM first returns a tool call for "write" with args:
      | path    | mock1.txt     |
      | content | MOCK_FILE_1   |
    And the mock LLM then returns a text response "File written"
    When I run quecto agent -s - -m "Create file mock1.txt with content MOCK_FILE_1"
    Then the exit code should be 0
    And the file "mock1.txt" should exist in the e2e workspace
    And the file "mock1.txt" in the e2e workspace should contain "MOCK_FILE_1"

  Scenario: Mocked agent writes a nested file path
    Given a mocked OpenAI workspace is configured
    And the mock LLM first returns a tool call for "write" with args:
      | path    | notes/mock2.txt |
      | content | MOCK_FILE_2     |
    And the mock LLM then returns a text response "Nested file written"
    When I run quecto agent -s - -m "Create file notes/mock2.txt with content MOCK_FILE_2"
    Then the exit code should be 0
    And the file "notes/mock2.txt" should exist in the e2e workspace

  Scenario: Mocked agent reads a file then reports a token
    Given a mocked OpenAI workspace is configured
    And a file "facts.txt" in the e2e workspace with content "token=READ_X9"
    And the mock LLM first returns a tool call for "read" with args:
      | path | facts.txt |
    And the mock LLM then returns a text response "The file contains READ_X9"
    When I run quecto agent -s - -m "Read facts.txt and include READ_X9 in your response"
    Then the exit code should be 0
    And stdout should contain "READ_X9"

  Scenario: Mocked agent edits a file value
    Given a mocked OpenAI workspace is configured
    And a file "mode.txt" in the e2e workspace with content "mode=alpha"
    And the mock LLM first returns a tool call for "edit" with args:
      | path    | mode.txt |
      | oldText | alpha    |
      | newText | beta     |
    And the mock LLM then returns a text response "Edited"
    When I run quecto agent -s - -m "Edit mode.txt so alpha becomes beta"
    Then the exit code should be 0
    And the file "mode.txt" in the e2e workspace should contain "mode=beta"

  Scenario: Mocked agent runs a shell command and echoes a marker
    Given a mocked OpenAI workspace is configured
    And the mock LLM first returns a tool call for "bash" with args:
      | command | echo MOCK_EXEC_1 |
    And the mock LLM then returns a text response "The command printed MOCK_EXEC_1"
    When I run quecto agent -s - -m "Run echo MOCK_EXEC_1 and include MOCK_EXEC_1"
    Then the exit code should be 0
    And stdout should contain "MOCK_EXEC_1"

  Scenario: Mocked agent chains read then write in one task
    Given a mocked OpenAI workspace is configured
    And a file "num.txt" in the e2e workspace with content "value=2468"
    And the mock LLM returns a tool call sequence:
      | call | read  | {"path":"num.txt"}                          |
      | call | write | {"path":"out-num.txt","content":"2468"}     |
      | text | Done extracting 2468 | |
    When I run quecto agent -s - -m "Read num.txt and create out-num.txt containing 2468"
    Then the exit code should be 0
    And the file "out-num.txt" should exist in the e2e workspace
    And the file "out-num.txt" in the e2e workspace should contain "2468"

  Scenario: Mocked agent uses multiple tools in one task
    Given a mocked OpenAI workspace is configured
    And the mock LLM returns a tool call sequence:
      | call | write | {"path":"mock-a.txt","content":"AVAL"} |
      | call | write | {"path":"mock-b.txt","content":"BVAL"} |
      | text | Created both files | |
    When I run quecto agent -s - -m "Create files mock-a.txt with AVAL and mock-b.txt with BVAL"
    Then the exit code should be 0
    And the file "mock-a.txt" should exist in the e2e workspace
    And the file "mock-b.txt" should exist in the e2e workspace

  Scenario: Mocked agent recovers from a tool error
    Given a mocked OpenAI workspace is configured
    And the mock LLM returns a tool call sequence:
      | call | read | {"path":"no-such-file-77.txt"}          |
      | text | FALLBACK_77 the file was missing | |
    When I run quecto agent -s - -m "Try reading no-such-file-77.txt. If missing, reply FALLBACK_77"
    Then the exit code should be 0
    And stdout should contain "FALLBACK_77"

  Scenario: Mocked agent remembers context across session turns
    Given a mocked OpenAI workspace is configured
    And the mock LLM returns a text response "ACK_KIWI noted"
    When I run quecto agent -s memkiwi -m "Remember phrase kiwi river. Reply ACK_KIWI"
    Then the exit code should be 0
    And stdout should contain "ACK_KIWI"
    And the session "cli:memkiwi" should contain at least 2 messages
    When I run quecto agent -s memkiwi -m "What phrase did I ask you to remember?"
    Then the exit code should be 0
    And the session "cli:memkiwi" should contain text "kiwi river"
