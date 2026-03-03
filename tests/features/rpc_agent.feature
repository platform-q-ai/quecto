Feature: RPC mode for headless agent operation
  As an external tool or IDE integration
  I want to drive quecto agent via a JSON-lines protocol over stdin/stdout
  So that I can interact with a long-lived agent session programmatically

  # ─── Flag parsing ───────────────────────────────────────────────────────────

  @wip
  Scenario: --mode rpc is accepted as a valid flag
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "hello"
    When I start the RPC agent with no session
    And I queue RPC prompt "hi"
    And I close RPC stdin
    Then the RPC process exits with code 0

  @wip
  Scenario: --mode rpc is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--mode rpc"

  @wip
  Scenario: --mode rpc without config shows error
    Given a temp base directory
    When I start the RPC agent with no session
    And I close RPC stdin
    Then the RPC stderr should contain "config not found"
    And the RPC process exits with code 1

  @wip
  Scenario: --mode rpc with invalid value shows error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent --mode foobar -m "hello"
    Then the exit code should be 1
    And stderr should contain "--mode"

  # ─── prompt command ─────────────────────────────────────────────────────────

  @wip
  Scenario: prompt command triggers agent_start and agent_end events
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "The answer is 42"
    When I start the RPC agent with no session
    And I queue RPC prompt "What is the answer?"
    And I close RPC stdin
    Then the RPC stdout should contain an event of type "agent_start"
    And the RPC stdout should contain an event of type "agent_end"
    And the RPC process exits with code 0

  @wip
  Scenario: prompt command emits turn_start and turn_end events
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "pong"
    When I start the RPC agent with no session
    And I queue RPC prompt "ping"
    And I close RPC stdin
    Then the RPC stdout should contain an event of type "turn_start"
    And the RPC stdout should contain an event of type "turn_end"

  @wip
  Scenario: prompt command emits tool_execution_start and tool_execution_end for tool calls
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a tool call then a text response "done"
    When I start the RPC agent with no session
    And I queue RPC prompt "run a tool"
    And I close RPC stdin
    Then the RPC stdout should contain an event of type "tool_execution_start"
    And the RPC stdout should contain an event of type "tool_execution_end"

  @wip
  Scenario: prompt command with request id echoes id in response
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the RPC agent with no session
    And I queue RPC prompt with id "req-42" and message "hello"
    And I close RPC stdin
    Then the RPC stdout should contain a response with id "req-42"

  @wip
  Scenario: multiple sequential prompts are processed in order
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "first reply"
    And the mock LLM returns a text response "second reply"
    When I start the RPC agent with no session
    And I queue RPC prompt "first"
    And I queue RPC prompt "second"
    And I close RPC stdin
    Then the RPC stdout event "agent_end" should appear 2 times

  # ─── get_state command ──────────────────────────────────────────────────────

  @wip
  Scenario: get_state returns current session state
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the RPC agent with no session
    And I queue RPC prompt "hello"
    And I queue RPC command "get_state" with id "gs-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "get_state" with success true
    And the RPC get_state response should include field "isStreaming"
    And the RPC get_state response should include field "messageCount"
    And the RPC get_state response should include field "model"

  # ─── get_messages command ───────────────────────────────────────────────────

  @wip
  Scenario: get_messages returns conversation history
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "hello back"
    When I start the RPC agent with no session
    And I queue RPC prompt "hello"
    And I queue RPC command "get_messages" with id "gm-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "get_messages" with success true
    And the RPC get_messages response data should include a "messages" array

  # ─── get_session_stats command ──────────────────────────────────────────────

  @wip
  Scenario: get_session_stats returns token usage counts
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "stats test"
    When I start the RPC agent with no session
    And I queue RPC prompt "hi"
    And I queue RPC command "get_session_stats" with id "ss-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "get_session_stats" with success true
    And the RPC get_session_stats response should include field "userMessages"
    And the RPC get_session_stats response should include field "assistantMessages"
    And the RPC get_session_stats response should include field "totalMessages"
    And the RPC get_session_stats response should include field "tokens"

  # ─── set_model command ──────────────────────────────────────────────────────

  @wip
  Scenario: set_model switches the active model
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I start the RPC agent with no session
    And I queue RPC set_model "gpt-5-mini"
    And I queue RPC command "get_state" with id "sm-2"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "set_model" with success true
    And the RPC get_state response model should be "gpt-5-mini"

  # ─── abort command ──────────────────────────────────────────────────────────

  @wip
  Scenario: abort while idle returns success
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the RPC agent with no session
    And I queue RPC command "abort" with id "ab-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "abort" with success true

  # ─── follow_up command ──────────────────────────────────────────────────────

  @wip
  Scenario: follow_up while idle is queued and acknowledged
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the RPC agent with no session
    And I queue RPC follow_up "also do this" with id "fu-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "follow_up" with success true

  # ─── steer command ──────────────────────────────────────────────────────────

  @wip
  Scenario: steer while idle is acknowledged
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the RPC agent with no session
    And I queue RPC steer "change direction" with id "st-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response command "steer" with success true

  # ─── error handling ─────────────────────────────────────────────────────────

  @wip
  Scenario: malformed JSON line produces an error response
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the RPC agent with no session
    And I queue RPC raw line "not valid json{"
    And I close RPC stdin
    Then the RPC stdout should contain a parse error response

  @wip
  Scenario: unknown command type produces an error response
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the RPC agent with no session
    And I queue RPC unknown command with id "u-1"
    And I close RPC stdin
    Then the RPC stdout should contain a response with success false

  # ─── session persistence ─────────────────────────────────────────────────────

  @wip
  Scenario: session is saved on process exit when using named session
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved response"
    When I start the RPC agent with session "rpc-test"
    And I queue RPC prompt "save this"
    And I close RPC stdin
    Then the RPC process exits with code 0
    And a session file for "rpc-test" should exist

  @wip
  Scenario: RPC mode with --no-session does not persist session
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ephemeral"
    When I start the RPC agent with --no-session flag
    And I queue RPC prompt "do not save"
    And I close RPC stdin
    Then the RPC process exits with code 0
    And no session file for "rpc-no-session" should exist

  # ─── EOF / shutdown ──────────────────────────────────────────────────────────

  @wip
  Scenario: EOF on stdin causes clean shutdown
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the RPC agent with no session
    And I close RPC stdin
    Then the RPC process exits with code 0
