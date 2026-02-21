Feature: E2E Real LLM Entrypoints Matrix
  Additional real endpoint checks through subprocess entry points.

  Background:
    Given a real LLM workspace is configured

  @done @real-llm
  Scenario: Agent subprocess sentinel E1
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s - -m "Reply with ENTRY_E1"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_E1"

  @done @real-llm
  Scenario: Agent subprocess sentinel E2
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s - -m "Reply with ENTRY_E2"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_E2"

  @done @real-llm
  Scenario: Agent subprocess creates file
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s - -m "Create file entry-file-a.txt containing ENTRY_FILE_A"
    Then the subprocess exit code should be 0
    And the file "entry-file-a.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: Agent subprocess reads prepared file
    Given a file "entry-read.txt" in the e2e workspace with content "ENTRY_READ_OK"
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s - -m "Read entry-read.txt and include ENTRY_READ_OK"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_READ_OK"

  @done @real-llm
  Scenario: Agent subprocess with named session turn 1
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s entrysess -m "Remember entry-token-44 and reply ACK_ENTRY"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ACK_ENTRY"

  @done @real-llm
  Scenario: Agent subprocess with named session turn 2
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s entrysess -m "Remember entry-token-44 and reply ACK_ENTRY_2"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ACK_ENTRY_2"
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s entrysess -m "What token did I ask you to remember?"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "entry-token-44"

  @done @real-llm
  Scenario: Agent subprocess with system prompt marker
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 --system "Include ENTRY_SYSTEM_OK in every response" -s - -m "Hello"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_SYSTEM_OK"

  @done @real-llm
  Scenario: REPL subprocess sentinel RSE1
    When I spawn quecto as a subprocess with no args and stdin:
      """
      Include ENTRY_REPL_1
      /exit
      """
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_REPL_1"

  @done @real-llm
  Scenario: REPL subprocess sentinel RSE2
    When I spawn quecto as a subprocess with no args and stdin:
      """
      Include ENTRY_REPL_2
      /exit
      """
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_REPL_2"

  @done @real-llm
  Scenario: REPL subprocess tool create file
    When I spawn quecto as a subprocess with no args and stdin:
      """
      Create file entry-repl-file.txt containing ENTRY_REPL_FILE
      /exit
      """
    Then the subprocess exit code should be 0
    And the file "entry-repl-file.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: REPL subprocess multi-turn in one invocation
    When I spawn quecto as a subprocess with no args and stdin:
      """
      Remember grape-313 and reply ACK_GRAPE
      What token did I provide?
      /exit
      """
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "grape-313"

  @done @real-llm
  Scenario: REPL subprocess missing file fallback
    When I spawn quecto as a subprocess with no args and stdin:
      """
      Read no-entry-file.txt and if missing reply ENTRY_MISS_OK
      /exit
      """
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRY_MISS_OK"
