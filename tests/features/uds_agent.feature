Feature: UDS mode for headless agent operation
  As an external tool or IDE integration
  I want to drive quecto agent via a JSON-lines protocol over a Unix domain socket
  So that I can interact with a long-lived agent session programmatically

  # ─── --system flag ──────────────────────────────────────────────────────────

  @done
  Scenario: --system flag injects system prompt into UDS session
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "hello"
    When I start the UDS agent with no session and system prompt "You are a helpful assistant."
    And I send prompt "hi"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an event of type "agent_end"

  @done
  Scenario: --system flag system prompt is not persisted in session history
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved"
    When I start the UDS agent with session "sys-persist-test" and system prompt "You are helpful."
    And I send prompt "remember this"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And a session file for "sys-persist-test" should exist
    And the session for "sys-persist-test" should not contain a system message

  # ─── Flag parsing ───────────────────────────────────────────────────────────

  @done
  Scenario: --mode uds is accepted as a valid flag
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "hello"
    When I start the UDS agent with no session
    And I send prompt "hi"
    And I close the UDS connection
    Then the UDS agent exits with code 0

  @done
  Scenario: --mode uds is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--mode uds"

  @done
  Scenario: --mode uds without config shows error
    Given a temp base directory
    When I start the UDS agent with no session
    And I close the UDS connection
    Then the agent stderr should contain "config not found"
    And the UDS agent exits with code 1

  @done
  Scenario: --mode uds with invalid value shows error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent --mode foobar -m "hello"
    Then the exit code should be 1
    And stderr should contain "--mode"

  # ─── prompt command ─────────────────────────────────────────────────────────

  @done
  Scenario: prompt command triggers agent_start and agent_end events
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "The answer is 42"
    When I start the UDS agent with no session
    And I send prompt "What is the answer?"
    And I close the UDS connection
    Then the agent output should contain an event of type "agent_start"
    And the agent output should contain an event of type "agent_end"
    And the UDS agent exits with code 0

  @done
  Scenario: prompt command emits turn_start and turn_end events
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "pong"
    When I start the UDS agent with no session
    And I send prompt "ping"
    And I close the UDS connection
    Then the agent output should contain an event of type "turn_start"
    And the agent output should contain an event of type "turn_end"

  @done
  Scenario: prompt command emits tool_execution_start and tool_execution_end for tool calls
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call then a text response "done"
    When I start the UDS agent with no session
    And I send prompt "run a tool"
    And I close the UDS connection
    Then the agent output should contain an event of type "tool_execution_start"
    And the agent output should contain an event of type "tool_execution_end"

  @done
  Scenario: prompt command with request id echoes id in response
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the UDS agent with no session
    And I send prompt with id "req-42" and message "hello"
    And I close the UDS connection
    Then the agent output should contain a response with id "req-42"

  @done
  Scenario: multiple sequential prompts are processed in order
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "first reply"
    And the mock LLM returns a text response "second reply"
    When I start the UDS agent with no session
    And I send prompt "first"
    And I send prompt "second"
    And I close the UDS connection
    Then the agent output event "agent_end" should appear 2 times

  # ─── get_state command ──────────────────────────────────────────────────────

  @done
  Scenario: get_state returns current session state
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the UDS agent with no session
    And I send prompt "hello"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the agent output should contain a response command "get_state" with success true
    And the get_state response should include field "isStreaming"
    And the get_state response should include field "messageCount"
    And the get_state response should include field "model"

  # ─── get_messages command ───────────────────────────────────────────────────

  @done
  Scenario: get_messages returns conversation history
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "hello back"
    When I start the UDS agent with no session
    And I send prompt "hello"
    And I send command "get_messages" with id "gm-1"
    And I close the UDS connection
    Then the agent output should contain a response command "get_messages" with success true
    And the get_messages response data should include a "messages" array

  # ─── get_session_stats command ──────────────────────────────────────────────

  @done
  Scenario: get_session_stats returns token usage counts
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "stats test"
    When I start the UDS agent with no session
    And I send prompt "hi"
    And I send command "get_session_stats" with id "ss-1"
    And I close the UDS connection
    Then the agent output should contain a response command "get_session_stats" with success true
    And the get_session_stats response should include field "userMessages"
    And the get_session_stats response should include field "assistantMessages"
    And the get_session_stats response should include field "totalMessages"
    And the get_session_stats response should include field "tokens"

  # ─── set_model command ──────────────────────────────────────────────────────

  @done
  Scenario: set_model switches the active model
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the UDS agent with no session
    And I send set_model "gpt-5-mini"
    And I send command "get_state" with id "sm-2"
    And I close the UDS connection
    Then the agent output should contain a response command "set_model" with success true
    And the get_state response model should be "gpt-5-mini"

  @done
  Scenario: set_model accepts Pi-compatible provider and modelId fields
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the UDS agent with no session
    And I send set_model provider "openai-codex" modelId "gpt-5.3-codex"
    And I send command "get_state" with id "sm-3"
    And I close the UDS connection
    Then the agent output should contain a response command "set_model" with success true
    And the get_state response model should be "openai-codex/gpt-5.3-codex"

  # ─── abort command ──────────────────────────────────────────────────────────

  @done
  Scenario: abort while idle returns success
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send command "abort" with id "ab-1"
    And I close the UDS connection
    Then the agent output should contain a response command "abort" with success true

  # ─── follow_up command ──────────────────────────────────────────────────────

  @done
  Scenario: follow_up while idle is queued and acknowledged
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send follow_up "also do this" with id "fu-1"
    And I close the UDS connection
    Then the agent output should contain a response command "follow_up" with success true

  # ─── steer command ──────────────────────────────────────────────────────────

  @done
  Scenario: steer while idle is acknowledged
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send steer "change direction" with id "st-1"
    And I close the UDS connection
    Then the agent output should contain a response command "steer" with success true

  # ─── error handling ─────────────────────────────────────────────────────────

  @done
  Scenario: malformed JSON line produces an error response
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send raw line "not valid json{"
    And I close the UDS connection
    Then the agent output should contain a parse error response

  @done
  Scenario: unknown command type produces an error response
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send unknown command with id "u-1"
    And I close the UDS connection
    Then the agent output should contain a response with success false

  # ─── session persistence ─────────────────────────────────────────────────────

  @done
  Scenario: session is saved on process exit when using named session
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved response"
    When I start the UDS agent with session "uds-test"
    And I send prompt "save this"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And a session file for "uds-test" should exist

  @done
  Scenario: UDS mode with --no-session does not persist session
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ephemeral"
    When I start the UDS agent with --no-session flag
    And I send prompt "do not save"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And no session file for "uds-no-session" should exist

  # ─── EOF / shutdown ──────────────────────────────────────────────────────────

  @done
  Scenario: closing the connection causes clean shutdown
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I close the UDS connection
    Then the UDS agent exits with code 0

  # ─── parse_error event shape ─────────────────────────────────────────────────

  @wip
  Scenario: parse error response uses type response with command parse_error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send raw line "{{not json"
    And I close the UDS connection
    Then the agent output should contain a parse error response
    And the agent output should contain a response command "parse_error" with success false

  # ─── get_messages_tail command ───────────────────────────────────────────────

  @done
  Scenario: get_messages_tail returns last N messages
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "reply one"
    And the mock LLM returns a text response "reply two"
    When I start the UDS agent with no session
    And I send prompt "first"
    And I send prompt "second"
    And I send get_messages_tail with count 2 and id "gmt-1"
    And I close the UDS connection
    Then the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail response should include a "messages" array
    And the get_messages_tail messages count should be at most 2

  @done
  Scenario: get_messages_tail with count larger than history returns all messages
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "only reply"
    When I start the UDS agent with no session
    And I send prompt "only prompt"
    And I send get_messages_tail with count 100 and id "gmt-2"
    And I close the UDS connection
    Then the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail response should include a "messages" array

  @done
  Scenario: get_messages_tail with count 0 returns empty messages array
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "any reply"
    When I start the UDS agent with no session
    And I send prompt "any prompt"
    And I send get_messages_tail with count 0 and id "gmt-3"
    And I close the UDS connection
    Then the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail messages count should be exactly 0

  @done
  Scenario: get_messages_tail on empty history returns empty array
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send get_messages_tail with count 5 and id "gmt-4"
    And I close the UDS connection
    Then the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail messages count should be exactly 0

  # ─── compute_session_stats correctness ───────────────────────────────────────

  @wip
  Scenario: get_session_stats message counts are accurate after a prompt
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "counted"
    When I start the UDS agent with no session
    And I send prompt "count me"
    And I send command "get_session_stats" with id "stats-2"
    And I close the UDS connection
    Then the agent output should contain a response command "get_session_stats" with success true
    And the get_session_stats userMessages should equal 1
    And the get_session_stats assistantMessages should equal 1
