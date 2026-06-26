Feature: E2E Real LLM Agent Matrix
  Broader real endpoint coverage for the one-shot agent entry point.

  Background:
    Given a real LLM workspace is configured

  @done @manual-real-llm @mock-llm
  Scenario: Agent sentinel token response A
    When I run the real LLM agent with [message] "Reply with the token MATRIX_A1 in your response"
    Then the exit code should be 0
    And stdout should contain "MATRIX_A1"

  @done @manual-real-llm @mock-llm
  Scenario: Agent sentinel token response B
    When I run the real LLM agent with [message] "Reply with the token MATRIX_B2 in your response"
    Then the exit code should be 0
    And stdout should contain "MATRIX_B2"

  @done @manual-real-llm @mock-llm
  Scenario: Agent system prompt style marker
    When I run the real LLM agent with system "Always include STYLE_OK in your response" and [message] "Say hello"
    Then the exit code should be 0
    And stdout should contain "STYLE_OK"

  @done @manual-real-llm @mock-llm
  Scenario: Agent creates a plain text file
    When I run the real LLM agent with [message] "Create file matrix1.txt with content MATRIX_FILE_1"
    Then the exit code should be 0
    And the file "matrix1.txt" should exist in the e2e workspace

  @done @manual-real-llm @mock-llm
  Scenario: Agent creates a nested file path
    When I run the real LLM agent with [message] "Create file notes/matrix2.txt with content MATRIX_FILE_2"
    Then the exit code should be 0
    And the file "notes/matrix2.txt" should exist in the e2e workspace

  @done @manual-real-llm @mock-llm
  Scenario: Agent reads file and returns specific token
    Given a file "facts.txt" in the e2e workspace with content "token=READ_X9"
    When I run the real LLM agent with [message] "Read facts.txt and include READ_X9 in your response"
    Then the exit code should be 0
    And stdout should contain "READ_X9"

  @done @manual-real-llm @mock-llm
  Scenario: Agent edits a file value
    Given a file "mode.txt" in the e2e workspace with content "mode=alpha"
    When I run the real LLM agent with [message] "Edit mode.txt so alpha becomes beta"
    Then the exit code should be 0
    And the file "mode.txt" in the e2e workspace should contain "mode=beta"

  @done @manual-real-llm @mock-llm
  Scenario: Agent quick token response C3
    When I run the real LLM agent with [message] "Reply with the token MATRIX_C3"
    Then the exit code should be 0
    And stdout should contain "MATRIX_C3"

  @done @manual-real-llm @mock-llm
  Scenario: Agent lists directory containing prepared files
    Given a file "list-a.txt" in the e2e workspace with content "A"
    And a file "list-b.txt" in the e2e workspace with content "B"
    When I run the real LLM agent with [message] "List the workspace files and include list-a.txt and list-b.txt"
    Then the exit code should be 0
    And stdout should contain "list-a.txt"
    And stdout should contain "list-b.txt"

  @done @manual-real-llm @mock-llm
  Scenario: Agent runs exec and echoes marker
    When I run the real LLM agent with [message] "Run command echo MATRIX_EXEC_1 and include MATRIX_EXEC_1 in your response"
    Then the exit code should be 0
    And stdout should contain "MATRIX_EXEC_1"

  @done @manual-real-llm @mock-llm
  Scenario: Agent chains read and write number extraction
    Given a file "num.txt" in the e2e workspace with content "value=2468"
    When I run the real LLM agent with [message] "Read num.txt and create out-num.txt containing 2468"
    Then the exit code should be 0
    And the file "out-num.txt" should exist in the e2e workspace
    And the file "out-num.txt" in the e2e workspace should contain "2468"

  @done @manual-real-llm @mock-llm
  Scenario: Agent writes JSON-like text file
    When I run the real LLM agent with [message] "Create file data.json containing exactly {\"ok\":true,\"id\":7}"
    Then the exit code should be 0
    And the file "data.json" should exist in the e2e workspace

  @done @manual-real-llm @mock-llm
  Scenario: Agent handles missing file with fallback token
    When I run the real LLM agent with [message] "Try reading no-such-file-77.txt. If missing, reply with FALLBACK_77"
    Then the exit code should be 0
    And stdout should contain "FALLBACK_77"

  @done @manual-real-llm @mock-llm
  Scenario: Agent named session remembers short phrase
    When I run the real LLM agent with session memone and message "Remember phrase kiwi river. Reply ACK_KIWI"
    Then the exit code should be 0
    And stdout should contain "ACK_KIWI"
    When I run the real LLM agent with session memone and message "What phrase did I ask you to remember?"
    Then the exit code should be 0
    And stdout should contain "kiwi river"

  @done @manual-real-llm @mock-llm
  Scenario: Agent named session remembers digits
    When I run the real LLM agent with session memtwo and message "Remember number 55321. Reply ACK_55321"
    Then the exit code should be 0
    And stdout should contain "ACK_55321"
    When I run the real LLM agent with session memtwo and message "Return the number I gave you earlier"
    Then the exit code should be 0
    And stdout should contain "55321"

  @done @manual-real-llm @mock-llm
  Scenario: Agent session isolation between two names
    When I run the real LLM agent with session sessa and message "Remember ISOLATE_ALPHA"
    Then the exit code should be 0
    When I run the real LLM agent with session sessb and message "What did I ask in other session? If unknown reply ISOLATE_NONE"
    Then the exit code should be 0
    And stdout should contain "ISOLATE_NONE"

  @done @manual-real-llm @mock-llm
  Scenario: Agent ephemeral run leaves no session files
    When I run the real LLM agent with [message] "Reply with EPHEMERAL_OK"
    Then the exit code should be 0
    And stdout should contain "EPHEMERAL_OK"
    And no [session] files should exist

  @done @manual-real-llm @mock-llm
  Scenario: Agent spawn tool basic task in matrix
    When I run the real LLM agent with session spawna and message "Try spawn tool with task 'Matrix delegation task'. If unavailable, say so briefly."
    Then the exit code should be 0
    And stdout should contain "spawn"

  @done @manual-real-llm @mock-llm
  Scenario: Agent spawn tool with deliver target in matrix
    When I run the real LLM agent with session spawnb and message "Try spawn tool with task 'Matrix notify' and deliver_to 'telegram:999'. If unavailable reply MATRIX_SPAWN_B_UNAVAILABLE, otherwise MATRIX_SPAWN_B_OK"
    Then the exit code should be 0
    And stdout should contain "MATRIX_SPAWN_B"

  @done @manual-real-llm @mock-llm
  Scenario: Agent can create two files in one request
    When I run the real LLM agent with [message] "Create files matrix-a.txt with AVAL and matrix-b.txt with BVAL"
    Then the exit code should be 0
    And the file "matrix-a.txt" should exist in the e2e workspace
    And the file "matrix-b.txt" should exist in the e2e workspace

  @done @manual-real-llm @mock-llm
  Scenario: Agent reads one file then appends to another
    Given a file "seed.txt" in the e2e workspace with content "SEED42"
    Given a file "collector.txt" in the e2e workspace with content "begin"
    When I run the real LLM agent with [message] "Read seed.txt then append SEED42 to collector.txt"
    Then the exit code should be 0
    And the file "collector.txt" in the e2e workspace should contain "begin"
    And the file "collector.txt" in the e2e workspace should contain "SEED42"

  @done @manual-real-llm @mock-llm
  Scenario: Agent file edit keeps surrounding text
    Given a file "phrase.txt" in the e2e workspace with content "alpha middle omega"
    When I run the real LLM agent with [message] "In phrase.txt replace middle with center"
    Then the exit code should be 0
    And the file "phrase.txt" in the e2e workspace should contain "alpha center omega"
