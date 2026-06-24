Feature: E2E Real LLM REPL Matrix
  Broader real endpoint coverage for interactive REPL behavior.

  Background:
    Given a real LLM workspace is configured

  @done @real-llm
  Scenario: REPL sentinel response R1
    When I start quecto in REPL mode
    And I type "Include token REPL_R1"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_R1"

  @done @real-llm
  Scenario: REPL sentinel response R2
    When I start quecto in REPL mode
    And I type "Include token REPL_R2"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_R2"

  @done @real-llm
  Scenario: REPL system prompt marker
    When I start quecto in REPL mode with flags "--system 'Include REPL_SYSTEM_OK in every response'"
    And I type "Say hello"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_SYSTEM_OK"

  @done @real-llm
  Scenario: REPL creates file tool task A
    When I start quecto in REPL mode
    And I type "Create file repl-a.txt containing REPL_FILE_A"
    And I type "/exit"
    Then the exit code should be 0
    And the file "repl-a.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: REPL creates file tool task B
    When I start quecto in REPL mode
    And I type "Create file repl-b.txt containing REPL_FILE_B"
    And I type "/exit"
    Then the exit code should be 0
    And the file "repl-b.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: REPL reads prepared file
    Given a file "repl-read.txt" in the e2e workspace with content "REPL_READ_OK"
    When I start quecto in REPL mode
    And I type "Read repl-read.txt and include REPL_READ_OK"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_READ_OK"

  @done @real-llm
  Scenario: REPL multi-turn remembers phrase in one run
    When I start quecto in REPL mode
    And I type "Remember phrase orange cloud and reply ACK_ORANGE"
    And I type "What phrase did I tell you?"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "orange cloud"

  @done @real-llm
  Scenario: REPL multi-turn remembers number in one run
    When I start quecto in REPL mode
    And I type "Remember number 6622 and reply ACK_6622"
    And I type "What number did I provide?"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "6622"

  @done @real-llm
  Scenario: REPL named session memory S1
    When I start quecto in REPL mode with flags "-s repls1"
    And I type "Remember kiwi-11"
    And I type "/exit"
    Then the exit code should be 0
    When I start quecto in REPL mode with flags "-s repls1"
    And I type "What token did I give?"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "kiwi-11"

  @done @real-llm
  Scenario: REPL named session memory S2
    When I start quecto in REPL mode with flags "-s repls2"
    And I type "Remember melon-22"
    And I type "/exit"
    Then the exit code should be 0
    When I start quecto in REPL mode with flags "-s repls2"
    And I type "What token did I give?"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "melon-22"

  @done @real-llm
  Scenario: REPL with model override still responds
    When I start quecto in REPL mode with flags "--model gpt-5.2"
    And I type "Include REPL_MODEL_OK"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_MODEL_OK"

  @done @real-llm
  Scenario: REPL executes shell command through tools
    When I start quecto in REPL mode
    And I type "Run command echo REPL_EXEC_OK and include REPL_EXEC_OK"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_EXEC_OK"

  @done @real-llm
  Scenario: REPL can append to existing file
    Given a file "repl-append.txt" in the e2e workspace with content "first"
    When I start quecto in REPL mode
    And I type "Append second on new line to repl-append.txt"
    And I type "/exit"
    Then the exit code should be 0
    And the file "repl-append.txt" in the e2e workspace should contain "first"
    And the file "repl-append.txt" in the e2e workspace should contain "second"

  @done @real-llm
  Scenario: REPL can edit existing file
    Given a file "repl-edit.txt" in the e2e workspace with content "state=old"
    When I start quecto in REPL mode
    And I type "Use edit to change repl-edit.txt from state=old to state=new"
    And I type "/exit"
    Then the exit code should be 0
    And the file "repl-edit.txt" in the e2e workspace should contain "state=new"

  @done @real-llm
  Scenario: REPL handles missing file with fallback token
    When I start quecto in REPL mode
    And I type "Try reading no-repl-file.txt and if missing reply REPL_MISS_OK"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "REPL_MISS_OK"

  @done @real-llm
  Scenario: REPL can list workspace files
    Given a file "repl-list-a.txt" in the e2e workspace with content "a"
    And a file "repl-list-b.txt" in the e2e workspace with content "b"
    When I start quecto in REPL mode
    And I type "List files and include repl-list-a.txt and repl-list-b.txt"
    And I type "/exit"
    Then the exit code should be 0
    And stdout should contain "repl-list-a.txt"
    And stdout should contain "repl-list-b.txt"
