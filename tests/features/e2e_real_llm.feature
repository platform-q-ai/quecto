Feature: E2E Real LLM
  End-to-end tests that call a real OpenAI endpoint.
  These scenarios are gated by the @real-llm tag and only run when
  QUECTO_REAL_LLM=1 is set. They require OPENAI_API_KEY in the
  environment and are excluded from normal CI runs.

  Background:
    Given a real LLM workspace is configured

  # --- Basic tool use ---

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

  # --- Additional tool coverage ---

  @done @real-llm
  Scenario: Real LLM edits a file
    Given a file "config.txt" in the e2e workspace with content "mode=debug"
    When I run the real LLM agent with message "Edit the file config.txt and replace 'debug' with 'release'. Do not change anything else."
    Then the exit code should be 0
    And the file "config.txt" in the e2e workspace should contain "mode=release"

  @done @real-llm
  Scenario: Real LLM appends to a file
    Given a file "log.txt" in the e2e workspace with content "line1"
    When I run the real LLM agent with message "Append the text 'line2' on a new line to the file log.txt. Do not overwrite existing content."
    Then the exit code should be 0
    And the file "log.txt" in the e2e workspace should contain "line1"
    And the file "log.txt" in the e2e workspace should contain "line2"

  @done @real-llm
  Scenario: Real LLM lists directory contents
    Given a file "alpha.txt" in the e2e workspace with content "a"
    And a file "beta.txt" in the e2e workspace with content "b"
    When I run the real LLM agent with message "List the files in the workspace directory. Reply with just the filenames, one per line."
    Then the exit code should be 0
    And stdout should contain "alpha.txt"
    And stdout should contain "beta.txt"

  # --- Multi-turn and chaining ---

  @done @real-llm
  Scenario: Real LLM chains read then write
    Given a file "source.txt" in the e2e workspace with content "The answer is 42"
    When I run the real LLM agent with message "Read the file source.txt, then create a new file called result.txt containing only the number you found in source.txt."
    Then the exit code should be 0
    And the file "result.txt" should exist in the e2e workspace
    And the file "result.txt" in the e2e workspace should contain "42"

  @done @real-llm
  Scenario: Real LLM uses multiple tools in one task
    When I run the real LLM agent with message "Create two files: first.txt containing 'hello' and second.txt containing 'world'. Do not include any other text."
    Then the exit code should be 0
    And the file "first.txt" should exist in the e2e workspace
    And the file "second.txt" should exist in the e2e workspace

  # --- Session persistence ---

  @done @real-llm
  Scenario: Real LLM remembers context across session turns
    When I run the real LLM agent with session chat1 and message "Remember this secret code: ZEBRA42. Just confirm you received it."
    Then the exit code should be 0
    When I run the real LLM agent with session chat1 and message "What was the secret code I told you earlier? Reply with just the code."
    Then the exit code should be 0
    And stdout should contain "ZEBRA42"

  # --- System prompt ---

  @done @real-llm
  Scenario: System prompt influences real LLM behavior
    When I run the real LLM agent with system "You are a pirate. Always end your response with 'Arrr!'" and message "Say hello."
    Then the exit code should be 0
    And stdout should contain "Arrr!"

  # --- Skill loading ---

  @done @real-llm
  Scenario: Skill content influences real LLM behavior
    Given a workspace skill "format" with content "Always format your response as a bullet list using dashes."
    When I run the real LLM agent with message "Name three colors."
    Then the exit code should be 0
    And stdout should contain "- "

  # --- Error recovery ---

  @done @real-llm
  Scenario: Real LLM recovers from tool error
    When I run the real LLM agent with message "Read the file nonexistent_file_xyz.txt. If the file does not exist, reply with exactly 'FILE_NOT_FOUND'."
    Then the exit code should be 0
    And stdout should contain "FILE_NOT_FOUND"
