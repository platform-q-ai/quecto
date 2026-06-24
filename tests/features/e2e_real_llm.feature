Feature: E2E Real LLM
  End-to-end tests that call a real OpenAI endpoint.
  These scenarios are gated by the @real-llm tag and only run when
  QUECTO_REAL_LLM=1 is set. They require OPENAI_API_KEY in the
  environment and are excluded from normal CI runs.

  Background:
    Given a real LLM workspace is configured

  # --- Basic tool use ---

  @done @real-llm @real-llm-smoke
  Scenario: Simple text response from real LLM
    When I run the real LLM agent with [message] "Reply with exactly the word PONG and nothing else"
    Then the exit code should be 0
    And stdout should not be empty

  @done @real-llm @real-llm-smoke
  Scenario: Real LLM writes a file via tool use
    When I run the real LLM agent with [message] "Create a file called hello.txt containing the text 'Hello from LLM' and nothing else. Do not include any other text in the file."
    Then the exit code should be 0
    And the file "hello.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: Real LLM reads and summarises a file
    Given a file "data.txt" in the e2e workspace with content "The quick brown fox jumps over the lazy dog"
    When I run the real LLM agent with [message] "Read the file data.txt and tell me what animal jumps. Reply with just the animal name."
    Then the exit code should be 0
    And stdout should not be empty

  @done @real-llm
  Scenario: Real LLM executes a shell command
    When I run the real LLM agent with [message] "Run the command 'echo HELLO_QUECTO' and tell me what it printed. Reply with just the output."
    Then the exit code should be 0
    And stdout should contain "HELLO_QUECTO"

  @done @real-llm @real-llm-web-search
  Scenario: Real LLM can perform a web search via tool use
    When I run the real LLM agent with [message] "Use the web_search tool to search for the official Rust website. If any search result includes rust-lang.org, reply with exactly WEB_SEARCH_OK. Otherwise reply with exactly WEB_SEARCH_FAIL."
    Then the exit code should be 0
    And stdout should contain "WEB_SEARCH_"

  # --- Additional tool coverage ---

  @done @real-llm
  Scenario: Real LLM edits a file
    Given a file "config.txt" in the e2e workspace with content "mode=debug"
    When I run the real LLM agent with [message] "Edit the file config.txt and replace 'debug' with 'release'. Do not change anything else."
    Then the exit code should be 0
    And the file "config.txt" in the e2e workspace should contain "mode=release"

  @done @real-llm
  Scenario: Real LLM appends to a file
    Given a file "log.txt" in the e2e workspace with content "line1"
    When I run the real LLM agent with [message] "Append the text 'line2' on a new line to the file log.txt. Do not overwrite existing content."
    Then the exit code should be 0
    And the file "log.txt" in the e2e workspace should contain "line1"
    And the file "log.txt" in the e2e workspace should contain "line2"

  @done @real-llm
  Scenario: Real LLM lists directory contents
    Given a file "alpha.txt" in the e2e workspace with content "a"
    And a file "beta.txt" in the e2e workspace with content "b"
    When I run the real LLM agent with [message] "List the files in the workspace directory. Reply with just the filenames, one per line."
    Then the exit code should be 0
    And stdout should contain "alpha.txt"
    And stdout should contain "beta.txt"

  # --- Multi-turn and chaining ---

  @done @real-llm
  Scenario: Real LLM chains read then write
    Given a file "source.txt" in the e2e workspace with content "The answer is 42"
    When I run the real LLM agent with [message] "Read the file source.txt, then create a new file called result.txt containing only the number you found in source.txt."
    Then the exit code should be 0
    And the file "result.txt" should exist in the e2e workspace
    And the file "result.txt" in the e2e workspace should contain "42"

  @done @real-llm
  Scenario: Real LLM uses multiple tools in one task
    When I run the real LLM agent with [message] "Create two files: first.txt containing 'hello' and second.txt containing 'world'. Do not include any other text."
    Then the exit code should be 0
    And the file "first.txt" should exist in the e2e workspace
    And the file "second.txt" should exist in the e2e workspace

  # --- Session persistence ---

  @done @real-llm @real-llm-smoke
  Scenario: Real LLM remembers context across session turns
    When I run the real LLM agent with session chat1 and message "My favorite color is turquoise. Just confirm you noted it."
    Then the exit code should be 0
    When I run the real LLM agent with session chat1 and message "What is my favorite color? Reply with exactly turquoise in lowercase."
    Then the exit code should be 0
    And stdout should contain "turquoise"

  @done @real-llm
  Scenario: Real LLM multi-turn session can reuse a numeric token
    When I run the real LLM agent with session memo42 and message "Remember this token exactly: 7319. Reply with ACK_7319"
    Then the exit code should be 0
    And stdout should contain "ACK_7319"
    When I run the real LLM agent with session memo42 and message "What token did I ask you to remember? Reply with only the digits."
    Then the exit code should be 0
    And stdout should contain "7319"

  # --- System prompt ---

  @done @real-llm
  Scenario: System prompt influences real LLM behavior
    When I run the real LLM agent with system "You are a pirate. Always end your response with 'Arrr!'" and [message] "Say hello."
    Then the exit code should be 0
    And stdout should contain "Arrr!"

  # --- Subagent / spawn tool ---

  @done @real-llm
  Scenario: Real LLM can invoke spawn tool for delegation
    When I run the real LLM agent with session spawn-session and message "Try to call the spawn tool with task 'Draft release notes'. If unavailable, explain briefly."
    Then the exit code should be 0
    And stdout should contain "spawn"

  @done @real-llm
  Scenario: Real LLM spawn tool call can include metadata fields
    When I run the real LLM agent with session spawn-meta and message "Try to call spawn with task 'Analyze logs', agent_id 'analyst', and deliver_to 'telegram:12345'. If unavailable reply SPAWN_META_UNAVAILABLE, otherwise reply SPAWN_META_OK"
    Then the exit code should be 0
    And stdout should contain "SPAWN_META"

  # --- Error recovery ---

  @done @real-llm @real-llm-smoke
  Scenario: Real LLM recovers from tool error
    When I run the real LLM agent with [message] "Read the file nonexistent_file_xyz.txt. If the file does not exist, reply with exactly 'FILE_NOT_FOUND'."
    Then the exit code should be 0
    And stdout should contain "FILE_NOT_FOUND"
